use std::{
    cmp,
    fs::File,
    io::{self, BufReader},
    ops::{self, Bound},
    path::PathBuf,
};

use crossterm::{event::KeyEvent, style::Color};
use ropey::{LineType, Rope};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation as _};
use unicode_width::UnicodeWidthStr;

use crate::{
    buffer::Buffer,
    keymap::{Action, KeyMap},
    terminal::{Columns, Dimensions, EventOutcome, Rows},
};

#[derive(Debug)]
pub(crate) struct Document {
    text: Rope,
    selection: Selection,
    normal_keymap: KeyMap,
    dimensions: Dimensions,

    /// Number of lines from the top of the file that the buffer text should start from
    scroll_offset: LineIndex,
}

impl Document {
    pub(crate) fn new(file_path: &PathBuf, dimensions: Dimensions) -> io::Result<Self> {
        Ok(Self {
            text: Rope::from_reader(BufReader::new(File::open(file_path)?))?,
            selection: Selection::default(),
            normal_keymap: KeyMap::normal(),
            dimensions,
            scroll_offset: LineIndex::default(),
        })
    }

    /// Fills the editor's [`Buffer`].
    ///
    /// This buffer will later be used to draw the content to the terminal
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

            assert!(position.left() >= &gutter_width);

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
        self.normal_keymap
            .get(key_event)
            .map_or(EventOutcome::Unhandled, |action| {
                self.apply_action(action);

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
        }
    }

    fn move_cursor_down(&mut self) {
        let line_index = self.byte_to_line(self.selection.cursor);
        let line_start = self.line_to_byte(line_index);

        let target_column: usize = self
            .text
            .slice(line_start.value()..self.selection.cursor.value())
            .chunks()
            .map(UnicodeWidthStr::width)
            .sum();

        let next_line_index = line_index + 1;
        if next_line_index.value() >= self.line_count() {
            return;
        }

        let next_line_start = self.line_to_byte(next_line_index);

        let mut column = 0;
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
        let line_index = self.byte_to_line(self.selection.cursor);

        let line_start = self.line_to_byte(line_index);

        let target_column: usize = self
            .text
            .slice(line_start.value()..self.selection.cursor.value())
            .chunks()
            .map(UnicodeWidthStr::width)
            .sum();

        let prev_line_index = line_index.saturating_sub(1);

        let prev_line_start = self.line_to_byte(prev_line_index);

        let mut column = 0;
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
        let text_slice = self.text.slice(..self.selection.cursor.value());

        let (mut chunk, mut chunk_start_index) = text_slice.chunk(self.selection.cursor.value());

        let mut grapheme_cursor =
            GraphemeCursor::new(self.selection.cursor.value(), text_slice.len(), true);

        let byte_index = loop {
            match grapheme_cursor.prev_boundary(chunk, chunk_start_index) {
                Ok(None) => break ByteIndex::from(0),
                Ok(Some(index)) => break ByteIndex::from(index),

                Err(GraphemeIncomplete::PrevChunk) => {
                    assert!(chunk_start_index > 0);
                    (chunk, chunk_start_index) = text_slice.chunk(chunk_start_index - 1);
                }

                Err(GraphemeIncomplete::PreContext(offset)) => {
                    assert!(offset > 0);
                    let (context_chunk, context_chunk_start) = text_slice.chunk(offset - 1);
                    grapheme_cursor.provide_context(context_chunk, context_chunk_start);
                }

                Err(GraphemeIncomplete::NextChunk | GraphemeIncomplete::InvalidOffset) => {
                    unreachable!()
                }
            }
        };

        self.set_cursor(byte_index);
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

    /// Ensures that the cursor does not go past the end of the file
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
        match grapheme {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, derive_more::From)]
struct ByteIndex(usize);

impl ByteIndex {
    const fn value(self) -> usize {
        self.0
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

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;

    use super::*;
    use std::io::Write as _;

    const TEST_DIMENSIONS: Dimensions = Dimensions::new(Columns::new(80usize), Rows::new(24usize));

    struct TestCase<'text> {
        initial_text: &'text str,
        initial_cursor: usize,
        expected_initial_visual_position: (usize, usize),

        keys: Vec<char>,

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

            for key in self.keys {
                let _ = document.handle_key_event(KeyCode::Char(key).into());
            }

            assert_eq!(document.text.to_string(), self.expected_text);
            assert_eq!(document.selection.cursor, self.expected_cursor.into());
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

            keys: vec!['l'; 8],

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

            keys: vec!['l'; 1],

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

            keys: vec!['h'; 1],

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

            keys: vec!['h'; 1],

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

            keys: vec!['j'; 1],

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

            keys: vec!['j'; 1],

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

            keys: vec!['j'; 1],

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

            keys: vec!['k'; 1],

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

            keys: vec!['k'; 1],

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

            keys: vec!['j'; 1],

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

            keys: [vec!['j'; 26], vec!['k'; 25]].concat(),

            expected_text: &text,
            expected_cursor: 6,
            // visual position stays the same because we scrolled up, keeping the
            // cursor on the first line
            expected_visual_position: (4, 0),
        }
        .run();
    }
}
