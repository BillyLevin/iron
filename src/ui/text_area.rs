use ropey::RopeSlice;

use crate::{
    cursor::visual_cursor_position,
    grapheme_layout::RowStart,
    text::{
        ByteIndex,
        LineIndex,
        RopeSliceExt as _,
    },
    ui::{
        NonZeroColumns,
        Position,
        Rectangle,
        Rows,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextArea {
    rectangle: Rectangle,
    scroll_offset: RowStart,
}

impl TextArea {
    pub(crate) const fn new(rectangle: Rectangle) -> Self {
        Self {
            rectangle,
            scroll_offset: RowStart::new_unchecked(ByteIndex::new(0)),
        }
    }

    pub(crate) const fn area(&self) -> &Rectangle {
        &self.rectangle
    }

    pub(crate) const fn scroll_offset(&self) -> RowStart {
        self.scroll_offset
    }

    pub(crate) fn scroll_to_line(&mut self, text: RopeSlice<'_>, line: LineIndex) {
        self.scroll_offset = RowStart::from_line(text, line);
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
        let height = self.rectangle.height();

        let anchor = self.scroll_offset.byte_index();

        let Some(text_width) = self.text_width() else {
            return;
        };

        self.scroll_offset = if cursor_index < anchor {
            // upwards scroll
            text.row_start_containing_byte(cursor_index, text_width)
        } else {
            let cursor_line = text.line_idx_containing_byte(cursor_index);
            let anchor_line = text.line_idx_containing_byte(anchor);

            // move the scroll offset further down so that we don't have to scan
            // as much of the rope when calculating the cursor
            // position. `scroll_offset` will still be
            // off the screen, so it won't impact the calculation
            let closer_anchor = if cursor_line > anchor_line + height.value() {
                RowStart::from_line(text, cursor_line.saturating_sub(height.value()))
            } else {
                text.row_start_containing_byte(anchor, text_width)
            };

            let cursor_row =
                visual_cursor_position(text, cursor_index, closer_anchor, text_width).top();

            // downwards scroll
            if cursor_row >= height {
                let hidden_rows = cursor_row + Rows::new(1) - height;

                let start_line = text.line_idx_containing_byte(closer_anchor.byte_index());

                let end_line = text.last_line_idx();

                (start_line.value()..=end_line.value())
                    .map(LineIndex::new)
                    .flat_map(|line_index| text.row_starts_for_line(line_index, text_width))
                    .skip_while(|&row_start| row_start < closer_anchor)
                    .nth(hidden_rows.value())
                    .unwrap_or_else(|| RowStart::new_unchecked(ByteIndex::new(text.len())))
            } else {
                closer_anchor
            }
        };
    }

    pub(crate) fn visual_cursor_position(
        &self,
        text: RopeSlice<'_>,
        cursor_index: ByteIndex,
    ) -> Position {
        self.text_width()
            .map_or_else(Position::default, |text_width| {
                visual_cursor_position(text, cursor_index, self.scroll_offset, text_width)
            })
            .offset(self.rectangle.offset())
    }

    fn text_width(&self) -> Option<NonZeroColumns> {
        NonZeroColumns::new(self.rectangle.width())
    }
}
