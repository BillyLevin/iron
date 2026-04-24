use std::{
    cmp,
    fmt,
    fs::File,
    io::{
        self,
        BufReader,
    },
    iter,
    mem,
    ops::{
        ControlFlow,
        Range,
    },
    path::PathBuf,
    process::Command,
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
    RopeSlice,
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
    language::Language,
    terminal::EventOutcome,
    text::{
        ByteIndex,
        LeftChar,
        LineIndex,
        RightChar,
        RopeSliceExt as _,
        VisualLineInfo,
    },
    ui::{
        Columns,
        Dimensions,
        Position,
        Rectangle,
        Rows,
        Span,
    },
};

#[derive(Debug)]
pub(crate) struct Document {
    text: Rope,
    selection: Selection,

    normal_keymap: KeyMap,
    insert_keymap: KeyMap,
    visual_keymap: KeyMap,

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

    file_path: PathBuf,

    layout_info: LayoutInfo,

    /// The detected language of the document - this is based on the extension
    /// rather than the contents of the file.
    language: Language,
}

impl Document {
    pub(crate) fn new(file_path: PathBuf, dimensions: Dimensions) -> io::Result<Self> {
        Ok(Self {
            text: Rope::from_reader(BufReader::new(File::open(&file_path)?))?,
            selection: Selection::default(),
            normal_keymap: KeyMap::normal(),
            insert_keymap: KeyMap::insert(),
            visual_keymap: KeyMap::visual(),
            scroll_offset: LineIndex::default(),
            desired_cursor_column: None,
            mode: Mode::Normal,
            key_sequence: KeySequence::default(),
            language: Language::new(&file_path),
            file_path,
            layout_info: LayoutInfo::new(dimensions),
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
            if visual_grapheme.position().top() >= self.layout_info.text_rect.height() {
                break;
            }

            if visual_grapheme.position().left() == Columns::new(0) {
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
                translated_position.left() >= gutter_width,
                "filling in the gutter should've taken the position past the gutter"
            );

            let grapheme = visual_grapheme.grapheme();

            buffer[translated_position].set_content(grapheme.as_str());

            if matches!(self.mode, Mode::Visual)
                && self
                    .selection
                    .range(self.text.slice(..))
                    .contains(&(start_byte + visual_grapheme.byte_index()))
            {
                // TODO: theme!
                buffer[translated_position]
                    .set_foreground(Color::Black)
                    .set_background(Color::Grey);
            }

            if matches!(grapheme, Grapheme::LineBreak) {
                line_number += 1;
            }
        }

        self.render_status_line(buffer);
        self.render_key_hint(buffer);
    }

    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) -> EventOutcome {
        let keymap = match self.mode {
            Mode::Normal => &self.normal_keymap,
            Mode::Insert => &self.insert_keymap,
            Mode::Visual => &self.visual_keymap,
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

    pub(crate) fn resize(&mut self, dimensions: Dimensions) {
        self.layout_info = LayoutInfo::new(dimensions);
        self.recalculate_scroll();
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
            Mode::Visual => &self.visual_keymap,
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

        let container = &self.layout_info.text_rect;

        let hints_rectangle = container.bottom_right(Dimensions::new(
            cmp::min(container.width(), max_width),
            cmp::min(container.height(), max_height),
        ));

        buffer.fill_background(&hints_rectangle, Color::Rgb {
            r: 235,
            g: 219,
            b: 178,
        });

        for visual_grapheme in GraphemeLayoutIterator::new(
            hints.graphemes(true),
            hints_rectangle.width(),
            WrapBehavior::NoWrap,
        ) {
            if visual_grapheme.position().top() >= hints_rectangle.height() {
                break;
            }

            // wrapping has been turned off, and therefore we ignore all graphemes
            // that would overflow
            if visual_grapheme.position().left() >= hints_rectangle.width() {
                continue;
            }

            buffer[visual_grapheme.position().offset(hints_rectangle.offset())]
                .set_content(visual_grapheme.grapheme().as_str())
                .set_foreground(Color::White);
        }
    }

    fn render_status_line(&self, buffer: &mut Buffer) {
        buffer.fill_background(&self.layout_info.status_line_rect, Color::Rgb {
            r: 235,
            g: 219,
            b: 178,
        });

        let mut spans = vec![
            Span::new(format!(" {} ", self.mode))
                .with_fg(Color::Black)
                .with_bg(Color::Cyan),
        ];

        if let Some(file_name) = self
            .file_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!(" {name} "))
        {
            spans.push(Span::new(file_name).with_fg(Color::White));
        }

        if let Ok(jj_log_output) = Command::new("jj")
            .arg("log")
            .arg("--no-pager")
            .arg("--no-graph")
            .arg("--color=never")
            .arg("--revision=@")
            .arg(r#"--template=surround('"', '"', self.description().first_line())"#)
            .output()
            && jj_log_output.status.success()
            && let Ok(jj_description) = String::from_utf8(jj_log_output.stdout)
        {
            spans.push(Span::new(format!(" {jj_description} ")).with_fg(Color::White));
        }

        spans.push(Span::new(format!(" {} ", self.language)).with_fg(Color::White));

        buffer.render_spans(&spans, &self.layout_info.status_line_rect);
    }

    const fn set_cursor(&mut self, index: ByteIndex) {
        self.selection.cursor = index;
    }

    const fn set_anchor(&mut self, index: ByteIndex) {
        self.selection.anchor = index;
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
            Action::SwitchToVisualMode => self.visual_mode(),
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
            Action::AppendText => self.append_text(),
            Action::AppendTextLineEnd => self.append_text_line_end(),
            Action::MoveWordEnd => self.move_cursor_word_end(),
            Action::DeleteToWordEnd => self.delete_to_word_end(),
            Action::ChangeToLineStart => self.change_to_line_start(),
            Action::ChangeToLineFirstNonBlank => self.change_to_first_non_blank(),
            Action::ChangeLine => self.change_line(),
            Action::ChangeWholeWord => self.change_whole_word(),
            Action::ChangeToPrevWordStart => self.change_to_prev_word_start(),
            Action::ChangeToWordEnd => self.change_to_word_end(),
            Action::DeleteSelection => self.delete_selection(),
            Action::ChangeSelection => self.change_selection(),
            Action::ReverseSelection => self.reverse_selection(),
            Action::OpenLineBelow => self.open_new_line_below(),
            Action::OpenLineAbove => self.open_new_line_above(),
            Action::SelectCurrentWord => self.select_current_word(),
            Action::DeleteDown => self.delete_down(),
            Action::DeleteUp => self.delete_up(),
        }

        if action.should_reset_desired_column() {
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
        self.set_cursor(
            self.text
                .slice(..)
                .next_grapheme_position(self.selection.cursor),
        );
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

            let height = self.layout_info.text_rect.height();

            // downwards scroll
            if cursor.top() >= height {
                self.scroll_offset += LineIndex::from(cursor.top().value() - height.value() + 1);
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
            Mode::Normal | Mode::Visual => None,
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

    const fn visual_mode(&mut self) {
        self.selection.anchor = self.selection.cursor;
        self.mode = Mode::Visual;
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

        self.set_cursor(text.line_break(line_index).position);
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

        let offset = line
            .char_indices()
            .find_map(|(byte, ch)| (!ch.is_whitespace()).then(|| ByteIndex::new(byte)))
            .unwrap_or(ByteIndex::new(0));

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
        self.layout_info.dimensions.width() - self.gutter_width()
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

        self.text
            .remove(self.selection.cursor.value()..text.line_break(line_index).position.value());
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
            .slice(..self.selection.cursor.value())
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

    fn append_text(&mut self) {
        self.move_cursor_right();
        self.insert_mode();
    }

    fn append_text_line_end(&mut self) {
        let text = self.text.slice(..);

        let line_index = text.line_idx_containing_byte(self.selection.cursor);

        let line_break = text.line_break(line_index);

        self.set_cursor(line_break.position);

        // there is no linebreak, and so we need to make room to append text by adding
        // one. we will not shift the cursor, so the user will overwrite the
        // empty space when they enter text
        if !line_break.has_linebreak {
            // TODO: use the same linebreak style that the rest of the document uses, if
            // applicable
            self.text.insert_char(self.selection.cursor.value(), '\n');
        }

        self.insert_mode();
    }

    fn move_cursor_word_end(&mut self) {
        // we start searching at the next grapheme so that the cursor doesn't stay where
        // it is if it's already at the end of a word (in that case, we want to
        // go to the end of the **next** word)
        let search_start = self
            .text
            .slice(..)
            .next_grapheme_position(self.selection.cursor);

        let word_end = match self
            .text
            .slice(search_start.value()..)
            .chars()
            .tuple_windows()
            .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(search_start, |index, (left, right)| {
                if left.is_word_end(right) {
                    ControlFlow::Break(index)
                } else {
                    ControlFlow::Continue(index + left.ch().len_utf8())
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        };

        self.set_cursor(word_end);
    }

    fn delete_to_word_end(&mut self) {
        // we start searching at the next grapheme so that the cursor doesn't stay where
        // it is if it's already at the end of a word (in that case, we want to
        // go to the end of the **next** word)
        let search_start = self
            .text
            .slice(..)
            .next_grapheme_position(self.selection.cursor);

        let word_end = match self
            .text
            .slice(search_start.value()..)
            .chars()
            .tuple_windows()
            .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(search_start, |index, (left, right)| {
                let next_index = index + left.ch().len_utf8();

                if left.is_word_end(right) {
                    // we want to delete the whole character, which may be multiple bytes,
                    // and so we delete up to (but not including) the next character index
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        };

        self.text
            .remove(self.selection.cursor.value()..word_end.value());
    }

    fn change_to_line_start(&mut self) {
        self.delete_to_line_start();
        self.insert_mode();
    }

    fn change_to_first_non_blank(&mut self) {
        self.delete_to_first_non_blank();
        self.insert_mode();
    }

    fn change_line(&mut self) {
        let text = self.text.slice(..);

        let line_index = text.line_idx_containing_byte(self.selection.cursor);

        let line_start = text.line_start_byte(line_index);
        let line_break = text.line_break(line_index);

        self.text
            .remove(line_start.value()..line_break.position.value());

        self.set_cursor(line_start);

        // there is no linebreak, and so we need to make room to append text by adding
        // one. we will not shift the cursor, so the user will overwrite the
        // empty space when they enter text
        if !line_break.has_linebreak {
            // TODO: use the same linebreak style that the rest of the document uses, if
            // applicable
            self.text.insert_char(self.selection.cursor.value(), '\n');
        }

        self.insert_mode();
    }

    fn change_whole_word(&mut self) {
        self.delete_whole_word();
        self.insert_mode();
    }

    fn change_to_prev_word_start(&mut self) {
        self.delete_to_prev_word_start();
        self.insert_mode();
    }

    fn change_to_word_end(&mut self) {
        self.delete_to_word_end();
        self.insert_mode();
    }

    fn delete_selection(&mut self) {
        let delete_range = self.selection.range(self.text.slice(..));

        self.text
            .remove(delete_range.start.value()..delete_range.end.value());

        self.set_cursor(delete_range.start);

        self.normal_mode();
    }

    fn change_selection(&mut self) {
        self.delete_selection();
        self.insert_mode();
    }

    const fn reverse_selection(&mut self) {
        self.selection.reverse();
    }

    fn open_new_line_below(&mut self) {
        let text = self.text.slice(..);

        let line_index = text.line_idx_containing_byte(self.selection.cursor);
        let line_break = text.line_break(line_index);

        let to_insert = if line_break.has_linebreak {
            "\n"
        } else {
            "\n\n"
        };

        self.text.insert(line_break.position.value(), to_insert);
        self.set_cursor(self.text.slice(..).line_start_byte(line_index + 1));
        self.insert_mode();
    }

    fn open_new_line_above(&mut self) {
        let text = self.text.slice(..);

        let line_start = text.line_start_byte(text.line_idx_containing_byte(self.selection.cursor));

        self.text.insert_char(line_start.value(), '\n');
        self.set_cursor(line_start);
        self.insert_mode();
    }

    fn select_current_word(&mut self) {
        let current_ch = self.text.char(self.selection.cursor.value());

        let reversed_chars = self
            .text
            .slice(..self.selection.cursor.value())
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
                    ControlFlow::Break(index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        };

        self.set_anchor(start);
        self.set_cursor(end);
    }

    /// Deletes the current and next line.
    fn delete_down(&mut self) {
        let text = self.text.slice(..);

        let current_line = text.line_idx_containing_byte(self.selection.cursor);
        let next_line = current_line + 1;

        let start = text.line_start_byte(current_line);

        let Some(next_line_start) = text.get_line_start_byte(next_line) else {
            return;
        };

        let end = next_line_start + ByteIndex::new(text.line_at(next_line).len());

        self.text.remove(start.value()..end.value());
        self.set_cursor(start);
        self.move_cursor_first_non_blank();
    }

    fn delete_up(&mut self) {
        let text = self.text.slice(..);

        let current_line = text.line_idx_containing_byte(self.selection.cursor);
        let Some(prev_line) = current_line.checked_sub(1) else {
            return;
        };

        let start = text.line_start_byte(prev_line);
        let end =
            text.line_start_byte(current_line) + ByteIndex::new(text.line_at(current_line).len());

        self.text.remove(start.value()..end.value());
        self.set_cursor(match prev_line.checked_sub(1) {
            Some(line) => self.text.slice(..).line_start_byte(line),
            None => start,
        });
        self.move_cursor_first_non_blank();
    }
}

#[derive(Debug)]
pub(crate) struct LayoutInfo {
    dimensions: Dimensions,
    /// Size and position of the area that the file contents (including
    /// gutters) can be rendered into.
    text_rect: Rectangle,
    /// Size and position of the area that the status line can be rendered
    /// into.
    status_line_rect: Rectangle,
}

impl LayoutInfo {
    fn new(dimensions: Dimensions) -> Self {
        let (text_rect, status_line_rect) = Rectangle::from_dimensions(dimensions)
            .split_at(dimensions.height().saturating_sub(Rows::new(1)));

        Self {
            dimensions,
            text_rect,
            status_line_rect,
        }
    }
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
    /// The "start" of the selection. This is set to the current cursor position
    /// when entering [`Mode::Visual`] and then does not change while in that
    /// mode. If the document is not [`Mode::Visual`] then the value of this
    /// field is meaningless.
    anchor: ByteIndex,
    /// The "end" of the selection, which changes during movement. It is **not**
    /// restricted to appearing after [`Selection::anchor`]: it can overlap it,
    /// or appear before it in the document.
    cursor: ByteIndex,
}

impl Selection {
    /// Gets the range of bytes that the selection represents.
    fn range(&self, text: RopeSlice) -> Range<ByteIndex> {
        let start = cmp::min(self.cursor, self.anchor);
        // since each byte index represents the **start** of a grapheme, in order to get
        // all of the selected bytes, we extend the rightmost index to the start
        // of the **next** grapheme and represent it as a half-open range.
        let end = text.next_grapheme_position(cmp::max(self.cursor, self.anchor));

        start..end
    }

    const fn reverse(&mut self) {
        mem::swap(&mut self.anchor, &mut self.cursor);
    }
}

fn number_of_digits(value: usize) -> usize {
    (value.checked_ilog10().unwrap_or(0) + 1) as usize
}

#[derive(Debug)]
enum Mode {
    Normal,
    Insert,
    Visual,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match *self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
        })
    }
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
            Columns::from(expected.0),
            "{label} did not match"
        );
        assert_eq!(
            actual.top(),
            Rows::from(expected.1),
            "{label} did not match"
        );
    }

    fn doc(contents: &str) -> Document {
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        write!(temp_file, "{contents}").unwrap();

        Document::new(temp_file.path().to_path_buf(), TEST_DIMENSIONS).unwrap()
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
            initial_cursor: 22 * 5,
            expected_initial_visual_position: (3, 22),

            keys: vec![key_event!('j'); 1],

            expected_text: &text,
            expected_cursor: 23 * 5,
            // visual position stays the same because we scrolled down, keeping the
            // cursor on the final line
            expected_visual_position: (3, 22),
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
    fn move_cursor_first_non_blank_empty() {
        TestCase {
            initial_text: "\nHello",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![key_event!('^')],

            expected_text: "\nHello",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
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

    #[test]
    fn append_text() {
        TestCase {
            initial_text: "Hello",
            initial_cursor: 1,
            expected_initial_visual_position: (4, 0),

            keys: vec![key_event!('a'), key_event!('y'), key_event!('y')],

            expected_text: "Heyyllo",
            expected_cursor: 4,
            expected_visual_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn append_text_end_of_line_no_newline() {
        TestCase {
            initial_text: "Hello",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('A'), key_event!('!'), key_event!('!')],

            expected_text: "Hello!!\n",
            expected_cursor: 7,
            expected_visual_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn append_text_end_of_line_with_newline() {
        TestCase {
            initial_text: "Hello\n",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('A'), key_event!('!'), key_event!('!')],

            expected_text: "Hello!!\n",
            expected_cursor: 7,
            expected_visual_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_word_end() {
        TestCase {
            initial_text: "Hello__123: there",
            initial_cursor: 1,
            expected_initial_visual_position: (4, 0),

            keys: vec![key_event!('e')],

            expected_text: "Hello__123: there",
            expected_cursor: 9,
            expected_visual_position: (12, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_word_end() {
        TestCase {
            initial_text: "Hello there",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('d'), key_event!('e')],

            expected_text: "He there",
            expected_cursor: 2,
            expected_visual_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn change_to_line_start() {
        TestCase {
            initial_text: "Hello there!\n     Next line!",
            initial_cursor: 26,
            expected_initial_visual_position: (16, 1),

            keys: vec![
                key_event!('c'),
                key_event!('0'),
                key_event!('y'),
                key_event!('o'),
                key_event!('!'),
            ],

            expected_text: "Hello there!\nyo!!",
            expected_cursor: 16,
            expected_visual_position: (6, 1),
        }
        .run();
    }

    #[test]
    fn change_to_line_first_non_blank() {
        TestCase {
            initial_text: "Hello there!\n     Next line!",
            initial_cursor: 26,
            expected_initial_visual_position: (16, 1),

            keys: vec![
                key_event!('c'),
                key_event!('^'),
                key_event!('y'),
                key_event!('o'),
                key_event!('!'),
            ],

            expected_text: "Hello there!\n     yo!!",
            expected_cursor: 21,
            expected_visual_position: (11, 1),
        }
        .run();
    }

    #[test]
    fn change_line() {
        TestCase {
            initial_text: "Hello there!\nNext line!",
            initial_cursor: 4,
            expected_initial_visual_position: (7, 0),

            keys: vec![
                key_event!('c'),
                key_event!('c'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey\nNext line!",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn change_line_no_newline() {
        TestCase {
            initial_text: "Hello there!",
            initial_cursor: 4,
            expected_initial_visual_position: (7, 0),

            keys: vec![
                key_event!('c'),
                key_event!('c'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey\n",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn change_whole_word() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 8,
            expected_initial_visual_position: (11, 0),

            keys: vec![
                key_event!('c'),
                key_event!('i'),
                key_event!('w'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hello Hey!!!",
            expected_cursor: 9,
            expected_visual_position: (12, 0),
        }
        .run();
    }

    #[test]
    fn change_whole_word_from_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 0,
            expected_initial_visual_position: (3, 0),

            keys: vec![
                key_event!('c'),
                key_event!('i'),
                key_event!('w'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey there!!!",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn change_whole_word_from_end() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 4,
            expected_initial_visual_position: (7, 0),

            keys: vec![
                key_event!('c'),
                key_event!('i'),
                key_event!('w'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey there!!!",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn change_to_prev_word_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 8,
            expected_initial_visual_position: (11, 0),

            keys: vec![key_event!('c'), key_event!('b'), key_event!('H')],

            expected_text: "Hello Here!!!",
            expected_cursor: 7,
            expected_visual_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn change_to_prev_word_start_from_word_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 6,
            expected_initial_visual_position: (9, 0),

            keys: vec![
                key_event!('c'),
                key_event!('b'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
                key_event!(' '),
            ],

            expected_text: "Hey there!!!",
            expected_cursor: 4,
            expected_visual_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn change_to_word_end() {
        TestCase {
            initial_text: "Hello there",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('c'), key_event!('e'), key_event!('y')],

            expected_text: "Hey there",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn visual_mode_delete() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![
                key_event!('v'),
                key_event!('j'),
                key_event!('l'),
                key_event!('d'),
            ],

            expected_text: "Heher line!",
            expected_cursor: 2,
            expected_visual_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn visual_mode_change() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![
                key_event!('v'),
                key_event!('j'),
                key_event!('l'),
                key_event!('c'),
                key_event!('y'),
                key_event!(Enter),
                key_event!('a'),
                key_event!('n'),
                key_event!('o'),
                key_event!('t'),
            ],

            expected_text: "Hey\nanother line!",
            expected_cursor: 8,
            expected_visual_position: (7, 1),
        }
        .run();
    }

    #[test]
    fn reverse_selection() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![
                key_event!('v'),
                key_event!('j'),
                key_event!('l'),
                key_event!('o'),
            ],

            expected_text: "Hello there\nAnother line!",
            expected_cursor: 2,
            expected_visual_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn reverse_selection_twice() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![
                key_event!('v'),
                key_event!('j'),
                key_event!('l'),
                key_event!('o'),
                key_event!('o'),
            ],

            expected_text: "Hello there\nAnother line!",
            expected_cursor: 15,
            expected_visual_position: (6, 1),
        }
        .run();
    }

    #[test]
    fn open_new_line() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 15,
            expected_initial_visual_position: (6, 1),

            keys: vec![
                key_event!('o'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hello there\nAnother line!\nHey\n",
            expected_cursor: 29,
            expected_visual_position: (6, 2),
        }
        .run();
    }

    #[test]
    fn open_new_line_middle() {
        TestCase {
            initial_text: "Hello there\nAnother line!\nAgain a line :)",
            initial_cursor: 15,
            expected_initial_visual_position: (6, 1),

            keys: vec![
                key_event!('o'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hello there\nAnother line!\nHey\nAgain a line :)",
            expected_cursor: 29,
            expected_visual_position: (6, 2),
        }
        .run();
    }

    #[test]
    fn open_new_line_above() {
        TestCase {
            initial_text: "Hello there\nAnother line!\nAgain a line :)",
            initial_cursor: 15,
            expected_initial_visual_position: (6, 1),

            keys: vec![
                key_event!('O'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hello there\nHey\nAnother line!\nAgain a line :)",
            expected_cursor: 15,
            expected_visual_position: (6, 1),
        }
        .run();
    }

    #[test]
    fn open_new_line_above_top() {
        TestCase {
            initial_text: "Hello there\nAnother line!\nAgain a line :)",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![
                key_event!('O'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey\nHello there\nAnother line!\nAgain a line :)",
            expected_cursor: 3,
            expected_visual_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn select_current_word() {
        TestCase {
            initial_text: "Hello there",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![
                key_event!('v'),
                key_event!('i'),
                key_event!('w'),
                key_event!('d'),
            ],

            expected_text: " there",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn delete_down() {
        TestCase {
            initial_text: "Hello there\nAnother line\n     And another",
            initial_cursor: 2,
            expected_initial_visual_position: (5, 0),

            keys: vec![key_event!('d'), key_event!('j')],

            expected_text: "     And another",
            expected_cursor: 5,
            expected_visual_position: (8, 0),
        }
        .run();
    }

    #[test]
    fn delete_up() {
        TestCase {
            initial_text: "Hello there\nAnother line\nAnd another",
            initial_cursor: 27,
            expected_initial_visual_position: (5, 2),

            keys: vec![key_event!('d'), key_event!('k')],

            expected_text: "Hello there\n",
            expected_cursor: 0,
            expected_visual_position: (3, 0),
        }
        .run();
    }
}
