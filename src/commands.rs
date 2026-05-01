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
        Rows,
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
        // TODO: figure out what to do when there's not enough room. i don't care about
        // it not working since the screen should never be that tiny, but i'd
        // rather the program didn't crash in that case.

        let app_rectangle = Rectangle::from_dimensions(buffer.dimensions());

        let rectangle = app_rectangle.at_center(Dimensions::new(
            app_rectangle.width() / 2,
            app_rectangle.height() / 2,
        ));

        buffer.fill_background(&rectangle, Color::Magenta);

        let (input_rectangle, _rest) = rectangle.split_at_row(Rows::new(3));

        let _input_text_rectangle = buffer.draw_border(&input_rectangle, Color::Black);
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
