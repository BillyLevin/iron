use crossterm::{
    event::Event,
    style::Color,
};

use crate::{
    buffer::Buffer,
    editor::EventOutcome,
    ui::{
        Dimensions,
        EventContext,
        Layer,
        Position,
        Rectangle,
    },
};

#[derive(Debug)]
pub(crate) struct CommandList;

impl CommandList {
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl Layer for CommandList {
    fn render(&self, buffer: &mut Buffer) {
        let app_rectangle = Rectangle::from_dimensions(buffer.dimensions());

        let rectangle = app_rectangle.at_center(Dimensions::new(
            app_rectangle.width() / 2,
            app_rectangle.height() / 2,
        ));

        buffer.fill_background(&rectangle, Color::Magenta);
    }

    fn handle_event(&mut self, _event: &Event, _event_context: &mut EventContext) -> EventOutcome {
        todo!()
    }

    fn visual_cursor_position(&self) -> Position {
        // TODO: implement properly
        Position::default()
    }

    fn handle_internal_events(&mut self) -> EventOutcome {
        EventOutcome::Unhandled
    }
}
