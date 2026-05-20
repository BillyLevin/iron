use std::ops;

use ropey::{
    LineType,
    Rope,
    RopeSlice,
};
use unicode_segmentation::{
    GraphemeCursor,
    GraphemeIncomplete,
    UnicodeSegmentation as _,
};

use crate::{
    document::Grapheme,
    grapheme_layout::{
        GraphemeLayoutIterator,
        WrapBehavior,
    },
    ui::Columns,
};

const LINE_TYPE: LineType = LineType::LF_CR;

pub(crate) trait RopeSliceExt<'rope> {
    fn line_idx_containing_byte(&self, byte: ByteIndex) -> LineIndex;

    /// Gets the [`ByteIndex`] of the start of the line at the given
    /// [`LineIndex`]. NOTE: this does NOT allow a one-past-the-end line
    /// index like [`RopeSlice::line_to_byte_idx`] does.
    ///
    /// # Panics
    ///
    /// Panics if `line > self.len_lines()`.
    fn line_start_byte(&self, line: LineIndex) -> ByteIndex;

    /// Non-panicking version of [`RopeSliceExt::line_start_byte`].
    fn get_line_start_byte(&self, line: LineIndex) -> Option<ByteIndex>;

    fn line_at(&self, line_index: LineIndex) -> RopeSlice<'rope>;

    /// Gets the byte index of the first byte of the previous grapheme from the
    /// given byte index.
    fn previous_grapheme_position(&self, from: ByteIndex) -> ByteIndex;

    /// Gets the byte index of the first byte of the next grapheme from the
    /// given byte index.
    fn next_grapheme_position(&self, from: ByteIndex) -> ByteIndex;

    fn graphemes(&self) -> impl Iterator<Item = &'rope str>;

    fn line_count(&self) -> usize;

    fn last_line_idx(&self) -> LineIndex;

    fn is_whitespace(&self) -> bool;

    /// Gets the byte index of either:
    ///   * the byte index corresponding to the given column; or:
    ///   * the byte index corresponding to the last column (if the text is
    ///     narrower than the given column)
    ///
    /// NOTE: the returned byte index is relative to the start of the text
    /// slice.
    fn byte_at_column(&self, target_column: Columns) -> ByteIndex;

    /// Gets the line break information for the line at the given `line_index`.
    fn line_break(&self, line_index: LineIndex) -> LineBreakOutcome;
}

impl<'rope> RopeSliceExt<'rope> for RopeSlice<'rope> {
    fn line_idx_containing_byte(&self, byte: ByteIndex) -> LineIndex {
        LineIndex::from(self.byte_to_line_idx(byte.value(), LINE_TYPE))
    }

    fn line_start_byte(&self, line: LineIndex) -> ByteIndex {
        self.get_line_start_byte(line).unwrap()
    }

    fn get_line_start_byte(&self, line: LineIndex) -> Option<ByteIndex> {
        (line.value() <= self.len_lines(LINE_TYPE))
            .then(|| ByteIndex::new(self.line_to_byte_idx(line.value(), LINE_TYPE)))
    }

    fn line_at(&self, line_index: LineIndex) -> Self {
        self.line(line_index.value(), LINE_TYPE)
    }

    fn previous_grapheme_position(&self, from: ByteIndex) -> ByteIndex {
        let text_slice = self.slice(..from.value());

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
                        "there should be a chunk that ends at `offset`, and therefore it must be \
                         non-zero"
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

    fn next_grapheme_position(&self, from: ByteIndex) -> ByteIndex {
        from + self
            .slice(from.value()..)
            .graphemes()
            .next()
            .map_or(0, str::len)
    }

    fn graphemes(&self) -> impl Iterator<Item = &'rope str> {
        self.chunks().flat_map(|chunk| chunk.graphemes(true))
    }

    fn line_count(&self) -> usize {
        // NOTE: we are doing this because of:
        // https://docs.rs/ropey/2.0.0-beta.1/ropey/#a-note-about-line-breaks. if the file
        // has a trailing line break, ropey counts that in the line count, but we want
        // to act as if it doesn't exist. so, if the last line is empty, we'll
        // lower the line count
        let lines = self.len_lines(LINE_TYPE);

        let last_line = self.line(lines.saturating_sub(1), LINE_TYPE);

        if last_line.len() == 0 {
            lines.saturating_sub(1)
        } else {
            lines
        }
    }

    fn last_line_idx(&self) -> LineIndex {
        LineIndex::new(self.line_count().saturating_sub(1))
    }

    fn is_whitespace(&self) -> bool {
        self.chars().all(char::is_whitespace)
    }

    fn byte_at_column(&self, target_column: Columns) -> ByteIndex {
        let mut column = Columns::new(0);
        let mut byte_index = ByteIndex::new(0);

        for grapheme in self.graphemes().map(Grapheme::from) {
            if column >= target_column {
                break;
            }

            match grapheme {
                Grapheme::LineBreak => break,
                Grapheme::Tab => {
                    column += TAB_VISUAL_WIDTH;
                    byte_index += '\t'.len_utf8();
                }
                Grapheme::Text(raw) => {
                    column += text_width(raw);
                    byte_index += raw.len();
                }
            }
        }

        byte_index
    }

    fn line_break(&self, line_index: LineIndex) -> LineBreakOutcome {
        let line = self.line_at(line_index);
        let line_start = self.line_start_byte(line_index);

        let (offset, has_linebreak) = match line.trailing_line_break_idx(LINE_TYPE) {
            Some(offset) => (ByteIndex::new(offset), true),
            None => (ByteIndex::new(line.len()), false),
        };

        LineBreakOutcome {
            position: line_start + offset,
            has_linebreak,
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
    derive_more::AddAssign,
    derive_more::Sum,
)]
pub(crate) struct ByteIndex(usize);

impl ByteIndex {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> usize {
        self.0
    }

    #[must_use = "`saturating_sub` does not mutate the current value, but returns a new value"]
    pub(crate) const fn saturating_sub(self, rhs: usize) -> Self {
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
    derive_more::Display,
)]
pub(crate) struct LineIndex(usize);

impl LineIndex {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> usize {
        self.0
    }

    #[must_use = "`saturating_sub` does not mutate the current value, but returns a new value"]
    pub(crate) const fn saturating_sub(self, rhs: usize) -> Self {
        Self(self.0.saturating_sub(rhs))
    }

    #[must_use = "`checked_sub` does not mutate the current value, but returns an `Option`"]
    pub(crate) fn checked_sub(self, rhs: usize) -> Option<Self> {
        Some(Self(self.0.checked_sub(rhs)?))
    }

    pub(crate) const fn abs_diff(self, other: Self) -> Self {
        Self(self.0.abs_diff(other.0))
    }
}

impl ops::Add<usize> for LineIndex {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

#[derive(Debug)]
pub(crate) struct VisualLineInfo<'text> {
    text: &'text Rope,
    line_index: LineIndex,
    /// Byte indices of the start of the **visual** lines produced by the text
    /// line. These indices are relative to the start of the text slice.
    ///
    /// A line can have multiple visual lines due to text wrapping.
    visual_line_starts: Vec<ByteIndex>,
    /// The maximum visual width of a line before the text needs to wrap.
    max_width: Columns,
}

impl<'text> VisualLineInfo<'text> {
    pub(crate) fn new(text: &'text Rope, line_index: LineIndex, max_width: Columns) -> Self {
        let mut visual_line_starts = Vec::new();

        let text_slice = text.slice(..);

        let start = text_slice.line_start_byte(line_index);

        for grapheme in GraphemeLayoutIterator::new(
            text_slice.line_at(line_index).graphemes(),
            max_width,
            WrapBehavior::Wrap,
        ) {
            if grapheme.position().left() == Columns::new(0) {
                visual_line_starts.push(start + grapheme.byte_index());
            }
        }

        Self {
            text,
            line_index,
            visual_line_starts,
            max_width,
        }
    }

    /// Attempts to get the byte index of the target column (or lower if the
    /// line isn't wide enough) on the **previous visual line** from the
    /// given `byte_index`. If the visual line doesn't exist, simply returns
    /// `None`.
    pub(crate) fn prev_at_column(
        &self,
        byte_index: ByteIndex,
        column: Columns,
    ) -> Option<ByteIndex> {
        let start = self.prev_visual_line(byte_index)?;
        Some(start + self.text.slice(start.value()..).byte_at_column(column))
    }

    /// Attempts to get the byte index of the target column (or lower if the
    /// line isn't wide enough) on the **next visual line** from the given
    /// `byte_index`. If the visual line doesn't exist, simply returns
    /// `None`.
    pub(crate) fn next_at_column(
        &self,
        byte_index: ByteIndex,
        column: Columns,
    ) -> Option<ByteIndex> {
        let start = self.next_visual_line(byte_index)?;
        Some(start + self.text.slice(start.value()..).byte_at_column(column))
    }

    /// Gets the **start byte** of the **visual** line above the visual line
    /// that contains the given byte index. This could either be part of the
    /// current text line (if it's wrapped) or the previous text line (if it
    /// exists).
    fn prev_visual_line(&self, byte_index: ByteIndex) -> Option<ByteIndex> {
        debug_assert!(
            self.visual_line_starts.is_sorted(),
            "`partition_point` will only work properly if the indices are sorted"
        );

        let partition = self
            .visual_line_starts
            .partition_point(|start_index| *start_index <= byte_index);

        // the partition logic actually gets the index of the **next** visual line (if
        // it exists), and so we have to subtract two from the result. this
        // works even if there isn't a visual line below the current one, since
        // `partition_point` returns the length of `visual_line_starts` if the
        // predicate matches for all elements
        if partition >= 2 {
            self.visual_line_starts.get(partition - 2).copied()
        } else {
            Self::new(self.text, self.line_index.checked_sub(1)?, self.max_width)
                .bottom_visual_line()
        }
    }

    /// Gets the **start byte** of the **visual** line below the visual line
    /// that contains the given byte index. This could either be part of the
    /// current text line (if it's wrapped) or the next text line (if it
    /// exists).
    fn next_visual_line(&self, byte_index: ByteIndex) -> Option<ByteIndex> {
        debug_assert!(
            self.visual_line_starts.is_sorted(),
            "`partition_point` will only work properly if the indices are sorted"
        );

        let partition = self
            .visual_line_starts
            .partition_point(|start_index| *start_index <= byte_index);

        self.visual_line_starts.get(partition).copied().or_else(|| {
            if self.text.slice(..).last_line_idx() == self.line_index {
                None
            } else {
                Self::new(self.text, self.line_index + 1, self.max_width).top_visual_line()
            }
        })
    }

    /// Gets the **start byte** of the bottom **visual** line of the text line.
    fn bottom_visual_line(&self) -> Option<ByteIndex> {
        self.visual_line_starts.last().copied()
    }

    /// Gets the **start byte** of the top **visual** line of the text line.
    fn top_visual_line(&self) -> Option<ByteIndex> {
        self.visual_line_starts.first().copied()
    }
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

impl From<LeftChar> for WordBoundaryKind {
    fn from(left: LeftChar) -> Self {
        Self::from(left.0)
    }
}

impl From<RightChar> for WordBoundaryKind {
    fn from(right: RightChar) -> Self {
        Self::from(right.0)
    }
}

/// A semantic wrapper around a `char` - only meaningful when paired with a
/// corresponding [`RightChar`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct LeftChar(char);

impl LeftChar {
    pub(crate) const fn new(ch: char) -> Self {
        Self(ch)
    }

    pub(crate) const fn ch(self) -> char {
        self.0
    }

    pub(crate) fn is_word_end(self, right: RightChar) -> bool {
        let left_kind = WordBoundaryKind::from(self);
        let right_kind = WordBoundaryKind::from(right);

        left_kind != right_kind && left_kind != WordBoundaryKind::Whitespace
    }
}

/// A semantic wrapper around a `char` - only meaningful when paired with a
/// corresponding [`LeftChar`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct RightChar(char);

impl RightChar {
    pub(crate) const fn new(ch: char) -> Self {
        Self(ch)
    }

    pub(crate) fn is_word_start(self, left: LeftChar) -> bool {
        let left_kind = WordBoundaryKind::from(left);
        let right_kind = WordBoundaryKind::from(self);

        left_kind != right_kind && right_kind != WordBoundaryKind::Whitespace
    }
}

#[derive(Debug)]
pub(crate) struct LineBreakOutcome {
    /// The byte index of the slice that the line break appears (or should
    /// appear, if it doesn't exist).
    pub(crate) position: ByteIndex,
    /// The last line of a file may not have a line break, and this often
    /// requires special handling.
    pub(crate) has_linebreak: bool,
}

pub(crate) const TAB_VISUAL_WIDTH: Columns = Columns::new(4);

/// Wrapper around [`unicode_width::UnicodeWidthStr`] that treats tabs as the
/// width that we display them as.
pub(crate) fn text_width(text: &str) -> Columns {
    text.graphemes(true)
        .map(|grapheme| {
            match grapheme {
                "\t" => TAB_VISUAL_WIDTH,
                g => Columns::new(unicode_width::UnicodeWidthStr::width(g)),
            }
        })
        .sum()
}
