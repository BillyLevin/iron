use std::ops;

use crate::buffer::Buffer;

/// A structure representing (unsurprisingly) a rectangular region of the
/// interface.
#[derive(Debug)]
pub(crate) struct Rectangle {
    /// How far from the top-left of the interface that the top-left of the
    /// rectangle begins.
    offset: Offset,
    /// The size of the rectangle.
    dimensions: Dimensions,
}

impl Rectangle {
    pub(crate) fn from_buffer(buffer: &Buffer) -> Self {
        Self {
            offset: Offset::default(),
            dimensions: buffer.dimensions(),
        }
    }

    pub(crate) const fn height(&self) -> Rows {
        self.dimensions.height
    }

    pub(crate) const fn width(&self) -> Columns {
        self.dimensions.width
    }

    /// Splits the current [`Rectangle`] into two vertically stacked
    /// [`Rectangle`]s at the given `row`. This is **inclusive**, so for row
    /// `n`, `top` keeps the first `n` rows, and the bottom gets the rest.
    pub(crate) fn split_at(&self, row: Rows) -> (Self, Self) {
        let bottom_height = self.dimensions.height.saturating_sub(row);

        assert!(
            bottom_height <= self.dimensions.height,
            "we have subtracted from `self.dimensions.height` above"
        );
        let top_height = self.dimensions.height - bottom_height;

        let top = Self {
            dimensions: Dimensions::new(self.dimensions.width, top_height),
            offset: self.offset,
        };

        let bottom = Self {
            dimensions: Dimensions::new(self.dimensions.width, bottom_height),
            offset: Offset::new(self.offset.left, self.offset.top + top_height),
        };

        (top, bottom)
    }

    pub(crate) const fn offset(&self) -> Offset {
        self.offset
    }

    /// Constructs a new [`Rectangle`] of size `dimensions` within `self`,
    /// placed at the bottom right of `self`.
    ///
    /// # Panics
    ///
    ///  Panics if the given `dimensions` are larger than the dimensions of
    /// `self`. This precondition should be guaranteed by the caller.
    pub(crate) fn bottom_right(&self, dimensions: Dimensions) -> Self {
        // TODO: should these be errors rather an assertions?
        assert!(
            dimensions.width() <= self.width(),
            "it is illegal to construct a rectangle that's larger than its container"
        );
        assert!(
            dimensions.height() <= self.height(),
            "it is illegal to construct a rectangle that's larger than its container"
        );
        Self {
            offset: Offset::new(
                self.width().saturating_sub(dimensions.width),
                self.height().saturating_sub(dimensions.height),
            ),
            dimensions,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Offset {
    left: Columns,
    top: Rows,
}

impl Offset {
    pub(crate) const fn new(left: Columns, top: Rows) -> Self {
        Self { left, top }
    }

    pub(crate) const fn left(&self) -> Columns {
        self.left
    }

    pub(crate) const fn top(&self) -> Rows {
        self.top
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Dimensions {
    width: Columns,
    height: Rows,
}

impl Dimensions {
    pub(crate) const fn new(columns: Columns, rows: Rows) -> Self {
        Self {
            width: columns,
            height: rows,
        }
    }

    pub(crate) const fn width(&self) -> Columns {
        self.width
    }

    pub(crate) const fn height(&self) -> Rows {
        self.height
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
    derive_more::Sub,
)]
#[from(forward)]
pub(crate) struct Columns(usize);

impl Columns {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> usize {
        self.0
    }

    pub(crate) fn map(self, map_fn: impl FnOnce(usize) -> usize) -> Self {
        Self(map_fn(self.0))
    }

    #[must_use = "`saturating_sub` does not mutate the current value, but returns a new value"]
    pub(crate) const fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl ops::Add<usize> for Columns {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl ops::AddAssign<usize> for Columns {
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
    derive_more::Div,
    derive_more::Sub,
)]
#[from(forward)]
pub(crate) struct Rows(usize);

impl Rows {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> usize {
        self.0
    }

    #[must_use = "`saturating_sub` does not mutate the current value, but returns a new value"]
    pub(crate) const fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_at() {
        let _ = color_eyre::install();

        let dimensions = Dimensions::new(Columns::new(80), Rows::new(24));
        let buffer = Buffer::new(dimensions);

        let rectangle = Rectangle::from_buffer(&buffer);

        let (top_rect, bottom_rect) = rectangle.split_at(Rows::new(20));

        assert_eq!(
            top_rect.dimensions,
            Dimensions::new(Columns::new(80), Rows::new(20))
        );

        assert_eq!(top_rect.offset, Offset::new(Columns::new(0), Rows::new(0)));

        assert_eq!(
            bottom_rect.dimensions,
            Dimensions::new(Columns::new(80), Rows::new(4))
        );

        assert_eq!(
            bottom_rect.offset,
            Offset::new(Columns::new(0), Rows::new(20))
        );
    }
}
