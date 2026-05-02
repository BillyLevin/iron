use crossterm::{
    event::{
        Event,
        KeyCode,
        KeyModifiers,
    },
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
        Span,
    },
};

#[derive(Debug)]
pub(crate) struct CommandList {
    search_term: String,
}

impl CommandList {
    pub(crate) const fn new() -> Self {
        Self {
            search_term: String::new(),
        }
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

        let input_text_rectangle = buffer.draw_border(&input_rectangle, Color::Black);

        buffer.render_span(
            &Span::new(self.search_term.clone()).with_fg(Color::Black),
            &input_text_rectangle.offset(),
            &input_text_rectangle,
        );
    }

    fn handle_event(&mut self, event: &Event, _event_context: &mut EventContext) -> EventOutcome {
        match *event {
            Event::Key(key_event) => {
                if let KeyCode::Char(ch) = key_event.code
                    && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && !key_event.modifiers.contains(KeyModifiers::ALT)
                {
                    self.search_term.push(ch);
                    EventOutcome::Handled
                } else {
                    EventOutcome::Unhandled
                }
            }

            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Resize(_, _) => EventOutcome::Unhandled,
        }
    }

    fn visual_cursor_position(&self) -> Position {
        // TODO: implement properly
        Position::default()
    }

    fn handle_internal_events(&mut self) -> EventOutcome {
        EventOutcome::Unhandled
    }
}
