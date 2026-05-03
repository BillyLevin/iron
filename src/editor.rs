use crossterm::event::Event;

use crate::{
    buffer::Buffer,
    commands::CommandList,
    document::Document,
    ui::{
        EditorAction,
        EventContext,
        Layer,
        LayerKind,
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

    pub(crate) fn visual_cursor_position(&self) -> crate::ui::Position {
        self.layers
            .last()
            .expect("should be non-empty (TODO: guarantee this at the type level)")
            .visual_cursor_position()
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

    pub(crate) fn render(&self, buffer: &mut Buffer) {
        for layer in &self.layers {
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
