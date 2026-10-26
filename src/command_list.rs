use std::cmp;

use crossterm::event::{
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

use crate::{
    buffer::Buffer,
    editor::{
        EditorAction,
        EventContext,
        EventOutcome,
    },
    keymap::KeyBinding,
    style::Style,
    text::{
        ByteIndex,
        text_width,
    },
    ui::{
        Columns,
        Dimensions,
        Layer,
        LayerEvent,
        LayerKind,
        Line,
        MaybeBordered,
        Position,
        Rectangle,
        Rows,
        Span,
    },
};

const COMMANDS: &[Command] = &[
    Command {
        name: "quit",
        action: || EditorAction::Quit,
    },
    Command {
        name: "write",
        action: || EditorAction::Write,
    },
    Command {
        name: "write-quit",
        action: || EditorAction::WriteQuit,
    },
];

#[derive(Debug)]
struct Command {
    name: &'static str,
    action: fn() -> EditorAction,
}

impl AsRef<str> for Command {
    fn as_ref(&self) -> &str {
        self.name
    }
}

#[derive(Debug)]
pub(crate) struct CommandList {
    search_term: String,
    cursor_position: Option<Position>,
    /// The index of [`visible_commands`](Self::visible_commands) that is
    /// currently selected.
    selected_index: usize,
    /// List of references to each [`Command`] still visible after filtering.
    visible_commands: Vec<&'static Command>,
    matcher: Matcher,
    layout: Layout,
}

impl CommandList {
    pub(crate) fn new(app_dimensions: Dimensions) -> Self {
        Self {
            search_term: String::new(),
            cursor_position: None,
            selected_index: 0,
            visible_commands: Vec::new(),
            matcher: Matcher::new(Config::DEFAULT),
            layout: Layout::new(app_dimensions),
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
                if let Some(cmd) = self.visible_commands.get(self.selected_index) {
                    context.push_action((cmd.action)());
                    context.push_action(EditorAction::RemoveLayer(LayerKind::CommandList));
                }
            }
        }
    }

    fn horizontal_scroll(&self, text_rectangle: &Rectangle) -> ByteIndex {
        let mut scroll = ByteIndex::new(0);
        let mut width = Columns::new(0);

        for grapheme in self.search_term.graphemes(true).rev() {
            width += text_width(grapheme);
            if width >= text_rectangle.width() {
                scroll += ByteIndex::new(grapheme.len());
            }
        }

        scroll
    }
}

impl Layer for CommandList {
    fn render(&mut self, buffer: &mut Buffer) {
        debug_assert_eq!(
            self.layout,
            Layout::new(buffer.dimensions()),
            "resizes must be handled correctly"
        );

        self.cursor_position = None;

        let Layout {
            ref container,
            ref search_input,
            ref commands,
        } = self.layout;

        let input = search_input.inner();

        buffer.clear_and_style_rectangle(container, Style::COMMAND_LIST);

        buffer.draw_border(search_input, Style::COMMAND_LIST_BORDER);
        buffer.draw_border(commands, Style::COMMAND_LIST_BORDER);

        let text = self
            .search_term
            .get(self.horizontal_scroll(input).value()..)
            .expect("horizontal scroll should give a byte index on a char boundary");

        buffer.render_span(
            &Span::new(text).with_style(Style::COMMAND_LIST_INPUT_TEXT),
            &input.offset(),
            input,
        );

        self.cursor_position = Some(input.offset().col_offset(cmp::min(
            text_width(text),
            input.width().saturating_sub(1_usize),
        )));

        // TODO: check whether this re-allocates because of the `.collect()`
        // (match_list returns a Vec rather than iterator)
        self.visible_commands = Pattern::parse(
            &self.search_term,
            CaseMatching::Ignore,
            Normalization::Smart,
        )
        .match_list(COMMANDS, &mut self.matcher)
        .iter()
        .map(|&(cmd, _score)| cmd)
        .collect();

        buffer.render_lines(
            &self
                .visible_commands
                .iter()
                .enumerate()
                .map(|(i, cmd)| {
                    let span = if i == self.selected_index {
                        Span::new(cmd.name).with_style(Style::COMMAND_LIST_ITEM_SELECTED)
                    } else {
                        Span::new(cmd.name).with_style(Style::COMMAND_LIST_ITEM)
                    };

                    Line::new(vec![span])
                })
                .collect::<Vec<_>>(),
            commands.inner(),
        );
    }

    fn handle_event(
        &mut self,
        event: &LayerEvent,
        event_context: &mut EventContext,
    ) -> EventOutcome {
        match *event {
            LayerEvent::Key(key_event) => {
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

            LayerEvent::FocusGained
            | LayerEvent::FocusLost
            | LayerEvent::Mouse(_)
            | LayerEvent::Paste => EventOutcome::Unhandled,
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

    fn resize(&mut self, app_dimensions: Dimensions) {
        self.layout = Layout::new(app_dimensions);
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

#[derive(Debug, PartialEq, Eq)]
struct Layout {
    /// The entire command list.
    container: Rectangle,
    search_input: MaybeBordered,
    commands: MaybeBordered,
}

impl Layout {
    fn new(app_dimensions: Dimensions) -> Self {
        let app_rectangle = Rectangle::from_dimensions(app_dimensions);

        let container = app_rectangle.at_center(Dimensions::new(
            app_rectangle.width() / 2,
            app_rectangle.height() / 2,
        ));

        let (search_input, commands) = container.split_at_row(Rows::new(3));

        let search_input = MaybeBordered::new(search_input);
        let commands = MaybeBordered::new(commands);

        Self {
            container,
            search_input,
            commands,
        }
    }
}
