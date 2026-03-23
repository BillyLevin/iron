use std::ops::{self, Bound};

use ropey::{LineType, RopeSlice};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation as _};

pub(crate) trait RopeSliceExt<'rope> {
    fn line_idx_containing_byte(&self, byte: ByteIndex) -> LineIndex;

    fn line_start_byte(&self, line: LineIndex) -> ByteIndex;

    fn line_at(&self, line_index: LineIndex) -> RopeSlice<'rope>;

    /// Gets the byte index of the first byte of the previous grapheme from the given byte
    /// index.
    fn previous_grapheme_position(&self, from: ByteIndex) -> ByteIndex;

    fn graphemes(
        &self,
        range: impl ops::RangeBounds<ByteIndex>,
    ) -> impl Iterator<Item = &'rope str>;

    fn line_count(&self) -> usize;

    fn last_line_idx(&self) -> LineIndex;

    fn is_whitespace(&self) -> bool;
}

impl<'rope> RopeSliceExt<'rope> for RopeSlice<'rope> {
    fn line_idx_containing_byte(&self, byte: ByteIndex) -> LineIndex {
        LineIndex::from(self.byte_to_line_idx(byte.value(), ropey::LineType::LF_CR))
    }

    fn line_start_byte(&self, line: LineIndex) -> ByteIndex {
        ByteIndex::from(self.line_to_byte_idx(line.value(), LineType::LF_CR))
    }

    fn line_at(&self, line_index: LineIndex) -> Self {
        self.line(line_index.value(), LineType::LF_CR)
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

    fn graphemes(
        &self,
        range: impl ops::RangeBounds<ByteIndex>,
    ) -> impl Iterator<Item = &'rope str> {
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

        self.slice((start, end))
            .chunks()
            .flat_map(|chunk| chunk.graphemes(true))
    }

    fn line_count(&self) -> usize {
        // NOTE: we are doing this because of:
        // https://docs.rs/ropey/2.0.0-beta.1/ropey/#a-note-about-line-breaks. if the file
        // has a trailing line break, ropey counts that in the line count, but we want to
        // act as if it doesn't exist. so, if the last line is empty, we'll lower the line
        // count
        let lines = self.len_lines(LineType::LF_CR);

        let last_line = self.line(lines.saturating_sub(1), LineType::LF_CR);

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
pub(crate) struct ByteIndex(usize);

impl ByteIndex {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> usize {
        self.0
    }

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
)]
pub(crate) struct LineIndex(usize);

impl LineIndex {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> usize {
        self.0
    }

    pub(crate) const fn saturating_sub(self, rhs: usize) -> Self {
        Self(self.0.saturating_sub(rhs))
    }
}

impl ops::Add<usize> for LineIndex {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}
