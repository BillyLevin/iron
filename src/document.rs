use std::{
    cmp,
    fs::File,
    io::{
        self,
        BufReader,
    },
    iter,
    ops::ControlFlow,
    path::PathBuf,
};

use crossterm::{
    event::{
        KeyCode,
        KeyEvent,
        KeyModifiers,
    },
    style::Color,
};
use itertools::Itertools as _;
use ropey::{
    LineType,
    Rope,
};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr;

use crate::{
    buffer::Buffer,
    grapheme_layout::{
        GraphemeLayoutIterator,
        WrapBehavior,
    },
    keymap::{
        Action,
        KeyBinding,
        KeyMap,
        KeySequence,
    },
    terminal::{
        Columns,
        Dimensions,
        EventOutcome,
        Rows,
    },
    text::{
        ByteIndex,
        LeftChar,
        LineIndex,
        RightChar,
        RopeSliceExt as _,
        VisualLineInfo,
    },
};

#[derive(Debug)]
pub(crate) struct Document {
    text: Rope,
    selection: Selection,

    normal_keymap: KeyMap,
    insert_keymap: KeyMap,

    dimensions: Dimensions,

    /// Number of lines from the top of the file that the buffer text should
    /// start from.
    scroll_offset: LineIndex,

    /// When navigating vertically, the cursor will be moved to the left if the
    /// next line is narrower than the current. We use this field to track
    /// where the cursor would ideally be so that we can move it there if
    /// the line is wide enough.
    ///
    /// The value is relative to the start of the **text**, and does NOT include
    /// the `gutter_width`.
    desired_cursor_column: Option<Columns>,

    mode: Mode,

    /// The keys that have been pressed which may add up to a registered
    /// keybinding. Used in the `KeyMap` lookups.
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
        let mut line_number = 1 + self.scroll_offset.value();

        let gutter_width = self.gutter_width();

        let start_byte = self.text.slice(..).line_start_byte(self.scroll_offset);
        let text = self.text.slice(start_byte.value()..);

        for visual_grapheme in
            GraphemeLayoutIterator::new(text.graphemes(), self.max_text_width(), WrapBehavior::Wrap)
        {
            if visual_grapheme.position().top() >= self.dimensions.height() {
                break;
            }

            if visual_grapheme.position().left() == &Columns::new(0) {
                // we only display the line number on the first visual row of a wrapped
                // line; the rest are just empty
                let gutter_contents = if visual_grapheme.is_wrapped() {
                    " ".repeat(gutter_width.value())
                } else {
                    format!("{line_number:>width$}", width = gutter_width.value())
                };

                buffer[visual_grapheme.position()]
                    .set_content(&gutter_contents)
                    .set_foreground(Color::Black)
                    .set_background(Color::White);
            }

            let translated_position = visual_grapheme.position().col_offset(gutter_width);

            assert!(
                *translated_position.left() >= gutter_width,
                "filling in the gutter should've taken the position past the gutter"
            );

            let grapheme = visual_grapheme.grapheme();

            buffer[translated_position].set_content(grapheme.as_str());

            if matches!(grapheme, Grapheme::LineBreak) {
                line_number += 1;
            }
        }

        self.render_key_hint(buffer);
    }

    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) -> EventOutcome {
        let keymap = match self.mode {
            Mode::Normal => &self.normal_keymap,
            Mode::Insert => &self.insert_keymap,
        };

        self.key_sequence.push(KeyBinding::from(key_event));

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
        let start = self.text.slice(..).line_start_byte(self.scroll_offset);

        GraphemeLayoutIterator::new(
            self.text.slice(start.value()..).graphemes(),
            self.max_text_width(),
            WrapBehavior::Wrap,
        )
        .find(|grapheme| start + grapheme.byte_index() >= self.selection.cursor)
        .map(|grapheme| grapheme.position())
        .unwrap_or_default()
        .col_offset(self.gutter_width())
    }

    /// Displays the keybindings (if any) that are currently possible for the
    /// user to invoke, based on the current sequence of key events.
    fn render_key_hint(&self, buffer: &mut Buffer) {
        if self.key_sequence.is_empty() {
            return;
        }

        let keymap = match self.mode {
            Mode::Normal => &self.normal_keymap,
            Mode::Insert => &self.insert_keymap,
        };

        let Some(&KeyMap::BindingPart { ref map }) = keymap.get(&self.key_sequence) else {
            return;
        };

        let (hints, max_width, max_height) = map.iter().fold(
            (String::new(), Columns::new(0), Rows::new(0)),
            |(hints, max_width, max_height), (event, map_node)| {
                let key = event.to_string();
                let label = match *map_node {
                    KeyMap::BindingPart { .. } => "...",
                    KeyMap::Action(action) => action.label(),
                };

                let hint = format!("{key}: {label}\n");

                (
                    hints + &hint,
                    cmp::max(max_width, Columns::new(hint.width())),
                    // we will disable wrapping so we can simply increment the count
                    max_height + Rows::new(1),
                )
            },
        );

        let width = cmp::min(*self.dimensions.width(), max_width);
        let height = cmp::min(*self.dimensions.height(), max_height);

        let offset = Offset::new(
            *self.dimensions.width() - width,
            *self.dimensions.height() - height,
        );

        let top_left = Position::default().offset(offset);

        for position in top_left.area_iter(width, height) {
            buffer[position]
                .reset()
                .set_background(Color::Rgb {
                    r: 235,
                    g: 219,
                    b: 178,
                })
                .set_foreground(Color::White);
        }

        for visual_grapheme in
            GraphemeLayoutIterator::new(hints.graphemes(true), width, WrapBehavior::NoWrap)
        {
            if *visual_grapheme.position().top() >= height {
                break;
            }

            // wrapping has been turned off, and therefore we ignore all graphemes
            // that would overflow
            if *visual_grapheme.position().left() >= width {
                continue;
            }

            buffer[visual_grapheme.position().offset(offset)]
                .set_content(visual_grapheme.grapheme().as_str());
        }
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
            Action::DeleteWord => self.delete_word(),
            Action::ChangeWord => self.change_word(),
            Action::DeleteToLineEnd => self.delete_to_line_end(),
            Action::ChangeToLineEnd => self.change_to_line_end(),
            Action::DeleteToLineStart => self.delete_to_line_start(),
            Action::DeleteToLineFirstNonBlank => self.delete_to_first_non_blank(),
            Action::DeleteLine => self.delete_line(),
            Action::DeleteWholeWord => self.delete_whole_word(),
            Action::DeleteToPrevWordStart => self.delete_to_prev_word_start(),
        }

        if action.is_non_vertical_movement() {
            self.clear_desired_column();
        }
    }

    fn move_cursor_down(&mut self) {
        let target_column = self.desired_column();
        let text = self.text.slice(..);

        let byte = VisualLineInfo::new(
            &self.text,
            text.line_idx_containing_byte(self.selection.cursor),
            self.max_text_width(),
        )
        .next_at_column(self.selection.cursor, target_column);

        if let Some(byte_index) = byte {
            self.set_cursor(byte_index);
        }
    }

    fn move_cursor_up(&mut self) {
        let target_column = self.desired_column();

        let text = self.text.slice(..);

        let byte = VisualLineInfo::new(
            &self.text,
            text.line_idx_containing_byte(self.selection.cursor),
            self.max_text_width(),
        )
        .prev_at_column(self.selection.cursor, target_column);

        if let Some(byte_index) = byte {
            self.set_cursor(byte_index);
        }
    }

    fn move_cursor_right(&mut self) {
        let next_grapheme_offset = self
            .text
            .slice(self.selection.cursor.value()..)
            .graphemes()
            .next()
            .map_or(0, str::len);

        self.set_cursor(self.selection.cursor + next_grapheme_offset);
    }

    fn move_cursor_left(&mut self) {
        self.set_cursor(
            self.text
                .slice(..)
                .previous_grapheme_position(self.selection.cursor),
        );
    }

    fn move_cursor_next_word_start(&mut self) {
        let byte_index = match self
            .text
            .slice(self.selection.cursor.value()..)
            .chars()
            .tuple_windows()
            .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                let next_index = index + left.ch().len_utf8();

                if right.is_word_start(left) {
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
            .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                let next_index = index.saturating_sub(left.ch().len_utf8());

                if right.is_word_start(left) {
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            })
            .break_value()
            .unwrap_or(ByteIndex::new(0));

        self.set_cursor(byte_index);
    }

    /// Ensures that the cursor does not go past the end of the file.
    fn clamp_cursor(&mut self) {
        self.set_cursor(cmp::min(
            self.selection.cursor,
            ByteIndex::new(self.text.slice(..).len().saturating_sub(1)),
        ));
    }

    fn gutter_width(&self) -> Columns {
        Columns::from(cmp::max(
            3,
            number_of_digits(self.text.slice(..).line_count()),
        ))
    }

    fn recalculate_scroll(&mut self) {
        let text = self.text.slice(..);
        let cursor_line = text.line_idx_containing_byte(self.selection.cursor);

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

        self.scroll_offset = cmp::min(cursor_line, self.scroll_offset);
    }

    /// Gets (or inserts the current cursor column) the desired column to
    /// navigate to on vertical cursor movement.
    fn desired_column(&mut self) -> Columns {
        let width = self.max_text_width();

        *self.desired_cursor_column.get_or_insert_with(|| {
            let text = self.text.slice(..);

            let line_start =
                text.line_start_byte(text.line_idx_containing_byte(self.selection.cursor));

            text.slice(line_start.value()..self.selection.cursor.value())
                .chunks()
                .map(UnicodeWidthStr::width)
                .map(Columns::new)
                .sum::<Columns>()
                .map(|cols| cols % width.value())
        })
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
        let start = self
            .text
            .slice(..)
            .previous_grapheme_position(self.selection.cursor);

        self.text
            .remove(start.value()..self.selection.cursor.value());

        self.set_cursor(start);
    }

    fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    fn move_cursor_line_end(&mut self) {
        let text = self.text.slice(..);

        let line_index = text.line_idx_containing_byte(self.selection.cursor);
        let line = text.line_at(line_index);

        let offset = ByteIndex::from(
            line.trailing_line_break_idx(LineType::LF_CR)
                .unwrap_or_else(|| line.len()),
        );

        self.set_cursor(text.line_start_byte(line_index) + offset);
    }

    fn move_cursor_line_start(&mut self) {
        let text = self.text.slice(..);

        self.set_cursor(text.line_start_byte(text.line_idx_containing_byte(self.selection.cursor)));
    }

    /// Moves the cursor to the first non-whitespace character on the current
    /// line.
    fn move_cursor_first_non_blank(&mut self) {
        let text = self.text.slice(..);
        let line_index = text.line_idx_containing_byte(self.selection.cursor);
        let line = text.line_at(line_index);

        let offset: ByteIndex = line
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .map(|ch| ByteIndex::new(ch.len_utf8()))
            .sum();

        self.set_cursor(text.line_start_byte(line_index) + offset);
    }

    fn move_cursor_next_paragraph(&mut self) {
        let text = self.text.slice(..);
        let line_index = text.line_idx_containing_byte(self.selection.cursor);

        let line_offset = self
            .text
            .lines_at(line_index.value(), LineType::LF_CR)
            .enumerate()
            .skip_while(|&(_i, line)| line.is_whitespace())
            .find(|&(_i, line)| line.is_whitespace())
            .map(|(i, _line)| i);

        #[expect(
            clippy::option_if_let_else,
            reason = "TODO: decide whether I want this lint"
        )]
        self.set_cursor(match line_offset {
            Some(offset) => text.line_start_byte(line_index + offset),
            None => ByteIndex::new(text.len()),
        });
    }

    fn move_cursor_prev_paragraph(&mut self) {
        let text = self.text.slice(..);
        let line_index = text.line_idx_containing_byte(self.selection.cursor);

        let line_offset = self
            .text
            // NOTE: +1 because when we use `reversed()`, the iterator does not consume the
            // line at the provided index
            .lines_at(line_index.value() + 1, LineType::LF_CR)
            .reversed()
            .enumerate()
            .skip_while(|&(_i, line)| line.is_whitespace())
            .find(|&(_i, line)| line.is_whitespace())
            .map(|(i, _line)| i);

        #[expect(
            clippy::option_if_let_else,
            reason = "TODO: decide whether I want this lint or not"
        )]
        self.set_cursor(match line_offset {
            Some(offset) => text.line_start_byte(line_index.saturating_sub(offset)),
            None => ByteIndex::new(0),
        });
    }

    fn go_to_last_line(&mut self) {
        let text = self.text.slice(..);

        self.set_cursor(text.line_start_byte(text.last_line_idx()));
    }

    const fn go_to_first_line(&mut self) {
        self.set_cursor(ByteIndex::new(0));
    }

    /// Determines the maximum room for text based on the dimensions of the
    /// [`Document`] and the size of its gutter.
    fn max_text_width(&self) -> Columns {
        // TODO: what about the unlikely case that width <= gutter_width? add an assert
        // and panic? allow weird behaviour? explicitly handle?
        *self.dimensions.width() - self.gutter_width()
    }

    /// Deletes from the current cursor position up to (but not including) the
    /// start of the next word.
    fn delete_word(&mut self) {
        let end = match self
            .text
            .slice(self.selection.cursor.value()..)
            .chars()
            .tuple_windows()
            .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                let next_index = index + left.ch().len_utf8();

                if right.is_word_start(left) {
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        };

        self.text.remove(self.selection.cursor.value()..end.value());
    }

    fn change_word(&mut self) {
        self.delete_word();
        self.insert_mode();
    }

    fn delete_to_line_end(&mut self) {
        let text = self.text.slice(..);

        let line_index = text.line_idx_containing_byte(self.selection.cursor);
        let line = text.line_at(line_index);

        let offset = ByteIndex::from(
            line.trailing_line_break_idx(LineType::LF_CR)
                .unwrap_or_else(|| line.len()),
        );

        let end = text.line_start_byte(line_index) + offset;

        self.text.remove(self.selection.cursor.value()..end.value());
    }

    fn change_to_line_end(&mut self) {
        self.delete_to_line_end();

        self.insert_mode();
    }

    fn delete_to_line_start(&mut self) {
        let text = self.text.slice(..);
        let index = text.line_idx_containing_byte(self.selection.cursor);
        let line_start = text.line_start_byte(index);

        self.text
            .remove(line_start.value()..=self.selection.cursor.value());

        self.set_cursor(line_start);
    }

    fn delete_to_first_non_blank(&mut self) {
        let text = self.text.slice(..);
        let line_index = text.line_idx_containing_byte(self.selection.cursor);
        let line = text.line_at(line_index);

        let offset: ByteIndex = line
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .map(|ch| ByteIndex::new(ch.len_utf8()))
            .sum();

        let start = text.line_start_byte(line_index) + offset;

        self.text
            .remove(start.value()..=self.selection.cursor.value());

        self.set_cursor(start);
    }

    fn delete_line(&mut self) {
        let text = self.text.slice(..);

        let index = text.line_idx_containing_byte(self.selection.cursor);
        let start = text.line_start_byte(index);
        let end = start + ByteIndex::new(text.line_at(index).len());

        self.text.remove(start.value()..end.value());
        self.set_cursor(start);
    }

    fn delete_whole_word(&mut self) {
        let current_ch = self.text.char(self.selection.cursor.value());

        let reversed_chars = self
            .text
            .slice(..=self.selection.cursor.value())
            .chars_at(self.selection.cursor.value())
            .reversed();

        let start = iter::once(current_ch)
            .chain(reversed_chars)
            .tuple_windows()
            .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                if right.is_word_start(left) {
                    ControlFlow::Break(index)
                } else {
                    ControlFlow::Continue(index.saturating_sub(left.ch().len_utf8()))
                }
            })
            .break_value()
            .unwrap_or(ByteIndex::new(0));

        let end = match self
            .text
            .slice(self.selection.cursor.value()..)
            .chars()
            .tuple_windows()
            .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                let next_index = index + left.ch().len_utf8();

                if left.is_word_end(right) {
                    // we started at the leftmost byte of the `left` char, and we want to
                    // delete it, and so the byte index we provide is the start of the next
                    // char, allowing us to use an exclusive range in the `remove` call below
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        };

        self.text.remove(start.value()..end.value());
        self.set_cursor(start);
    }

    fn delete_to_prev_word_start(&mut self) {
        let start = self
            .text
            .slice(..self.selection.cursor.value())
            .chars_at(self.selection.cursor.value())
            .reversed()
            .tuple_windows()
            .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                let next_index = index.saturating_sub(left.ch().len_utf8());

                if right.is_word_start(left) {
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            })
            .break_value()
            .unwrap_or(ByteIndex::new(0));

        self.text
            .remove(start.value()..self.selection.cursor.value());

        self.set_cursor(start);
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
    pub(crate) fn advance(self, grapheme: &Grapheme) -> Self {
        match *grapheme {
            Grapheme::LineBreak => {
                Self {
                    left: Columns::new(0),
                    top: self.top + Rows::new(1),
                }
            }
            Grapheme::Text(text) => {
                Self {
                    left: self.left + Columns::new(text.width()),
                    top: self.top,
                }
            }
        }
    }

    #[must_use]
    pub(crate) fn wrap(&self, max_width: Columns) -> (Self, WrapOutcome) {
        if *self.left() < max_width {
            (*self, WrapOutcome::NotWrapped)
        } else {
            (
                Self {
                    left: Columns::new(0),
                    top: self.top + Rows::new(1),
                },
                WrapOutcome::Wrapped,
            )
        }
    }

    #[must_use]
    fn col_offset(&self, gutter_width: Columns) -> Self {
        Self {
            left: self.left + gutter_width,
            top: self.top,
        }
    }

    #[must_use]
    fn offset(self, offset: Offset) -> Self {
        Self {
            left: offset.left + self.left,
            top: offset.top + self.top,
        }
    }

    /// Creates an iterator over each [`Position`] in the given area, assuming
    /// that `self` is at the top-left of the area.
    fn area_iter(&self, width: Columns, height: Rows) -> impl Iterator<Item = Self> {
        // TODO: iter::Step for Columns/Rows would make this cleaner but currently
        // unstable: https://github.com/rust-lang/rust/issues/42168
        (0..height.value()).flat_map(move |row| {
            (0..width.value()).map(move |col| {
                Self {
                    left: self.left + Columns::new(col),
                    top: self.top + Rows::new(row),
                }
            })
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct Offset {
    left: Columns,
    top: Rows,
}

impl Offset {
    const fn new(left: Columns, top: Rows) -> Self {
        Self { left, top }
    }
}

#[derive(Debug)]
pub(crate) enum WrapOutcome {
    Wrapped,
    NotWrapped,
}

#[derive(Debug)]
pub(crate) enum Grapheme<'grapheme> {
    LineBreak,
    Text(&'grapheme str),
}

impl Grapheme<'_> {
    pub(crate) const fn as_str(&self) -> &str {
        match *self {
            Grapheme::LineBreak => " ",
            Grapheme::Text(text) => text,
        }
    }
}

impl<'grapheme> From<&'grapheme str> for Grapheme<'grapheme> {
    fn from(value: &'grapheme str) -> Self {
        match value {
            "\n" | "\r\n" => Self::LineBreak,
            _ => Self::Text(value),
        }
    }
}

#[derive(Debug, Default)]
struct Selection {
    cursor: ByteIndex,
}

fn number_of_digits(value: usize) -> usize {
    (value.checked_ilog10().unwrap_or(0) + 1) as usize
}

#[derive(Debug)]
enum Mode {
    Normal,
    Insert,
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use crossterm::event::{
        KeyCode,
        KeyEvent,
    };

    use super::*;

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
    fn move_cursor_down_wrapped() {
        TestCase {
            initial_text: &"a".repeat(200),
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('j')],

            expected_text: &"a".repeat(200),
            expected_cursor: 77,
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
    fn move_cursor_up_wrapped() {
        TestCase {
            initial_text: &"a".repeat(200),
            initial_cursor: 77,
            expected_initial_visual_position: (3, 1),

            keys: vec![key_event!('k')],

            expected_text: &"a".repeat(200),
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn maintain_column_up_multiple_lines() {
        TestCase {
            initial_text: "Long line\nShort\nLong line",
            initial_cursor: 24,
            expected_initial_visual_position: (11, 2),

            keys: vec![key_event!('k'); 1],

            expected_text: "Long line\nShort\nLong line",
            expected_cursor: 15,
            expected_visual_position: (8, 1),
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
    fn scroll_down_wrapped() {
        let line = "a".repeat(200);
        let text = format!("{line}\n").repeat(100);

        TestCase {
            initial_text: &text,
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('G'); 1],

            expected_text: &text,
            expected_cursor: (200 * 99) + 99,
            expected_visual_position: (3, 0),
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

    #[test]
    fn delete_word_from_start() {
        TestCase {
            initial_text: "Hello world",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('d'), key_event!('w')],

            expected_text: "world",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn delete_word_from_middle() {
        TestCase {
            initial_text: "Hello world",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('d'), key_event!('w')],

            expected_text: "Heworld",
            expected_cursor: 2,
            expected_visual_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn delete_word_stop_at_hyphen() {
        TestCase {
            initial_text: "Hello-world",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('d'), key_event!('w')],

            expected_text: "-world",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn delete_word_leading_whitespace() {
        TestCase {
            initial_text: "      Hello-world",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('d'), key_event!('w')],

            expected_text: "Hello-world",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn change_word_from_start() {
        TestCase {
            initial_text: "Hello world",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![
                key_event!('c'),
                key_event!('w'),
                key_event!('h'),
                key_event!('i'),
            ],

            expected_text: "hiworld",
            expected_cursor: 2,
            expected_visual_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn change_word_from_middle() {
        TestCase {
            initial_text: "Hello world",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![
                key_event!('c'),
                key_event!('w'),
                key_event!('h'),
                key_event!('i'),
            ],

            expected_text: "Hehiworld",
            expected_cursor: 4,
            expected_visual_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn change_word_stop_at_hyphen() {
        TestCase {
            initial_text: "Hello-world",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![
                key_event!('c'),
                key_event!('w'),
                key_event!('h'),
                key_event!('i'),
            ],

            expected_text: "hi-world",
            expected_cursor: 2,
            expected_visual_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn change_word_leading_whitespace() {
        TestCase {
            initial_text: "      Hello-world",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![
                key_event!('c'),
                key_event!('w'),
                key_event!('h'),
                key_event!('i'),
            ],

            expected_text: "hiHello-world",
            expected_cursor: 2,
            expected_visual_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_line_end() {
        TestCase {
            initial_text: "Hello there!\nNext line",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('d'), key_event!('$')],

            expected_text: "He\nNext line",
            expected_cursor: 2,
            expected_visual_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn change_to_line_end() {
        TestCase {
            initial_text: "Hello there!\nNext line",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![
                key_event!('c'),
                key_event!('$'),
                key_event!('y'),
                key_event!('!'),
            ],

            expected_text: "Hey!\nNext line",
            expected_cursor: 4,
            expected_visual_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_line_start() {
        TestCase {
            initial_text: "Hello there!\n     Next line!",
            initial_cursor: 26,
            expected_initial_visual_position: (16, 1),

            keys: vec![key_event!('d'), key_event!('0')],

            expected_text: "Hello there!\n!",
            expected_cursor: 13,
            expected_visual_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn delete_to_line_first_non_blank() {
        TestCase {
            initial_text: "Hello there!\n     Next line!",
            initial_cursor: 26,
            expected_initial_visual_position: (16, 1),

            keys: vec![key_event!('d'), key_event!('^')],

            expected_text: "Hello there!\n     !",
            expected_cursor: 18,
            expected_visual_position: (8, 1),
        }
        .run();
    }

    #[test]
    fn delete_line() {
        TestCase {
            initial_text: "Hello there!\nNext line!",
            initial_cursor: 4,
            expected_initial_visual_position: (7, 0),

            keys: vec![key_event!('d'), key_event!('d')],

            expected_text: "Next line!",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn delete_whole_word() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 8,
            expected_initial_visual_position: (11, 0),

            keys: vec![key_event!('d'), key_event!('i'), key_event!('w')],

            expected_text: "Hello !!!",
            expected_cursor: 6,
            expected_visual_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn delete_whole_word_from_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('d'), key_event!('i'), key_event!('w')],

            expected_text: " there!!!",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn delete_whole_word_from_end() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 4,
            expected_initial_visual_position: (7, 0),

            keys: vec![key_event!('d'), key_event!('i'), key_event!('w')],

            expected_text: " there!!!",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_prev_word_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 6,
            expected_initial_visual_position: (9, 0),

            keys: vec![key_event!('d'), key_event!('b')],

            expected_text: "there!!!",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }
}
