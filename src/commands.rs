use std::cmp;

use crossterm::event::{
    Event,
    KeyCode,
    KeyModifiers,
};
use nucleo_matcher::{
    Config,
    Matcher,
    pattern::{
        CaseMatching,
        Normalization,
        Pattern,
    },
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
    style::Style,
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

const COMMANDS: &[Command] = &[
    Command {
        name: "quit",
        action: EditorAction::Quit,
    },
    Command {
        name: "write",
        action: EditorAction::Write,
    },
    Command {
        name: "write-quit",
        action: EditorAction::WriteQuit,
    },
];

#[derive(Debug)]
struct Command {
    name: &'static str,
    action: EditorAction,
}

#[derive(Debug)]
struct EnumeratedCommand {
    inner: (usize, &'static Command),
}

impl EnumeratedCommand {
    const fn new(value: (usize, &'static Command)) -> Self {
        Self { inner: value }
    }
}

impl AsRef<str> for EnumeratedCommand {
    fn as_ref(&self) -> &str {
        self.inner.1.name
    }
}

#[derive(Debug)]
pub(crate) struct CommandList {
    search_term: String,
    cursor_position: Option<Position>,
    /// The index of [`visible_commands`](Self::visible_commands) that is
    /// currently selected.
    selected_index: usize,
    // List of indices into `COMMANDS` of the filtered commands.
    visible_commands: Vec<usize>,
    matcher: Matcher,
}

impl CommandList {
    pub(crate) fn new() -> Self {
        Self {
            search_term: String::new(),
            cursor_position: None,
            selected_index: 0,
            visible_commands: Vec::new(),
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    fn apply_action(&mut self, action: CommandListAction, context: &mut EventContext) {
        match action {
            CommandListAction::InsertChar(ch) => {
                self.search_term.push(ch);
                self.selected_index = 0;
            }
            CommandListAction::Close => {
                context.push_action(EditorAction::RemoveLayer(LayerKind::CommandList));
            }
            CommandListAction::RemoveChar => {
                self.search_term.pop();
                self.selected_index = 0;
            }
            CommandListAction::GoToNextCommand => {
                self.selected_index = (self.selected_index + 1)
                    .checked_rem_euclid(self.visible_commands.len())
                    .unwrap_or(0);
            }
            CommandListAction::GoToPrevCommand => {
                self.selected_index = self
                    .selected_index
                    .checked_sub(1)
                    .unwrap_or_else(|| self.visible_commands.len().saturating_sub(1));
            }
            CommandListAction::ExecuteCommand => {
                if let Some(i) = self.visible_commands.get(self.selected_index) {
                    let cmd = &COMMANDS[*i];
                    context.push_action(cmd.action);
                    context.push_action(EditorAction::RemoveLayer(LayerKind::CommandList));
                }
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

        buffer.clear_and_style_rectangle(&rectangle, Style::COMMAND_LIST);

        let (input_rectangle, commands_rectangle) = rectangle.split_at_row(Rows::new(3));

        let input_text_rectangle = buffer.draw_border(&input_rectangle, Style::COMMAND_LIST_BORDER);

        let text = self
            .search_term
            .get(self.horizontal_scroll(&input_text_rectangle).value()..)
            .expect("horizontal scroll should give a byte index on a char boundary");

        buffer.render_span(
            &Span::new(text).with_style(Style::COMMAND_LIST_INPUT_TEXT),
            &input_text_rectangle.offset(),
            &input_text_rectangle,
        );

        self.cursor_position = Some(input_text_rectangle.offset().col_offset(cmp::min(
            Columns::new(text.width()),
            input_text_rectangle.width() - Columns::new(1),
        )));

        let commands_rectangle =
            buffer.draw_border(&commands_rectangle, Style::COMMAND_LIST_BORDER);

        // TODO: check whether this re-allocates because of the `.collect()` (match_list
        // returns a Vec rather than iterator)
        self.visible_commands = Pattern::parse(
            &self.search_term,
            CaseMatching::Ignore,
            Normalization::Smart,
        )
        .match_list(
            COMMANDS.iter().enumerate().map(EnumeratedCommand::new),
            &mut self.matcher,
        )
        .iter()
        .map(|&(ref cmd, _score)| cmd.inner.0)
        .collect();

        buffer.render_lines(
            self.visible_commands
                .iter()
                .map(|i| &COMMANDS[*i])
                .enumerate()
                .map(|(i, cmd)| {
                    let span = if i == self.selected_index {
                        Span::new(cmd.name).with_style(Style::COMMAND_LIST_ITEM_SELECTED)
                    } else {
                        Span::new(cmd.name).with_style(Style::COMMAND_LIST_ITEM)
                    };

                    Line::new(vec![span])
                })
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
                    (KeyCode::Down, KeyModifiers::NONE) => Some(CommandListAction::GoToNextCommand),
                    (KeyCode::Up, KeyModifiers::NONE) => Some(CommandListAction::GoToPrevCommand),
                    (KeyCode::Enter, KeyModifiers::NONE) => Some(CommandListAction::ExecuteCommand),
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
    GoToNextCommand,
    GoToPrevCommand,
    ExecuteCommand,
}
