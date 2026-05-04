use std::ops;

use crossterm::{
    event::Event,
    style::Color,
};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    buffer::Buffer,
    document::Grapheme,
    editor::{
        EventContext,
        EventOutcome,
    },
};

/// A visual layer in the UI. The app can have multiple layers that are
/// displayed on top of each other.
pub(crate) trait Layer {
    /// Fill the editor's [`Buffer`] with cells. The buffer will later draw
    /// these cells to the terminal.
    fn render(&mut self, buffer: &mut Buffer);

    /// Optionally handle an event if it applies to this layer.
    fn handle_event(&mut self, event: &Event, event_context: &mut EventContext) -> EventOutcome;

    /// Calculate the visual cursor position. This is relative to the whole
    /// screen, not the viewport of this layer.
    fn visual_cursor_position(&self) -> Option<Position>;

    /// Check for and handle any events that occur within the
    /// layer.
    fn handle_internal_events(&mut self) -> EventOutcome;

    /// Get the type of this layer. This is an [`Option`] because it only
    /// applies to layers that can be added/removed.
    fn kind(&self) -> Option<LayerKind>;
}

/// The types of layers that can be added/removed. In other words, this doesn't
/// include [`Document`](crate::document::Document`) despite it being [`Layer`]
/// because it's a permanent layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayerKind {
    CommandList,
}

/// A structure representing (unsurprisingly) a rectangular region of the
/// interface.
#[derive(Debug)]
pub(crate) struct Rectangle {
    /// How far from the top-left of the interface that the top-left of the
    /// rectangle begins.
    offset: Position,
    /// The size of the rectangle.
    dimensions: Dimensions,
}

impl Rectangle {
    pub(crate) fn from_dimensions(dimensions: Dimensions) -> Self {
        Self {
            offset: Position::default(),
            dimensions,
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
    pub(crate) fn split_at_row(&self, row: Rows) -> (Self, Self) {
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
            offset: Position::new(self.offset.left(), self.offset.top() + top_height),
        };

        (top, bottom)
    }

    /// Splits the current [`Rectangle`] into two vertically stacked
    /// [`Rectangle`]s at the given `column`. This is **inclusive**, so for
    /// column `n`, `left` keeps the first `n` rows, and the right gets the
    /// rest.
    pub(crate) fn split_at_column(&self, column: Columns) -> (Self, Self) {
        let right_width = self.dimensions.width.saturating_sub(column);

        assert!(
            right_width <= self.dimensions.width,
            "we have subtracted from `self.dimensions.height` above"
        );
        let left_width = self.dimensions.width - right_width;

        let left = Self {
            dimensions: Dimensions::new(left_width, self.dimensions.height),
            offset: self.offset,
        };

        let right = Self {
            dimensions: Dimensions::new(right_width, self.dimensions.height),
            offset: Position::new(self.offset.left() + left_width, self.offset.top()),
        };

        (left, right)
    }

    pub(crate) const fn offset(&self) -> Position {
        self.offset
    }

    /// Constructs a new [`Rectangle`] of size `dimensions` within `self`,
    /// placed at the bottom right of `self`.
    ///
    /// # Panics
    ///
    ///  Panics if the given `dimensions` are larger than the dimensions of
    /// `self`. This precondition should be guaranteed by the caller.
    pub(crate) fn at_bottom_right(&self, dimensions: Dimensions) -> Self {
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
            offset: Position::new(
                self.width().saturating_sub(dimensions.width),
                self.height().saturating_sub(dimensions.height),
            ),
            dimensions,
        }
    }

    /// Constructs a new [`Rectangle`] of dimensions `dimensions` within `self`,
    /// placed in the center of `self`.
    ///
    /// # Panics
    ///
    ///  Panics if the given `dimensions` are larger than the dimensions of
    /// `self`. This precondition should be guaranteed by the caller.
    pub(crate) fn at_center(&self, dimensions: Dimensions) -> Self {
        // TODO: should these be errors rather than assertions?
        assert!(
            dimensions.width() <= self.width(),
            "it is illegal to construct a rectangle that's larger than its container"
        );
        assert!(
            dimensions.height() <= self.height(),
            "it is illegal to construct a rectangle that's larger than its container"
        );

        let available_height = self.height() - dimensions.height();
        let available_width = self.width() - dimensions.width();

        Self {
            offset: Position::new(available_width / 2, available_height / 2),
            dimensions,
        }
    }

    /// Gets the column of the right **edge** (i.e. it is NOT inside) of the
    /// [`Rectangle`].
    pub(crate) fn right(&self) -> Columns {
        self.offset().left() + self.width()
    }

    /// Gets the row of the bottom **edge** (i.e. it is NOT inside) of the
    /// [`Rectangle`].
    pub(crate) fn bottom(&self) -> Rows {
        self.offset().top() + self.height()
    }

    /// Determines whether the given [`Position`] is inside the [`Rectangle`].
    /// NOTE: being on the edge does NOT count as being inside.
    pub(crate) fn contains(&self, position: &Position) -> bool {
        self.right() > position.left() && self.bottom() > position.top()
    }

    /// Gets the inner [`Rectangle`] of the `self`, assuming that it's bordered.
    /// This should only be called if `self` has been validated to be large
    /// enough to have a border.
    ///
    /// # Panics
    ///
    /// Panics if `self.width() < 3` or `self.height() < 3`.
    pub(crate) fn clip_border(&self) -> Self {
        assert!(
            self.width() >= Columns::new(3),
            "rectangle must be at least 3 cells wide in order to have a border"
        );
        assert!(
            self.height() >= Rows::new(3),
            "rectangle must be at least 3 cells high in order to have a border"
        );

        Self {
            offset: self
                .offset
                .offset(Position::new(Columns::new(1), Rows::new(1))),
            dimensions: Dimensions::new(
                self.width() - Columns::new(2),
                self.height() - Rows::new(2),
            ),
        }
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
    derive_more::Div,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Position {
    left: Columns,
    top: Rows,
}

impl Position {
    pub(crate) const fn new(left: Columns, top: Rows) -> Self {
        Self { left, top }
    }

    pub(crate) const fn top(&self) -> Rows {
        self.top
    }

    pub(crate) const fn left(&self) -> Columns {
        self.left
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
        if self.left() < max_width {
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
    pub(crate) fn col_offset(&self, gutter_width: Columns) -> Self {
        Self {
            left: self.left + gutter_width,
            top: self.top,
        }
    }

    #[must_use]
    pub(crate) fn row_offset(&self, offset: Rows) -> Self {
        Self {
            left: self.left,
            top: self.top + offset,
        }
    }

    #[must_use]
    pub(crate) fn offset(self, offset: Self) -> Self {
        Self {
            left: offset.left() + self.left,
            top: offset.top() + self.top,
        }
    }

    /// Creates an iterator over each [`Position`] in the given area, assuming
    /// that `self` is at the top-left of the area.
    pub(crate) fn area_iter(&self, width: Columns, height: Rows) -> impl Iterator<Item = Self> {
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

#[derive(Debug)]
pub(crate) enum WrapOutcome {
    Wrapped,
    NotWrapped,
}

#[derive(Debug)]
pub(crate) struct Line {
    spans: Vec<Span>,
}

impl Line {
    pub(crate) const fn new(spans: Vec<Span>) -> Self {
        Self { spans }
    }

    pub(crate) const fn spans(&self) -> &Vec<Span> {
        &self.spans
    }
}

#[derive(Debug)]
pub(crate) struct Span {
    text: String,
    foreground: Option<Color>,
    background: Option<Color>,
}

impl Span {
    #[must_use]
    pub(crate) const fn new(text: String) -> Self {
        Self {
            text,
            foreground: None,
            background: None,
        }
    }

    pub(crate) fn with_fg(self, foreground: Color) -> Self {
        Self {
            foreground: Some(foreground),
            ..self
        }
    }

    pub(crate) fn with_bg(self, background: Color) -> Self {
        Self {
            background: Some(background),
            ..self
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) const fn fg(&self) -> Option<Color> {
        self.foreground
    }

    pub(crate) const fn bg(&self) -> Option<Color> {
        self.background
    }
}

pub(crate) fn spans_width(spans: &[Span]) -> Columns {
    spans
        .iter()
        .map(|span| Columns::new(span.text().width()))
        .sum()
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Alignment {
    Left,
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_split_at_row() {
        let _ = color_eyre::install();

        let rectangle =
            Rectangle::from_dimensions(Dimensions::new(Columns::new(80), Rows::new(24)));

        let (top_rect, bottom_rect) = rectangle.split_at_row(Rows::new(20));

        assert_eq!(
            top_rect.dimensions,
            Dimensions::new(Columns::new(80), Rows::new(20))
        );

        assert_eq!(
            top_rect.offset,
            Position::new(Columns::new(0), Rows::new(0))
        );

        assert_eq!(
            bottom_rect.dimensions,
            Dimensions::new(Columns::new(80), Rows::new(4))
        );

        assert_eq!(
            bottom_rect.offset,
            Position::new(Columns::new(0), Rows::new(20))
        );
    }

    #[test]
    fn rectangle_split_at_column() {
        let _ = color_eyre::install();

        let rectangle =
            Rectangle::from_dimensions(Dimensions::new(Columns::new(80), Rows::new(24)));

        let (left_rect, right_rect) = rectangle.split_at_column(Columns::new(60));

        assert_eq!(
            left_rect.dimensions,
            Dimensions::new(Columns::new(60), Rows::new(24))
        );

        assert_eq!(
            left_rect.offset,
            Position::new(Columns::new(0), Rows::new(0))
        );

        assert_eq!(
            right_rect.dimensions,
            Dimensions::new(Columns::new(20), Rows::new(24))
        );

        assert_eq!(
            right_rect.offset,
            Position::new(Columns::new(60), Rows::new(0))
        );
    }

    #[test]
    fn rectangle_bottom_right() {
        let _ = color_eyre::install();

        let container =
            Rectangle::from_dimensions(Dimensions::new(Columns::new(80), Rows::new(24)));

        let rectangle = container.at_bottom_right(Dimensions::new(Columns::new(10), Rows::new(10)));

        assert_eq!(rectangle.offset().left(), Columns::new(70));
        assert_eq!(rectangle.offset().top(), Rows::new(14));
        assert_eq!(
            rectangle.dimensions,
            Dimensions::new(Columns::new(10), Rows::new(10))
        );
        assert_eq!(rectangle.right(), Columns::new(80));
        assert_eq!(rectangle.bottom(), Rows::new(24));
    }
}
