use std::{
    cmp,
    ops::Range,
};

use ropey::RopeSlice;

use crate::{
    text::{
        ByteIndex,
        RopeSliceExt as _,
    },
    ui::Columns,
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
