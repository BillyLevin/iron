use std::{
    cmp,
    iter,
    num::NonZeroUsize,
    ops::{
        ControlFlow,
        Range,
    },
};

use itertools::Itertools as _;
use ropey::{
    LineType,
    RopeSlice,
};

use crate::{
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
        NonZeroColumns,
    },
};

#[derive(Debug, Default)]
pub(crate) struct CursorState {
    selection: Selection,
    /// When navigating vertically, the cursor will be moved to the left if the
    /// next line is narrower than the current. We use this field to track
    /// where the cursor would ideally be so that we can move it there if
    /// the line is wide enough.
    ///
    /// The value is relative to the start of the **text rectangle** that the
    /// cursor belongs to.
    desired_cursor_column: Option<Columns>,
}

impl CursorState {
    pub(crate) const fn cursor(&self) -> ByteIndex {
        self.selection.cursor
    }

    pub(crate) const fn selection(&self) -> Selection {
        self.selection
    }

    pub(crate) const fn desired_cursor_column(&self) -> Option<Columns> {
        self.desired_cursor_column
    }

    pub(crate) const fn set_cursor(&mut self, index: ByteIndex) {
        self.selection.cursor = index;
    }

    pub(crate) const fn set_anchor(&mut self, index: ByteIndex) {
        self.selection.anchor = index;
    }

    pub(crate) const fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
    }

    pub(crate) const fn clear_desired_column(&mut self) {
        self.desired_cursor_column = None;
    }

    pub(crate) const fn set_desired_cursor_column(&mut self, column: Columns) {
        self.desired_cursor_column = Some(column);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Selection {
    /// The "start" of the selection. This is set to the current cursor position
    /// when entering [`Mode::Visual`](crate::document::Mode::Visual) and then
    /// does not change while in that mode. If the document is not
    /// [`Mode::Visual`](crate::document::Mode::Visual) then the value of this
    /// field is meaningless.
    anchor: ByteIndex,
    /// The "end" of the selection, which changes during movement. It is **not**
    /// restricted to appearing after [`Selection::anchor`]: it can overlap it,
    /// or appear before it in the document.
    cursor: ByteIndex,
}

impl Selection {
    /// Gets the range of bytes that the selection represents.
    pub(crate) fn range(&self, text: RopeSlice) -> Range<ByteIndex> {
        let start = cmp::min(self.cursor, self.anchor);
        let end = cmp::max(self.cursor, self.anchor);

        // since each byte index represents the **start** of a grapheme, in
        // order to get all of the selected bytes, we extend the
        // rightmost index to the start of the **next** grapheme and
        // represent it as a half-open range.
        text.inclusive_to_exclusive_range(start..=end)
    }

    /// Creates a new [`Selection`] with the cursor set to the given position.
    #[must_use]
    pub(crate) const fn with_cursor(self, cursor: ByteIndex) -> Self {
        Self { cursor, ..self }
    }

    /// Creates a new [`Selection`] with the anchor set to the given position.
    #[must_use]
    pub(crate) const fn with_anchor(self, anchor: ByteIndex) -> Self {
        Self { anchor, ..self }
    }

    #[must_use]
    pub(crate) const fn reversed(self) -> Self {
        Self {
            anchor: self.cursor,
            cursor: self.anchor,
        }
    }
}

pub(crate) fn move_cursor_down(
    text: RopeSlice<'_>,
    selection: Selection,
    target_column: Columns,
    text_width: NonZeroColumns,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _i| {
        VisualLineInfo::new(text, text.line_idx_containing_byte(cursor), text_width)
            .next_at_column(cursor, target_column)
            .unwrap_or(cursor)
    });

    selection.with_cursor(cursor)
}

pub(crate) fn move_cursor_up(
    text: RopeSlice<'_>,
    selection: Selection,
    target_column: Columns,
    text_width: NonZeroColumns,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _i| {
        VisualLineInfo::new(text, text.line_idx_containing_byte(cursor), text_width)
            .prev_at_column(cursor, target_column)
            .unwrap_or(cursor)
    });

    selection.with_cursor(cursor)
}

pub(crate) fn move_cursor_right(
    text: RopeSlice<'_>,
    selection: Selection,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _i| {
        text.next_grapheme_position(cursor)
    });

    selection.with_cursor(cursor)
}

pub(crate) fn move_cursor_left(
    text: RopeSlice<'_>,
    selection: Selection,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _i| {
        text.previous_grapheme_position(cursor)
    });

    selection.with_cursor(cursor)
}

pub(crate) fn move_cursor_next_word_start(
    text: RopeSlice<'_>,
    selection: Selection,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _i| {
        match text
            .slice(cursor.value()..)
            .chars()
            .tuple_windows()
            .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(cursor, |index, (left, right)| {
                let next_index = index + left.ch().len_utf8();

                if right.is_word_start(left) {
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        }
    });

    selection.with_cursor(cursor)
}

pub(crate) fn move_cursor_prev_word_start(
    text: RopeSlice<'_>,
    selection: Selection,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _i| {
        text.slice(..cursor.value())
            .chars_at(cursor.value())
            .reversed()
            .tuple_windows()
            .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(cursor, |index, (left, right)| {
                let next_index = index.saturating_sub(right.ch().len_utf8());

                if right.is_word_start(left) {
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            })
            .break_value()
            .unwrap_or_default()
    });

    selection.with_cursor(cursor)
}

/// Moves to the cursor to the last non-linebreak grapheme on the current
/// line.
pub(crate) fn move_cursor_line_end(text: RopeSlice<'_>, selection: Selection) -> Selection {
    let line_index = text.line_idx_containing_byte(selection.cursor);
    let cursor = cmp::max(
        text.line_start_byte(line_index),
        text.previous_grapheme_position(text.line_break(line_index).position),
    );

    selection.with_cursor(cursor)
}

pub(crate) fn move_cursor_line_start(text: RopeSlice<'_>, selection: Selection) -> Selection {
    let cursor = text.line_start_byte(text.line_idx_containing_byte(selection.cursor));

    selection.with_cursor(cursor)
}

/// Moves the cursor to the first non-whitespace character on the current
/// line.
pub(crate) fn move_cursor_first_non_blank(text: RopeSlice<'_>, selection: Selection) -> Selection {
    let line_index = text.line_idx_containing_byte(selection.cursor);
    let line = text.line_at(line_index);
    let cursor = text.line_start_byte(line_index) + line.first_non_blank_offset();

    selection.with_cursor(cursor)
}

pub(crate) fn move_cursor_next_paragraph(
    text: RopeSlice<'_>,
    selection: Selection,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _index| {
        let line_index = text.line_idx_containing_byte(cursor);

        let line_offset = text
            .lines_at(line_index.value(), LineType::LF_CR)
            .enumerate()
            .skip_while(|&(_i, line)| line.is_whitespace())
            .find(|&(_i, line)| line.is_whitespace())
            .map(|(i, _line)| i);

        match line_offset {
            Some(offset) => text.line_start_byte(line_index + offset),
            None => ByteIndex::new(text.len()),
        }
    });

    selection.with_cursor(cursor)
}

pub(crate) fn move_cursor_prev_paragraph(
    text: RopeSlice<'_>,
    selection: Selection,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _index| {
        let line_index = text.line_idx_containing_byte(cursor);

        let line_offset = text
            // NOTE: +1 because when we use `reversed()`, the iterator does not consume the
            // line at the provided index
            .lines_at(line_index.value() + 1, LineType::LF_CR)
            .reversed()
            .enumerate()
            .skip_while(|&(_i, line)| line.is_whitespace())
            .find(|&(_i, line)| line.is_whitespace())
            .map(|(i, _line)| i);

        match line_offset {
            Some(offset) => text.line_start_byte(line_index.saturating_sub(offset)),
            None => ByteIndex::new(0),
        }
    });

    selection.with_cursor(cursor)
}

pub(crate) fn go_to_last_line(text: RopeSlice<'_>, selection: Selection) -> Selection {
    let cursor = text.line_start_byte(text.last_line_idx());

    selection.with_cursor(cursor)
}

const fn go_to_first_line(selection: Selection) -> Selection {
    selection.with_cursor(ByteIndex::new(0))
}

pub(crate) fn go_to_nth_or_first_line(
    text: RopeSlice<'_>,
    selection: Selection,
    line_number: Option<NonZeroUsize>,
) -> Selection {
    if let Some(line) = line_number {
        go_to_line_index(
            text,
            selection,
            cmp::min(LineIndex::new(line.get() - 1), text.last_line_idx()),
        )
    } else {
        go_to_first_line(selection)
    }
}

pub(crate) fn go_to_nth_or_last_line(
    text: RopeSlice<'_>,
    selection: Selection,
    line_number: Option<NonZeroUsize>,
) -> Selection {
    if let Some(line) = line_number {
        go_to_line_index(
            text,
            selection,
            cmp::min(LineIndex::new(line.get() - 1), text.last_line_idx()),
        )
    } else {
        go_to_last_line(text, selection)
    }
}

fn go_to_line_index(text: RopeSlice<'_>, selection: Selection, line: LineIndex) -> Selection {
    selection.with_cursor(text.line_start_byte(line))
}

pub(crate) fn move_cursor_word_end(
    text: RopeSlice<'_>,
    selection: Selection,
    count: usize,
) -> Selection {
    let cursor = (0..count).fold(selection.cursor, |cursor, _i| {
        // we start searching at the next grapheme so that the cursor
        // doesn't stay where it is if it's already at the end
        // of a word (in that case, we want to go to the end of
        // the **next** word)
        let search_start = text.next_grapheme_position(cursor);

        match text
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
        }
    });

    selection.with_cursor(cursor)
}

pub(crate) const fn reverse_selection(selection: Selection) -> Selection {
    selection.reversed()
}

pub(crate) fn select_current_word(text: RopeSlice<'_>, selection: Selection) -> Selection {
    let cursor_value = selection.cursor.value();
    let current_ch = text.char(cursor_value);

    let reversed_chars = text.slice(..cursor_value).chars_at(cursor_value).reversed();

    let start = iter::once(current_ch)
        .chain(reversed_chars)
        .tuple_windows()
        .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
        .try_fold(selection.cursor, |index, (left, right)| {
            if right.is_word_start(left) {
                ControlFlow::Break(index)
            } else {
                ControlFlow::Continue(index.saturating_sub(left.ch().len_utf8()))
            }
        })
        .break_value()
        .unwrap_or(ByteIndex::new(0));

    let end = match text
        .slice(cursor_value..)
        .chars()
        .tuple_windows()
        .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
        .try_fold(selection.cursor, |index, (left, right)| {
            let next_index = index + left.ch().len_utf8();

            if left.is_word_end(right) {
                ControlFlow::Break(index)
            } else {
                ControlFlow::Continue(next_index)
            }
        }) {
        ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
    };

    Selection::default().with_anchor(start).with_cursor(end)
}

pub(crate) fn go_to_pair_match(text: RopeSlice<'_>, selection: Selection) -> Selection {
    let cursor = selection.cursor.value();

    let Some(pair) = text.get_byte(cursor).and_then(PairItem::new) else {
        return selection;
    };

    let current = pair.as_byte();
    let opposite = pair.opposite_as_byte();

    assert_ne!(current, opposite, "current should never be opposite!");

    let bytes = match pair.position {
        PairPosition::Start => text.bytes_at(cursor + 1),
        PairPosition::End => text.bytes_at(cursor).reversed(),
    };

    let Some(offset) = bytes
        .enumerate()
        .try_fold(1_u64, |depth, (offset, byte)| {
            let next_depth = if byte == current {
                depth + 1
            } else if byte == opposite {
                depth - 1
            } else {
                depth
            };

            if next_depth == 0 {
                ControlFlow::Break(ByteIndex::new(offset + 1))
            } else {
                ControlFlow::Continue(next_depth)
            }
        })
        .break_value()
    else {
        return selection;
    };

    selection.with_cursor(match pair.position {
        PairPosition::Start => selection.cursor + offset,
        PairPosition::End => selection.cursor - offset,
    })
}

#[derive(Debug)]
struct PairItem {
    kind: PairKind,
    position: PairPosition,
}

impl PairItem {
    const fn new(value: u8) -> Option<Self> {
        match value {
            b'(' => {
                Some(Self {
                    kind: PairKind::Paren,
                    position: PairPosition::Start,
                })
            }
            b')' => {
                Some(Self {
                    kind: PairKind::Paren,
                    position: PairPosition::End,
                })
            }
            b'{' => {
                Some(Self {
                    kind: PairKind::Brace,
                    position: PairPosition::Start,
                })
            }
            b'}' => {
                Some(Self {
                    kind: PairKind::Brace,
                    position: PairPosition::End,
                })
            }
            b'[' => {
                Some(Self {
                    kind: PairKind::Bracket,
                    position: PairPosition::Start,
                })
            }
            b']' => {
                Some(Self {
                    kind: PairKind::Bracket,
                    position: PairPosition::End,
                })
            }
            _ => None,
        }
    }

    const fn as_byte(&self) -> u8 {
        match self.position {
            PairPosition::Start => self.kind.start_byte(),
            PairPosition::End => self.kind.end_byte(),
        }
    }

    const fn opposite_as_byte(&self) -> u8 {
        match self.position {
            PairPosition::Start => self.kind.end_byte(),
            PairPosition::End => self.kind.start_byte(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PairKind {
    /// `(` or `)`.
    Paren,
    /// `{` or `}`.
    Brace,
    /// `[` or `]`.
    Bracket,
}

impl PairKind {
    const fn start_byte(self) -> u8 {
        match self {
            Self::Paren => b'(',
            Self::Brace => b'{',
            Self::Bracket => b'[',
        }
    }

    const fn end_byte(self) -> u8 {
        match self {
            Self::Paren => b')',
            Self::Brace => b'}',
            Self::Bracket => b']',
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PairPosition {
    Start,
    End,
}
