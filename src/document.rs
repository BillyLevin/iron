use std::{
    cmp,
    fs::File,
    io::{self, BufReader},
    ops::{self, Bound, ControlFlow},
    path::PathBuf,
};

use crossterm::{
    event::{KeyCode, KeyEvent, KeyModifiers},
    style::Color,
};
use itertools::Itertools as _;
use ropey::{LineType, Rope, RopeSlice};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation as _};
use unicode_width::UnicodeWidthStr;

use crate::{
    buffer::Buffer,
    keymap::{Action, KeyMap, KeySequence},
    terminal::{Columns, Dimensions, EventOutcome, Rows},
};

#[derive(Debug)]
pub(crate) struct Document {
    text: Rope,
    selection: Selection,

    normal_keymap: KeyMap,
    insert_keymap: KeyMap,

    dimensions: Dimensions,

    /// Number of lines from the top of the file that the buffer text should start from.
    scroll_offset: LineIndex,

    /// When navigating vertically, the cursor will be moved to the left if the next line is
    /// narrower than the current. We use this field to track where the cursor would ideally be so
    /// that we can move it there if the line is wide enough.
    ///
    /// The value is relative to the start of the **text**, and does NOT include the `gutter_width`.
    desired_cursor_column: Option<Columns>,

    mode: Mode,

    /// The keys that have been pressed which may add up to a registered keybinding. Used in
    /// the `KeyMap` lookups.
    key_sequence: KeySequence,
}

impl Document {
    pub(crate) fn new(file_path: &PathBuf, dimensions: Dimensions) -> io::Result<Self> {
        Ok(Self {
            text: Rope::from_reader(BufReader::new(File::open(file_path)?))?,
            selection: Selection::default(),
            normal_keymap: KeyMap::normal(),
            insert_keymap: KeyMap::insert(),
            dimensions,
            scroll_offset: LineIndex::default(),
            desired_cursor_column: None,
            mode: Mode::Normal,
            key_sequence: KeySequence::default(),
        })
    }

    /// Fills the editor's [`Buffer`].
    ///
    /// This buffer will later be used to draw the content to the terminal.
    pub(crate) fn render(&self, buffer: &mut Buffer) {
        let mut position = Position::default();

        let mut line_number = 1 + self.scroll_offset.value();

        let gutter_width = self.gutter_width();

        for grapheme in self
            .graphemes(self.line_to_byte(self.scroll_offset)..)
            .map(Grapheme::from)
        {
            if position.top() >= self.dimensions.height() {
                break;
            }

            if position.left() == &Columns::new(0) {
                let line_number_str =
                    format!("{line_number:>width$}", width = gutter_width.value());

                buffer[position]
                    .set_content(&line_number_str)
                    .set_foreground(Color::Black)
                    .set_background(Color::White);

                position = position.advance(&Grapheme::Text(&line_number_str));
            }

            assert!(
                position.left() >= &gutter_width,
                "filling in the gutter should've taken the position past the gutter"
            );

            buffer[position].set_content(match grapheme {
                Grapheme::LineBreak => " ",
                Grapheme::Text(text) => text,
            });

            if matches!(grapheme, Grapheme::LineBreak) {
                line_number += 1;
            }

            position = position.advance(&grapheme);
        }
    }

    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) -> EventOutcome {
        let keymap = match self.mode {
            Mode::Normal => &self.normal_keymap,
            Mode::Insert => &self.insert_keymap,
        };

        self.key_sequence.push(key_event);

        let maybe_action = match keymap.get(&self.key_sequence) {
            Some(&KeyMap::BindingPart { .. }) => {
                // the key sequence could form a binding with subsequent key events. since
                // we'd already pushed the latest event to the sequence store, we are done
                return EventOutcome::Handled;
            }
            Some(&KeyMap::Action(action)) => Some(action),
            None => {
                // the current key sequence does not form a binding for any of the registered
                // commands. therefore, we clear the sequence
                self.key_sequence.clear();

                // look for any fallback events that aren't registered in the map
                self.event_fallback(key_event)
            }
        };

        maybe_action.map_or(EventOutcome::Unhandled, |action| {
            self.apply_action(action);

            self.key_sequence.clear();

            self.clamp_cursor();
            self.recalculate_scroll();

            EventOutcome::Handled
        })
    }

    pub(crate) fn visual_cursor_position(&self) -> Position {
        let mut position = Position::default();
        let mut byte_index = self.line_to_byte(self.scroll_offset);

        for grapheme in self.graphemes(byte_index..) {
            if byte_index == self.selection.cursor {
                break;
            }

            byte_index += grapheme.len();

            position = position.advance(&Grapheme::from(grapheme));
        }

        position.col_offset(self.gutter_width())
    }

    const fn set_cursor(&mut self, index: ByteIndex) {
        self.selection.cursor = index;
    }

    fn apply_action(&mut self, action: Action) {
        match action {
            Action::MoveDown => self.move_cursor_down(),
            Action::MoveUp => self.move_cursor_up(),
            Action::MoveRight => self.move_cursor_right(),
            Action::MoveLeft => self.move_cursor_left(),
            Action::MoveNextWordStart => self.move_cursor_next_word_start(),
            Action::MovePrevWordStart => self.move_cursor_prev_word_start(),
            Action::SwitchToInsertMode => self.insert_mode(),
            Action::SwitchToNormalMode => self.normal_mode(),
            Action::InsertChar(ch) => self.insert_char(ch),
            Action::DeleteGrapheme => self.delete_grapheme(),
            Action::InsertNewline => self.insert_newline(),
            Action::MoveLineEnd => self.move_cursor_line_end(),
            Action::MoveLineStart => self.move_cursor_line_start(),
            Action::MoveLineFirstNonBlank => self.move_cursor_first_non_blank(),
            Action::MoveNextParagraph => self.move_cursor_next_paragraph(),
            Action::MovePrevParagraph => self.move_cursor_prev_paragraph(),
            Action::GoToLastLine => self.go_to_last_line(),
            Action::GoToFirstLine => self.go_to_first_line(),
        }

        if action.is_non_vertical_movement() {
            self.clear_desired_column();
        }
    }

    fn move_cursor_down(&mut self) {
        let target_column = self.update_desired_column();

        let next_line_index = self.byte_to_line(self.selection.cursor) + 1;
        if next_line_index.value() >= self.line_count() {
            return;
        }

        let next_line_start = self.line_to_byte(next_line_index);

        let mut column = Columns::new(0);
        let mut byte_index = next_line_start;

        for grapheme in self.graphemes(next_line_start..).map(Grapheme::from) {
            if column >= target_column {
                break;
            }

            match grapheme {
                Grapheme::LineBreak => break,
                Grapheme::Text(text) => {
                    column += text.width();
                    byte_index += text.len();
                }
            }
        }

        self.set_cursor(byte_index);
    }

    fn move_cursor_up(&mut self) {
        let target_column = self.update_desired_column();

        let prev_line_index = self.byte_to_line(self.selection.cursor).saturating_sub(1);
        let prev_line_start = self.line_to_byte(prev_line_index);

        let mut column = Columns::new(0);
        let mut byte_index = prev_line_start;

        for grapheme in self.graphemes(prev_line_start..).map(Grapheme::from) {
            if column >= target_column {
                break;
            }

            match grapheme {
                Grapheme::LineBreak => break,
                Grapheme::Text(text) => {
                    column += text.width();
                    byte_index += text.len();
                }
            }
        }

        self.set_cursor(byte_index);
    }

    fn move_cursor_right(&mut self) {
        let next_grapheme_offset = self
            .graphemes(self.selection.cursor..)
            .next()
            .map_or(0, str::len);

        self.set_cursor(self.selection.cursor + next_grapheme_offset);
    }

    fn move_cursor_left(&mut self) {
        self.set_cursor(self.previous_grapheme_position(self.selection.cursor));
    }

    fn move_cursor_next_word_start(&mut self) {
        let byte_index = match self
            .text
            .slice(self.selection.cursor.value()..)
            .chars()
            .tuple_windows()
            .try_fold(self.selection.cursor, |index, (prev, ch)| {
                let next_index = index + prev.len_utf8();

                if is_word_boundary(prev, ch) {
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        };

        self.set_cursor(byte_index);
    }

    fn move_cursor_prev_word_start(&mut self) {
        let byte_index = self
            .text
            .slice(..self.selection.cursor.value())
            .chars_at(self.selection.cursor.value())
            .reversed()
            .tuple_windows()
            .try_fold(self.selection.cursor, |index, (prev, ch)| {
                let next_index = index.saturating_sub(prev.len_utf8());

                if is_word_boundary(ch, prev) {
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            })
            .break_value()
            .unwrap_or(ByteIndex::new(0));

        self.set_cursor(byte_index);
    }

    /// Gets the byte index of the first byte of the previous grapheme from the given byte
    /// index.
    fn previous_grapheme_position(&self, from: ByteIndex) -> ByteIndex {
        let text_slice = self.text.slice(..from.value());

        let (mut chunk, mut chunk_start_index) = text_slice.chunk(from.value());

        let mut grapheme_cursor = GraphemeCursor::new(from.value(), text_slice.len(), true);

        loop {
            match grapheme_cursor.prev_boundary(chunk, chunk_start_index) {
                Ok(None) => break ByteIndex::from(0),
                Ok(Some(index)) => break ByteIndex::from(index),

                Err(GraphemeIncomplete::PrevChunk) => {
                    assert!(
                        chunk_start_index > 0,
                        "docs assert that `chunk_start_index` will be non-zero in this branch"
                    );
                    (chunk, chunk_start_index) = text_slice.chunk(chunk_start_index - 1);
                }

                Err(GraphemeIncomplete::PreContext(offset)) => {
                    assert!(
                        offset > 0,
                        "there should be a chunk that ends at `offset`, and therefore it must be non-zero"
                    );

                    let (context_chunk, context_chunk_start) = text_slice.chunk(offset - 1);
                    grapheme_cursor.provide_context(context_chunk, context_chunk_start);
                }

                Err(GraphemeIncomplete::NextChunk | GraphemeIncomplete::InvalidOffset) => {
                    unreachable!()
                }
            }
        }
    }

    fn byte_to_line(&self, byte: ByteIndex) -> LineIndex {
        LineIndex::from(
            self.text
                .byte_to_line_idx(byte.value(), ropey::LineType::LF_CR),
        )
    }

    fn line_to_byte(&self, line: LineIndex) -> ByteIndex {
        ByteIndex::from(self.text.line_to_byte_idx(line.value(), LineType::LF_CR))
    }

    fn line(&self, line_index: LineIndex) -> RopeSlice<'_> {
        self.text.line(line_index.value(), LineType::LF_CR)
    }

    fn graphemes(&self, range: impl ops::RangeBounds<ByteIndex>) -> impl Iterator<Item = &str> {
        let start = match range.start_bound() {
            Bound::Included(byte) => Bound::Included(byte.value()),
            Bound::Excluded(byte) => Bound::Excluded(byte.value()),
            Bound::Unbounded => Bound::Unbounded,
        };

        let end = match range.end_bound() {
            Bound::Included(byte) => Bound::Included(byte.value()),
            Bound::Excluded(byte) => Bound::Excluded(byte.value()),
            Bound::Unbounded => Bound::Unbounded,
        };

        self.text
            .slice((start, end))
            .chunks()
            .flat_map(|chunk| chunk.graphemes(true))
    }

    /// Ensures that the cursor does not go past the end of the file.
    fn clamp_cursor(&mut self) {
        self.set_cursor(cmp::min(
            self.selection.cursor,
            ByteIndex::from(self.text.len().saturating_sub(1)),
        ));
    }

    fn line_count(&self) -> usize {
        // NOTE: we are doing this because of:
        // https://docs.rs/ropey/2.0.0-beta.1/ropey/#a-note-about-line-breaks. if the file
        // has a trailing line break, ropey counts that in the line count, but we want to
        // act as if it doesn't exist. so, if the last line is empty, we'll lower the line
        // count
        let lines = self.text.len_lines(LineType::LF_CR);

        let last_line = self.text.line(lines.saturating_sub(1), LineType::LF_CR);

        if last_line.len() == 0 {
            lines.saturating_sub(1)
        } else {
            lines
        }
    }

    fn gutter_width(&self) -> Columns {
        Columns::from(cmp::max(3, number_of_digits(self.line_count())))
    }

    fn recalculate_scroll(&mut self) {
        let cursor_line = self.byte_to_line(self.selection.cursor);

        if cursor_line < self.scroll_offset {
            // upwards scroll
            self.scroll_offset = cursor_line;
        } else {
            let cursor = self.visual_cursor_position();

            // downwards scroll
            if cursor.top() >= self.dimensions.height() {
                self.scroll_offset +=
                    LineIndex::from(cursor.top().value() - self.dimensions.height().value() + 1);
            }
        }
    }

    /// Sets the desired column to the current cursor column if it's not already set. Returns the
    /// column for convenience to the caller.
    fn update_desired_column(&mut self) -> Columns {
        let column = self.desired_cursor_column.unwrap_or_else(|| {
            let line_start = self.line_to_byte(self.byte_to_line(self.selection.cursor));

            self.text
                .slice(line_start.value()..self.selection.cursor.value())
                .chunks()
                .map(UnicodeWidthStr::width)
                .map(Columns::new)
                .sum()
        });
        self.desired_cursor_column = Some(column);
        column
    }

    const fn clear_desired_column(&mut self) {
        self.desired_cursor_column = None;
    }

    const fn event_fallback(&self, key_event: KeyEvent) -> Option<Action> {
        match self.mode {
            Mode::Normal => None,
            Mode::Insert => {
                if let KeyCode::Char(ch) = key_event.code
                    && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && !key_event.modifiers.contains(KeyModifiers::ALT)
                {
                    Some(Action::InsertChar(ch))
                } else {
                    None
                }
            }
        }
    }

    const fn insert_mode(&mut self) {
        self.mode = Mode::Insert;
    }

    const fn normal_mode(&mut self) {
        self.mode = Mode::Normal;
    }

    fn insert_char(&mut self, ch: char) {
        self.text.insert_char(self.selection.cursor.value(), ch);
        self.set_cursor(self.selection.cursor + ch.len_utf8());
    }

    fn delete_grapheme(&mut self) {
        let start = self.previous_grapheme_position(self.selection.cursor);

        self.text
            .remove(start.value()..self.selection.cursor.value());

        self.set_cursor(start);
    }

    fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    fn move_cursor_line_end(&mut self) {
        let line_index = self.byte_to_line(self.selection.cursor);
        let line = self.line(line_index);

        let offset = ByteIndex::from(
            line.trailing_line_break_idx(LineType::LF_CR)
                .unwrap_or_else(|| line.len()),
        );

        self.set_cursor(self.line_to_byte(line_index) + offset);
    }

    fn move_cursor_line_start(&mut self) {
        self.set_cursor(self.line_to_byte(self.byte_to_line(self.selection.cursor)));
    }

    /// Moves the cursor to the first non-whitespace character on the current line.
    fn move_cursor_first_non_blank(&mut self) {
        let line_index = self.byte_to_line(self.selection.cursor);
        let line = self.line(line_index);

        let offset: ByteIndex = line
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .map(|ch| ByteIndex::new(ch.len_utf8()))
            .sum();

        self.set_cursor(self.line_to_byte(line_index) + offset);
    }

    fn move_cursor_next_paragraph(&mut self) {
        let line_index = self.byte_to_line(self.selection.cursor);

        let line_offset = self
            .text
            .lines_at(line_index.value(), LineType::LF_CR)
            .enumerate()
            .skip_while(|&(_i, line)| line.chars().all(char::is_whitespace))
            .find(|&(_i, line)| line.chars().all(char::is_whitespace))
            .map(|(i, _line)| i);

        self.set_cursor(match line_offset {
            Some(offset) => self.line_to_byte(line_index + offset),
            None => ByteIndex::new(self.text.len()),
        });
    }

    fn move_cursor_prev_paragraph(&mut self) {
        let line_index = self.byte_to_line(self.selection.cursor);

        let line_offset = self
            .text
            // NOTE: +1 because when we use `reversed()`, the iterator does not consume the
            // line at the provided index
            .lines_at(line_index.value() + 1, LineType::LF_CR)
            .reversed()
            .enumerate()
            .skip_while(|&(_i, line)| line.chars().all(char::is_whitespace))
            .find(|&(_i, line)| line.chars().all(char::is_whitespace))
            .map(|(i, _line)| i);

        #[expect(
            clippy::option_if_let_else,
            reason = "TODO: decide whether I want this lint or not"
        )]
        self.set_cursor(match line_offset {
            Some(offset) => self.line_to_byte(line_index.saturating_sub(offset)),
            None => ByteIndex::new(0),
        });
    }

    fn go_to_last_line(&mut self) {
        self.set_cursor(self.line_to_byte(LineIndex::new(self.line_count().saturating_sub(1))));
    }

    const fn go_to_first_line(&mut self) {
        self.set_cursor(ByteIndex::new(0));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Position {
    left: Columns,
    top: Rows,
}

impl Position {
    pub(crate) const fn top(&self) -> &Rows {
        &self.top
    }

    pub(crate) const fn left(&self) -> &Columns {
        &self.left
    }

    #[must_use]
    fn advance(self, grapheme: &Grapheme) -> Self {
        match *grapheme {
            Grapheme::LineBreak => Self {
                left: Columns::new(0),
                top: self.top + Rows::new(1),
            },
            Grapheme::Text(text) => Self {
                left: self.left + Columns::new(text.width()),
                top: self.top,
            },
        }
    }

    #[must_use]
    fn col_offset(&self, gutter_width: Columns) -> Self {
        Self {
            left: self.left + gutter_width,
            top: self.top,
        }
    }
}

#[derive(Debug)]
enum Grapheme<'grapheme> {
    LineBreak,
    Text(&'grapheme str),
}

impl<'grapheme> From<&'grapheme str> for Grapheme<'grapheme> {
    fn from(value: &'grapheme str) -> Self {
        match value {
            "\n" | "\r\n" => Self::LineBreak,
            _ => Self::Text(value),
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    derive_more::From,
    derive_more::Add,
    derive_more::Sum,
)]
struct ByteIndex(usize);

impl ByteIndex {
    const fn new(value: usize) -> Self {
        Self(value)
    }

    const fn value(self) -> usize {
        self.0
    }

    const fn saturating_sub(self, rhs: usize) -> Self {
        Self(self.0.saturating_sub(rhs))
    }
}

impl ops::Add<usize> for ByteIndex {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl ops::AddAssign<usize> for ByteIndex {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    derive_more::From,
    derive_more::Add,
    derive_more::AddAssign,
)]
struct LineIndex(usize);

impl LineIndex {
    const fn new(value: usize) -> Self {
        Self(value)
    }

    const fn value(self) -> usize {
        self.0
    }

    const fn saturating_sub(self, rhs: usize) -> Self {
        Self(self.0.saturating_sub(rhs))
    }
}

impl ops::Add<usize> for LineIndex {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

#[derive(Debug, Default)]
struct Selection {
    cursor: ByteIndex,
}

fn number_of_digits(value: usize) -> usize {
    (value.checked_ilog10().unwrap_or(0) + 1) as usize
}

#[derive(Debug, PartialEq, Eq)]
enum WordBoundaryKind {
    /// letters, digits, underscores.
    WordPart,
    Whitespace,
    Other,
}

impl From<char> for WordBoundaryKind {
    fn from(ch: char) -> Self {
        if ch.is_whitespace() {
            Self::Whitespace
        } else if ch.is_alphanumeric() || ch == '_' {
            Self::WordPart
        } else {
            Self::Other
        }
    }
}

fn is_word_boundary(prev_ch: char, current_ch: char) -> bool {
    let prev_kind = WordBoundaryKind::from(prev_ch);
    let current_kind = WordBoundaryKind::from(current_ch);

    prev_kind != current_kind && current_kind != WordBoundaryKind::Whitespace
}

#[derive(Debug)]
enum Mode {
    Normal,
    Insert,
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent};

    use super::*;
    use std::io::Write as _;

    macro_rules! key_event {
        ($key:literal) => {
            KeyEvent::from(KeyCode::Char($key))
        };

        ($key:ident) => {
            KeyEvent::from(KeyCode::$key)
        };
    }

    const TEST_DIMENSIONS: Dimensions = Dimensions::new(Columns::new(80), Rows::new(24));

    struct TestCase<'text> {
        initial_text: &'text str,
        initial_cursor: usize,
        expected_initial_visual_position: (usize, usize),

        keys: Vec<KeyEvent>,

        expected_text: &'text str,
        expected_cursor: usize,
        expected_visual_position: (usize, usize),
    }

    impl TestCase<'_> {
        fn run(self) {
            let _ = color_eyre::install();

            let mut document = doc(self.initial_text);

            document.set_cursor(self.initial_cursor.into());
            assert_position(
                "initial position",
                document.visual_cursor_position(),
                self.expected_initial_visual_position,
            );

            for event in self.keys {
                let _ = document.handle_key_event(event);
            }

            assert_eq!(
                document.text.to_string(),
                self.expected_text,
                "text is incorrect"
            );
            assert_eq!(
                document.selection.cursor,
                self.expected_cursor.into(),
                "cursor byte index is incorrect"
            );
            assert_position(
                "final position",
                document.visual_cursor_position(),
                self.expected_visual_position,
            );
        }
    }

    fn assert_position(label: &str, actual: Position, expected: (usize, usize)) {
        assert_eq!(
            actual.left(),
            &Columns::from(expected.0),
            "{label} did not match"
        );
        assert_eq!(
            actual.top(),
            &Rows::from(expected.1),
            "{label} did not match"
        );
    }

    fn doc(contents: &str) -> Document {
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        write!(temp_file, "{contents}").unwrap();

        Document::new(&temp_file.path().to_path_buf(), TEST_DIMENSIONS).unwrap()
    }

    #[test]
    fn move_cursor_right() {
        TestCase {
            initial_text: "Test ⚒️ 😀 ",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('l'); 8],

            expected_text: "Test ⚒️ 😀 ",
            expected_cursor: 16,
            expected_visual_position: (13, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_right_from_end() {
        TestCase {
            initial_text: "Test",
            initial_cursor: 3,
            expected_initial_visual_position: (6, 0),

            keys: vec![key_event!('l'); 1],

            expected_text: "Test",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_left() {
        TestCase {
            initial_text: "Test ⚒️ 😀 ",
            initial_cursor: 16,
            expected_initial_visual_position: (13, 0),

            keys: vec![key_event!('h'); 1],

            expected_text: "Test ⚒️ 😀 ",
            expected_cursor: 12,
            expected_visual_position: (11, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_left_from_start() {
        TestCase {
            initial_text: "Test",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('h'); 1],

            expected_text: "Test",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_down() {
        TestCase {
            initial_text: "Test\nTest",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('j'); 1],

            expected_text: "Test\nTest",
            expected_cursor: 5,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_down_from_bottom() {
        TestCase {
            initial_text: "Test\nTest",
            initial_cursor: 5,
            expected_initial_visual_position: (3, 1),

            keys: vec![key_event!('j'); 1],

            expected_text: "Test\nTest",
            expected_cursor: 5,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_down_from_bottom_trailing_newline() {
        TestCase {
            initial_text: "Test\nTest\n",
            initial_cursor: 5,
            expected_initial_visual_position: (3, 1),

            keys: vec![key_event!('j'); 1],

            expected_text: "Test\nTest\n",
            expected_cursor: 5,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_up() {
        TestCase {
            initial_text: "Test\nTest",
            initial_cursor: 5,
            expected_initial_visual_position: (3, 1),

            keys: vec![key_event!('k'); 1],

            expected_text: "Test\nTest",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_up_from_top() {
        TestCase {
            initial_text: "Test\nTest",
            initial_cursor: 1,
            expected_initial_visual_position: (4, 0),

            keys: vec![key_event!('k'); 1],

            expected_text: "Test\nTest",
            expected_cursor: 1,
            expected_visual_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn scroll_down() {
        let text = "Test\n".repeat(30);

        TestCase {
            initial_text: &text,
            initial_cursor: 23 * 5,
            expected_initial_visual_position: (3, 23),

            keys: vec![key_event!('j'); 1],

            expected_text: &text,
            expected_cursor: 24 * 5,
            // visual position stays the same because we scrolled down, keeping the
            // cursor on the final line
            expected_visual_position: (3, 23),
        }
        .run();
    }

    #[test]
    fn scroll_up() {
        let text = "Test\n".repeat(30);

        TestCase {
            initial_text: &text,
            initial_cursor: 1,
            expected_initial_visual_position: (4, 0),

            keys: [vec![key_event!('j'); 26], vec![key_event!('k'); 25]].concat(),

            expected_text: &text,
            expected_cursor: 6,
            // visual position stays the same because we scrolled up, keeping the
            // cursor on the first line
            expected_visual_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn maintain_column_down() {
        TestCase {
            initial_text: "Long line\nShort\nLong line",
            initial_cursor: 8,
            expected_initial_visual_position: (11, 0),

            keys: vec![key_event!('j'); 2],

            expected_text: "Long line\nShort\nLong line",
            expected_cursor: 24,
            expected_visual_position: (11, 2),
        }
        .run();
    }

    #[test]
    fn maintain_column_up() {
        TestCase {
            initial_text: "Long line\nShort\nLong line",
            initial_cursor: 24,
            expected_initial_visual_position: (11, 2),

            keys: vec![key_event!('k'); 2],

            expected_text: "Long line\nShort\nLong line",
            expected_cursor: 8,
            expected_visual_position: (11, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start() {
        TestCase {
            initial_text: "Test text",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('w')],

            expected_text: "Test text",
            expected_cursor: 5,
            expected_visual_position: (8, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_emoji() {
        TestCase {
            initial_text: "😀 hello",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('w')],

            expected_text: "😀 hello",
            expected_cursor: 5,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_multiple() {
        TestCase {
            initial_text: "hello world test",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('w'); 3],

            expected_text: "hello world test",
            expected_cursor: 15,
            expected_visual_position: (18, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_mid_word() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 1,
            expected_initial_visual_position: (4, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello world",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_at_space() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 5,
            expected_initial_visual_position: (8, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello world",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_end_of_file() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 6,
            expected_initial_visual_position: (9, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello world",
            expected_cursor: 10,
            expected_visual_position: (13, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_punctuation_to_word() {
        TestCase {
            initial_text: "hello, world",
            initial_cursor: 5,
            expected_initial_visual_position: (8, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello, world",
            expected_cursor: 7,
            expected_visual_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_multiple_spaces() {
        TestCase {
            initial_text: "hello    world",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello    world",
            expected_cursor: 9,
            expected_visual_position: (12, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_with_numbers() {
        TestCase {
            initial_text: "test123 abc456",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('w')],

            expected_text: "test123 abc456",
            expected_cursor: 8,
            expected_visual_position: (11, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_ignore_underscore() {
        TestCase {
            initial_text: "hello_world test",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello_world test",
            expected_cursor: 12,
            expected_visual_position: (15, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_across_lines() {
        TestCase {
            initial_text: "hello\nworld",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello\nworld",
            expected_cursor: 6,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_empty_file() {
        TestCase {
            initial_text: "",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('w')],

            expected_text: "",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start() {
        TestCase {
            initial_text: "Test text",
            initial_cursor: 5,
            expected_initial_visual_position: (8, 0),

            keys: vec![key_event!('b')],

            expected_text: "Test text",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_emoji() {
        TestCase {
            initial_text: "😀 hello",
            initial_cursor: 5,
            expected_initial_visual_position: (6, 0),

            keys: vec![key_event!('b')],

            expected_text: "😀 hello",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_multiple() {
        TestCase {
            initial_text: "hello world test",
            initial_cursor: 15,
            expected_initial_visual_position: (18, 0),

            keys: vec![key_event!('b'); 2],

            expected_text: "hello world test",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_mid_word() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 8,
            expected_initial_visual_position: (11, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello world",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_at_space() {
        TestCase {
            initial_text: "hello world ",
            initial_cursor: 11,
            expected_initial_visual_position: (14, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello world ",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_start_of_file() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello world",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_punctuation_to_word() {
        TestCase {
            initial_text: "hello world  ,",
            initial_cursor: 13,
            expected_initial_visual_position: (16, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello world  ,",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_with_numbers() {
        TestCase {
            initial_text: "test123 abc456",
            initial_cursor: 8,
            expected_initial_visual_position: (11, 0),

            keys: vec![key_event!('b')],

            expected_text: "test123 abc456",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_ignore_underscore() {
        TestCase {
            initial_text: "hello_world test",
            initial_cursor: 12,
            expected_initial_visual_position: (15, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello_world test",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_across_lines() {
        TestCase {
            initial_text: "hello\nworld",
            initial_cursor: 6,
            expected_initial_visual_position: (3, 1),

            keys: vec![key_event!('b')],

            expected_text: "hello\nworld",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_empty_file() {
        TestCase {
            initial_text: "",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('b')],

            expected_text: "",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn insert() {
        TestCase {
            initial_text: "lo",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![
                key_event!('i'),
                key_event!('H'),
                key_event!('e'),
                key_event!('l'),
            ],

            expected_text: "Hello",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn delete_simple() {
        TestCase {
            initial_text: "Hello!",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('i'), key_event!(Backspace)],

            expected_text: "Hllo!",
            expected_cursor: 1,
            expected_visual_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn delete_emoji() {
        TestCase {
            initial_text: "Hello ⚒️ !!",
            initial_cursor: 12,
            expected_initial_visual_position: (11, 0),

            keys: vec![key_event!('i'), key_event!(Backspace)],

            expected_text: "Hello  !!",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn delete_and_insert() {
        TestCase {
            initial_text: "Hello!!",
            initial_cursor: 6,
            expected_initial_visual_position: (9, 0),

            keys: vec![
                key_event!('i'),
                key_event!(Backspace),
                key_event!(Backspace),
                key_event!(Backspace),
                key_event!(Backspace),
                key_event!('y'),
            ],
            expected_text: "Hey!",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn normal_mode() {
        TestCase {
            initial_text: "Hello!",
            initial_cursor: 5,
            expected_initial_visual_position: (8, 0),

            keys: vec![
                key_event!('i'),
                key_event!('!'),
                key_event!(Esc),
                key_event!('h'),
                key_event!('h'),
            ],
            expected_text: "Hello!!",
            expected_cursor: 4,
            expected_visual_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn insert_newline() {
        TestCase {
            initial_text: "Hello!",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('i'), key_event!(Enter)],
            expected_text: "He\nllo!",
            expected_cursor: 3,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_line_end() {
        TestCase {
            initial_text: "Hello!!",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('$')],

            expected_text: "Hello!!",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_line_end_with_lf() {
        TestCase {
            initial_text: "Hello!!\nNext line",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('$')],

            expected_text: "Hello!!\nNext line",
            expected_cursor: 7,
            expected_visual_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_line_end_with_crlf() {
        TestCase {
            initial_text: "Hello!!\r\nNext line",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('$')],

            expected_text: "Hello!!\r\nNext line",
            expected_cursor: 7,
            expected_visual_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_line_start() {
        TestCase {
            initial_text: "Hello!!",
            initial_cursor: 3,
            expected_initial_visual_position: (6, 0),

            keys: vec![key_event!('0')],

            expected_text: "Hello!!",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_first_non_blank() {
        TestCase {
            initial_text: "   Hello!!",
            initial_cursor: 7,
            expected_initial_visual_position: (10, 0),

            keys: vec![key_event!('^')],

            expected_text: "   Hello!!",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_paragraph() {
        TestCase {
            initial_text: "hello\nworld\n\nparagraph",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('}')],

            expected_text: "hello\nworld\n\nparagraph",
            expected_cursor: 12,
            expected_visual_position: (3, 2),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_paragraph_consecutive_empty_lines() {
        TestCase {
            initial_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('}'); 2],

            expected_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            expected_cursor: 26,
            expected_visual_position: (3, 7),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_paragraph() {
        TestCase {
            initial_text: "hello\n\nworld\n\n",
            initial_cursor: 13,
            expected_initial_visual_position: (3, 3),

            keys: vec![key_event!('{')],

            expected_text: "hello\n\nworld\n\n",
            expected_cursor: 6,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_paragraph_consecutive_empty_lines() {
        TestCase {
            initial_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            initial_cursor: 26,
            expected_initial_visual_position: (3, 7),

            keys: vec![key_event!('{'); 2],

            expected_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn go_to_last_line() {
        TestCase {
            initial_text: "hello\nworld\n",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('G')],

            expected_text: "hello\nworld\n",
            expected_cursor: 6,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn go_to_last_line_alias() {
        TestCase {
            initial_text: "hello\nworld\n",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('g'), key_event!('e')],

            expected_text: "hello\nworld\n",
            expected_cursor: 6,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn go_to_first_line() {
        TestCase {
            initial_text: "hello\nworld\n",
            initial_cursor: 6,
            expected_initial_visual_position: (3, 1),

            keys: vec![key_event!('g'), key_event!('g')],

            expected_text: "hello\nworld\n",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }
}
