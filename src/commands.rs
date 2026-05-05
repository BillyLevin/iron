use std::cmp;

use crossterm::{
    event::{
        Event,
        KeyCode,
        KeyModifiers,
    },
    style::Color,
};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use crate::{
    buffer::Buffer,
    editor::{
        EditorAction,
        EventContext,
        EventOutcome,
    },
    keymap::KeyBinding,
    text::ByteIndex,
    ui::{
        Columns,
        Dimensions,
        Layer,
        LayerKind,
        Line,
        Position,
        Rectangle,
        Rows,
        Span,
    },
};

const COMMANDS: &[Command] = &[Command { name: "quit" }, Command { name: "write" }];

#[derive(Debug)]
struct Command {
    name: &'static str,
}

#[derive(Debug)]
pub(crate) struct CommandList {
    search_term: String,
    cursor_position: Option<Position>,
}

impl CommandList {
    pub(crate) const fn new() -> Self {
        Self {
            search_term: String::new(),
            cursor_position: None,
        }
    }

    fn apply_action(&mut self, action: CommandListAction, context: &mut EventContext) {
        match action {
            CommandListAction::InsertChar(ch) => {
                self.search_term.push(ch);
            }
            CommandListAction::Close => {
                context.push_action(EditorAction::RemoveLayer(LayerKind::CommandList));
            }
            CommandListAction::RemoveChar => {
                self.search_term.pop();
            }
        }
    }

    fn horizontal_scroll(&self, text_rectangle: &Rectangle) -> ByteIndex {
        let mut scroll = ByteIndex::new(0);
        let mut width = Columns::new(0);

        for grapheme in self.search_term.graphemes(true).rev() {
            width += Columns::new(grapheme.width());
            if width >= text_rectangle.width() {
                scroll += ByteIndex::new(grapheme.len());
            }
        }

        scroll
    }
}

impl Layer for CommandList {
    fn render(&mut self, buffer: &mut Buffer) {
        // TODO: figure out what to do when there's not enough room. i don't care about
        // it not working since the screen should never be that tiny, but i'd
        // rather the program didn't crash in that case.

        let app_rectangle = Rectangle::from_dimensions(buffer.dimensions());

        let rectangle = app_rectangle.at_center(Dimensions::new(
            app_rectangle.width() / 2,
            app_rectangle.height() / 2,
        ));

        buffer.fill_background(&rectangle, Color::Magenta);

        let (input_rectangle, commands_rectangle) = rectangle.split_at_row(Rows::new(3));

        let input_text_rectangle = buffer.draw_border(&input_rectangle, Color::Black);

        let text = self
            .search_term
            .get(self.horizontal_scroll(&input_text_rectangle).value()..)
            .expect("horizontal scroll should give a byte index on a char boundary");

        buffer.render_span(
            &Span::new(text.to_owned()).with_fg(Color::Black),
            &input_text_rectangle.offset(),
            &input_text_rectangle,
        );

        self.cursor_position = Some(input_text_rectangle.offset().col_offset(cmp::min(
            Columns::new(text.width()),
            input_text_rectangle.width() - Columns::new(1),
        )));

        let commands_rectangle = buffer.draw_border(&commands_rectangle, Color::Black);

        buffer.render_lines(
            COMMANDS
                .iter()
                .filter(|cmd| cmd.name.contains(&self.search_term))
                .map(|cmd| Line::new(vec![Span::new(cmd.name.to_owned()).with_fg(Color::Black)]))
                .collect(),
            &commands_rectangle,
        );
    }

    fn handle_event(&mut self, event: &Event, event_context: &mut EventContext) -> EventOutcome {
        match *event {
            Event::Key(key_event) => {
                let key_binding = KeyBinding::from(key_event);

                let action = match (key_binding.code(), key_binding.modifiers()) {
                    (KeyCode::Esc, KeyModifiers::NONE) => Some(CommandListAction::Close),
                    (KeyCode::Char(ch), KeyModifiers::NONE) => {
                        Some(CommandListAction::InsertChar(ch))
                    }
                    (KeyCode::Backspace, KeyModifiers::NONE) => Some(CommandListAction::RemoveChar),
                    _ => None,
                };

                action.map_or(EventOutcome::Unhandled, |a| {
                    self.apply_action(a, event_context);
                    EventOutcome::Handled
                })
            }

            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Resize(_, _) => EventOutcome::Unhandled,
        }
    }

    fn visual_cursor_position(&self) -> Option<Position> {
        self.cursor_position
    }

    fn handle_internal_events(&mut self) -> EventOutcome {
        EventOutcome::Unhandled
    }

    fn kind(&self) -> Option<LayerKind> {
        Some(LayerKind::CommandList)
    }
}

#[derive(Debug, Clone, Copy)]
enum CommandListAction {
    InsertChar(char),
    RemoveChar,
    Close,
}
