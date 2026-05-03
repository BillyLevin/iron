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
    layers: Vec<Box<dyn Layer>>,
}

impl Editor {
    pub(crate) fn new(document: Document) -> Self {
        Self {
            layers: vec![Box::new(document)],
        }
    }

    pub(crate) fn visual_cursor_position(&self) -> Position {
        self.layers
            .iter()
            .rev()
            .find_map(|layer| layer.visual_cursor_position())
            .expect(
                "at least one visual layer must have a cursor position (i.e. `Document` cursor \
                 position is always `Some`)",
            )
    }

    pub(crate) fn handle_event(&mut self, event: &Event) -> EventOutcome {
        let mut event_context = EventContext::new();

        for layer in self.layers.iter_mut().rev() {
            match layer.handle_event(event, &mut event_context) {
                outcome @ (EventOutcome::Handled | EventOutcome::CloseApp) => {
                    self.apply_actions(event_context.actions());
                    return outcome;
                }
                EventOutcome::Unhandled => {}
            }
        }

        EventOutcome::Unhandled
    }

    pub(crate) fn render(&mut self, buffer: &mut Buffer) {
        for layer in &mut self.layers {
            layer.render(buffer);
        }
    }

    pub(crate) fn handle_layer_events(&mut self) -> EventOutcome {
        let mut result = EventOutcome::Unhandled;

        for layer in &mut self.layers {
            match layer.handle_internal_events() {
                EventOutcome::Handled => result = EventOutcome::Handled,
                EventOutcome::CloseApp => return EventOutcome::CloseApp,
                EventOutcome::Unhandled => {}
            }
        }

        result
    }

    fn apply_actions(&mut self, actions: Vec<EditorAction>) {
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
            }
        }
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
}
