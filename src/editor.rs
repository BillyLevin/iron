use std::{
    iter,
    path::PathBuf,
};

use crossterm::event::Event;

use crate::{
    buffer::Buffer,
    commands::CommandList,
    document::Document,
    file_picker::{
        FileIndex,
        FilePicker,
    },
    jujutsu::JJPoller,
    lsp::{
        LspClient,
        LspEvent,
        LspTextEdit,
    },
    ui::{
        Columns,
        Dimensions,
        Layer,
        LayerKind,
        Position,
        Rows,
    },
};

pub(crate) struct Editor {
    document: Document,
    layers: Vec<Box<dyn Layer>>,
    file_index: FileIndex,
    dimensions: Dimensions,
    jj_poller: JJPoller,
    lsp: Option<LspClient>,
}

impl Editor {
    pub(crate) fn new(file_path: PathBuf, dimensions: Dimensions) -> anyhow::Result<Self> {
        let document = Document::new(file_path.clone(), dimensions)?;
        let lsp = LspClient::new()
            .inspect_err(|err| log::error!("failed to start LSP client: {err}"))
            .ok();

        if let Some(ref client) = lsp {
            client.document_opened(document.lsp_snapshot());
        }

        Ok(Self {
            document,
            layers: vec![],
            file_index: FileIndex::new(),
            dimensions,
            jj_poller: JJPoller::new(file_path),
            lsp,
        })
    }

    pub(crate) fn visual_cursor_position(&self) -> Position {
        self.layers()
            .rev()
            .find_map(Layer::visual_cursor_position)
            .expect(
                "at least one visual layer must have a cursor position (i.e. `Document` cursor \
                 position is always `Some`)",
            )
    }

    pub(crate) fn handle_event(&mut self, event: &Event) -> EventOutcome {
        puffin::profile_function!();

        let mut result = EventOutcome::Unhandled;

        let mut event_context = EventContext::new();

        if let Event::Resize(columns, rows) = *event {
            let dimensions = Dimensions::new(Columns::from(columns), Rows::from(rows));
            self.resize(dimensions);
            result = EventOutcome::Handled;
        }

        for layer in self.layers_mut().rev() {
            match layer.handle_event(event, &mut event_context) {
                EventOutcome::Handled => {
                    result = EventOutcome::Handled;
                    break;
                }
                EventOutcome::CloseApp => return EventOutcome::CloseApp,
                EventOutcome::Unhandled => {}
            }
        }

        match result {
            EventOutcome::Handled => self.apply_actions(event_context.actions()),
            EventOutcome::Unhandled | EventOutcome::CloseApp => result,
        }
    }

    pub(crate) fn render(&mut self, buffer: &mut Buffer) {
        puffin::profile_function!();

        for layer in self.layers_mut() {
            layer.render(buffer);
        }
    }

    pub(crate) fn handle_layer_events(&mut self) -> EventOutcome {
        let mut result = if let Some(status) = self.jj_poller.refresh() {
            self.document.set_jj_info(status);
            EventOutcome::Handled
        } else {
            EventOutcome::Unhandled
        };

        if let Some(ref lsp) = self.lsp {
            for event in lsp.events() {
                match event {
                    LspEvent::PublishDiagnostics {
                        params,
                        position_encoding,
                    } => {
                        // TODO: when we support multiple open documents, match up
                        // `params.uri` with the correct doc
                        if self.document.url() == &params.uri
                            && params
                                .version
                                .is_none_or(|version| version == self.document.version().value())
                        {
                            self.document
                                .publish_diagnostics(&params, position_encoding);

                            result = EventOutcome::Handled;
                        }
                    }
                }
            }
        }

        for layer in self.layers_mut() {
            match layer.handle_internal_events() {
                EventOutcome::Handled => result = EventOutcome::Handled,
                EventOutcome::CloseApp => return EventOutcome::CloseApp,
                EventOutcome::Unhandled => {}
            }
        }

        result
    }

    fn layers(&self) -> impl DoubleEndedIterator<Item = &dyn Layer> {
        iter::once(&self.document as &dyn Layer).chain(self.layers.iter().map(AsRef::as_ref))
    }

    fn layers_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut (dyn Layer + '_)> {
        iter::once(&mut self.document as &mut dyn Layer)
            .chain(self.layers.iter_mut().map(|l| l.as_mut() as &mut dyn Layer))
    }

    fn apply_actions(&mut self, actions: Vec<EditorAction>) -> EventOutcome {
        for action in actions {
            match action {
                EditorAction::AddLayer(layer) => {
                    match layer {
                        LayerKind::CommandList => {
                            self.layers.push(Box::new(CommandList::new()));
                        }
                        LayerKind::FilePicker => {
                            self.layers
                                .push(Box::new(FilePicker::new(self.file_index.picker())));
                        }
                    }
                }
                EditorAction::RemoveLayer(layer_kind) => {
                    // TODO: will we ever have multiple layers of the same type?
                    // if so, the layers will need ids so that we can ensure we
                    // remove the correct one
                    let remove_index = self
                        .layers
                        .iter()
                        .position(|layer| layer.kind() == Some(layer_kind));

                    if let Some(idx) = remove_index {
                        self.layers.remove(idx);
                    }
                }
                EditorAction::Quit => return EventOutcome::CloseApp,
                EditorAction::Write => {
                    match self.save_document() {
                        Ok(()) => {}
                        Err(err) => self.document.set_error(format!("{err:#}")),
                    }
                }
                EditorAction::WriteQuit => {
                    match self.save_document() {
                        Ok(()) => return EventOutcome::CloseApp,
                        Err(err) => self.document.set_error(format!("{err:#}")),
                    }
                }
                EditorAction::OpenFile(path) => {
                    match self.replace_document(path) {
                        Ok(()) => {}
                        Err(err) => self.document.set_error(format!("{err:#}")),
                    }
                }
                EditorAction::DocumentChanged(edit) => {
                    if let Some(ref lsp) = self.lsp {
                        lsp.document_changed(self.document.lsp_snapshot(), edit);
                    }
                }
            }
        }

        EventOutcome::Handled
    }

    pub(crate) const fn resize(&mut self, dimensions: Dimensions) {
        self.dimensions = dimensions;
    }

    fn save_document(&self) -> anyhow::Result<()> {
        self.document.save()?;
        if let Some(ref lsp) = self.lsp {
            lsp.document_saved(self.document.lsp_snapshot());
        }

        Ok(())
    }

    fn replace_document(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let new_doc = Document::new(path.clone(), self.dimensions)?;

        if let Some(ref lsp) = self.lsp {
            let old_doc_id = self.document.lsp_id();
            lsp.document_closed(old_doc_id.clone());
            lsp.document_opened(new_doc.lsp_snapshot());
        }

        self.document = new_doc;
        self.jj_poller.update_path(path);

        Ok(())
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) enum EventOutcome {
    Handled,
    Unhandled,
    CloseApp,
}

pub(crate) struct EventContext {
    actions: Vec<EditorAction>,
}

impl EventContext {
    pub(crate) const fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    /// Queue a new action to be applied to the
    /// [`Editor`](crate::editor::Editor).
    pub(crate) fn push_action(&mut self, action: EditorAction) {
        self.actions.push(action);
    }

    pub(crate) fn actions(self) -> Vec<EditorAction> {
        self.actions
    }
}

#[derive(Debug)]
pub(crate) enum EditorAction {
    AddLayer(LayerKind),
    RemoveLayer(LayerKind),
    Quit,
    Write,
    WriteQuit,
    OpenFile(PathBuf),
    DocumentChanged(LspTextEdit),
}
