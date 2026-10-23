use std::cmp;

use ropey::RopeSlice;

use crate::{
    cursor::visual_cursor_position,
    text::{
        ByteIndex,
        LineIndex,
        RopeSliceExt as _,
    },
    ui::{
        NonZeroColumns,
        Position,
        Rectangle,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextArea {
    rectangle: Rectangle,
    /// Number of lines from the top of the file that the text should start
    /// from.
    scroll_offset: LineIndex,
}

impl TextArea {
    pub(crate) const fn new(rectangle: Rectangle) -> Self {
        Self {
            rectangle,
            scroll_offset: LineIndex::new(0),
        }
    }

    pub(crate) const fn area(&self) -> &Rectangle {
        &self.rectangle
    }

    pub(crate) const fn scroll_offset(&self) -> LineIndex {
        self.scroll_offset
    }

    pub(crate) const fn set_scroll_offset(&mut self, offset: LineIndex) {
        self.scroll_offset = offset;
    }

    pub(crate) const fn into_area(self) -> Rectangle {
        self.rectangle
    }

    pub(crate) const fn with_area(&self, rectangle: Rectangle) -> Self {
        Self {
            rectangle,
            scroll_offset: self.scroll_offset,
        }
    }

    pub(crate) fn recalculate_scroll(&mut self, text: RopeSlice<'_>, cursor_index: ByteIndex) {
        let cursor_line = text.line_idx_containing_byte(cursor_index);

        let height = self.rectangle.height();

        let scroll_offset = if cursor_line < self.scroll_offset {
            // upwards scroll
            cursor_line
        } else {
            // move the scroll offset further down so that we don't have to scan
            // as much of the rope when calculating the cursor
            // position. `scroll_offset` will still be
            // off the screen, so it won't impact the calculation
            let scroll_offset = if cursor_line > self.scroll_offset + height.value() {
                cursor_line.saturating_sub(height.value())
            } else {
                self.scroll_offset
            };

            let cursor = self.relative_cursor_position(text, cursor_index, scroll_offset);

            // downwards scroll
            if cursor.top() >= height {
                scroll_offset + (cursor.top().value() - height.value() + 1)
            } else {
                scroll_offset
            }
        };

        self.scroll_offset = cmp::min(cursor_line, scroll_offset);
    }

    pub(crate) fn visual_cursor_position(
        &self,
        text: RopeSlice<'_>,
        cursor_index: ByteIndex,
    ) -> Position {
        self.relative_cursor_position(text, cursor_index, self.scroll_offset)
            .offset(self.rectangle.offset())
    }

    /// Cursor position relative to the text area if scrolled to
    /// `scroll_offset`.
    fn relative_cursor_position(
        &self,
        text: RopeSlice<'_>,
        cursor_index: ByteIndex,
        scroll_offset: LineIndex,
    ) -> Position {
        let Some(text_width) = NonZeroColumns::new(self.rectangle.width()) else {
            return Position::default();
        };

        visual_cursor_position(text, cursor_index, scroll_offset, text_width)
    }
}
