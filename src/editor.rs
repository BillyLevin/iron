use std::iter;

use crossterm::event::Event;

use crate::{
    buffer::Buffer,
    commands::CommandList,
    document::Document,
    ui::{
        Layer,
        LayerKind,
        Position,
    },
};

pub(crate) struct Editor {
    document: Document,
    layers: Vec<Box<dyn Layer>>,
}

impl Editor {
    pub(crate) fn new(document: Document) -> Self {
        Self {
            document,
            layers: vec![],
        }
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
        let mut result = EventOutcome::Unhandled;

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
                    match self.document.save() {
                        Ok(()) => {}
                        Err(err) => self.document.set_error(format!("{err:#}")),
                    }
                }
                EditorAction::WriteQuit => {
                    match self.document.save() {
                        Ok(()) => {}
                        Err(err) => self.document.set_error(format!("{err:#}")),
                    }

                    return EventOutcome::CloseApp;
                }
            }
        }

        EventOutcome::Handled
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum EditorAction {
    AddLayer(LayerKind),
    RemoveLayer(LayerKind),
    Quit,
    Write,
    WriteQuit,
}
