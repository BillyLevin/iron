use std::{
    iter,
    path::PathBuf,
};

use crate::{
    buffer::Buffer,
    command_list::CommandList,
    diagnostic_list::DiagnosticList,
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
        AppEvent,
        Dimensions,
        Layer,
        LayerKind,
        Position,
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
    pub(crate) fn new(
        file_path: PathBuf,
        dimensions: Dimensions,
        lsp: Option<LspClient>,
        jj_poller: JJPoller,
        file_index: FileIndex,
    ) -> anyhow::Result<Self> {
        let document = Document::new(file_path, dimensions)?;

        if let Some(ref client) = lsp {
            client.document_opened(document.lsp_snapshot());
        }

        Ok(Self {
            document,
            layers: vec![],
            file_index,
            dimensions,
            jj_poller,
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

    pub(crate) fn handle_event(&mut self, event: &AppEvent) -> EventOutcome {
        puffin::profile_function!();

        match *event {
            AppEvent::Resize(dimensions) => {
                self.resize(dimensions);
                EventOutcome::Handled
            }
            AppEvent::Layer(ref layer_event) => {
                let mut event_context = EventContext::new();

                let result = self
                    .layers_mut()
                    .rev()
                    .find_map(|layer| {
                        match layer.handle_event(layer_event, &mut event_context) {
                            EventOutcome::Handled => Some(EventOutcome::Handled),
                            EventOutcome::CloseApp => Some(EventOutcome::CloseApp),
                            EventOutcome::Unhandled => None,
                        }
                    })
                    .unwrap_or(EventOutcome::Unhandled);

                match result {
                    EventOutcome::Handled => self.apply_actions(event_context.actions()),
                    EventOutcome::Unhandled | EventOutcome::CloseApp => result,
                }
            }
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
                        // TODO: when we support multiple open documents, match
                        // up `params.uri` with the
                        // correct doc
                        if self.document.url() == &params.uri
                            && params
                                .version
                                .is_none_or(|version| version == self.document.version().value())
                        {
                            self.document.publish_diagnostics(params, position_encoding);

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
                            self.layers
                                .push(Box::new(CommandList::new(self.dimensions)));
                        }
                        LayerKind::FilePicker => {
                            self.layers
                                .push(Box::new(FilePicker::new(&self.file_index, self.dimensions)));
                        }
                        LayerKind::DiagnosticList => {
                            let diagnostics = self.document.diagnostics_for_cursor_line();

                            if !diagnostics.is_empty() {
                                self.layers.push(Box::new(DiagnosticList::new(
                                    &diagnostics,
                                    self.dimensions,
                                )));
                            }
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

    fn resize(&mut self, dimensions: Dimensions) {
        self.dimensions = dimensions;

        for layer in self.layers_mut() {
            layer.resize(dimensions);
        }
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

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use crossterm::event::KeyCode;

    use crate::{
        editor::EventOutcome,
        test_harness::TestHarness,
        ui::{
            Columns,
            Dimensions,
            Rows,
        },
    };

    #[test]
    fn displays_basic_screen() {
        let mut harness = TestHarness::new(
            "hello\nthere\nworld",
            Dimensions::new(Columns::new(24), Rows::new(6)),
        );

        insta::assert_snapshot!(
            harness.screen().to_string(),
            @"
        ┌─────▼──────────────────┐
        ▶   1 hello              │
        │   1 there              │
        │   2 world              │
        │                        │
        │ NORMAL  test.txt  text │
        │                        │
        └────────────────────────┘
    "
        );
    }

    #[test]
    fn moves_cursor_and_inserts_text() {
        let mut harness = TestHarness::new(
            "hello\nthere\nworld",
            Dimensions::new(Columns::new(24), Rows::new(6)),
        );

        insta::assert_snapshot!(
            harness.screen().to_string(),
            @"
        ┌─────▼──────────────────┐
        ▶   1 hello              │
        │   1 there              │
        │   2 world              │
        │                        │
        │ NORMAL  test.txt  text │
        │                        │
        └────────────────────────┘
    "
        );

        // navigate
        assert_matches!(harness.send_key(KeyCode::Char('j')), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Char('l')), EventOutcome::Handled);

        // enter text
        assert_matches!(harness.send_key(KeyCode::Char('i')), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Char('h')), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Char('e')), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Char('y')), EventOutcome::Handled);

        insta::assert_snapshot!(
            harness.screen().to_string(),
            @"
        ┌─────────▼──────────────┐
        │   1 hello              │
        ▶   2 theyhere           │
        │   1 world              │
        │                        │
        │ INSERT  test.txt  text │
        │                        │
        └────────────────────────┘
    "
        );
    }
}
