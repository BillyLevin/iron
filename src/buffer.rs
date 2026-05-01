use std::{
    cmp,
    ops,
};

use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use crate::{
    grapheme_layout::{
        GraphemeLayoutIterator,
        WrapBehavior,
    },
    ui::{
        Alignment,
        Columns,
        Dimensions,
        Position,
        Rectangle,
        Rows,
        Span,
        Spans,
    },
};

#[derive(Debug)]
pub(crate) struct Buffer {
    cells: Vec<Cell>,
    dimensions: Dimensions,
}

impl Buffer {
    pub(crate) fn new(dimensions: Dimensions) -> Self {
        Self {
            cells: vec![Cell::default(); dimensions.height().value() * dimensions.width().value()],
            dimensions,
        }
    }

    pub(crate) fn cells(&self) -> &[Cell] {
        &self.cells
    }

    pub(crate) const fn dimensions(&self) -> Dimensions {
        self.dimensions
    }

    pub(crate) fn clear(&mut self) {
        self.cells.fill_with(Cell::default);
    }

    /// Sets the background color for each cell within the provided `rectangle`.
    pub(crate) fn fill_background(&mut self, rectangle: &Rectangle, color: Color) {
        for position in rectangle
            .offset()
            .area_iter(rectangle.width(), rectangle.height())
        {
            self[position].reset().set_background(color);
        }
    }

    /// Draws a border on the edges of the given `rectangle` if there's room to
    /// do so.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///   * `rectangle.width() >= Columns::new(3)`, or:
    ///   * `rectangle.height() >= Rows::new(3)`
    ///
    /// because there wouldn't be room to draw a border.
    ///
    /// # Returns
    ///
    /// For convenience, returns the inner rectangle that doesn't include the
    /// border.
    pub(crate) fn draw_border(&mut self, rectangle: &Rectangle, color: Color) -> Rectangle {
        assert!(
            rectangle.width() >= Columns::new(3),
            "rectangle must be at least 3 cells wide in order to have a border"
        );
        assert!(
            rectangle.height() >= Rows::new(3),
            "rectangle must be at least 3 cells high in order to have a border"
        );

        let top_left = rectangle.offset();
        self[top_left]
            .set_content(&format!("┌{}┐", "─".repeat(rectangle.width().value() - 2)))
            .set_foreground(color);

        for row in (1..rectangle.height().value() - 1).map(Rows::new) {
            let left = top_left.offset(Position::new(Columns::new(0), row));
            let right = top_left.offset(Position::new(rectangle.width() - Columns::new(1), row));

            self[left].set_content("│").set_foreground(color);
            self[right].set_content("│").set_foreground(color);
        }

        let bottom_left = top_left.offset(Position::new(
            Columns::new(0),
            rectangle.height() - Rows::new(1),
        ));
        self[bottom_left]
            .set_content(&format!("└{}┘", "─".repeat(rectangle.width().value() - 2)))
            .set_foreground(color);

        rectangle.clip_border()
    }

    /// Renders each given [`Span`] **in order** inside the `rectangle`. See
    /// [`Buffer::render_span`] for more details on how spans get rendered.
    pub(crate) fn render_spans(
        &mut self,
        spans: &Spans,
        rectangle: &Rectangle,
        alignment: Alignment,
    ) {
        let mut position = match alignment {
            Alignment::Left => rectangle.offset(),
            Alignment::Right => {
                let offset = rectangle.offset();

                let start_column = cmp::max(
                    rectangle.right().saturating_sub(spans.width()),
                    offset.left(),
                );
                Position::new(start_column, offset.top())
            }
        };

        for span in spans.as_ref() {
            position = self.render_span(span, &position, rectangle);
        }
    }

    /// Renders a [`Span`] inside a [`Rectangle`], starting at the given
    /// [`Position`]. The text does NOT wrap - if we run out of space,
    /// we simply stop rendering any more text.
    ///
    /// # Returns
    ///
    /// The position at the end of the text that actually got rendered.
    fn render_span(&mut self, span: &Span, position: &Position, rectangle: &Rectangle) -> Position {
        let mut end_position = *position;

        for grapheme in GraphemeLayoutIterator::new(
            span.text().graphemes(true),
            rectangle.width(),
            WrapBehavior::NoWrap,
        ) {
            let current_position = position.offset(grapheme.position());

            if !rectangle.contains(&current_position) {
                break;
            }

            let cell = &mut self[current_position];

            cell.set_content(grapheme.grapheme().as_str());

            if let Some(fg) = span.fg() {
                cell.set_foreground(fg);
            }

            if let Some(bg) = span.bg() {
                cell.set_background(bg);
            }

            end_position = grapheme.end_position();
        }

        position.offset(end_position)
    }

    const fn position_index(&self, position: Position) -> usize {
        position.top().value() * self.dimensions.width().value() + position.left().value()
    }
}

impl ops::Index<Position> for Buffer {
    type Output = Cell;

    fn index(&self, position: Position) -> &Self::Output {
        &self.cells[self.position_index(position)]
    }
}

impl ops::IndexMut<Position> for Buffer {
    fn index_mut(&mut self, position: Position) -> &mut Self::Output {
        let index = self.position_index(position);
        &mut self.cells[index]
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Cell {
    content: String,
    foreground: Color,
    background: Color,
}

impl Cell {
    pub(crate) fn new(content: &str) -> Self {
        Self {
            content: String::from(content),
            foreground: Color::Reset,
            background: Color::Reset,
        }
    }

    pub(crate) fn content(&self) -> &str {
        &self.content
    }

    pub(crate) const fn foreground(&self) -> Color {
        self.foreground
    }

    pub(crate) const fn background(&self) -> Color {
        self.background
    }

    pub(crate) fn set_content(&mut self, text: &str) -> &mut Self {
        self.content.clear();
        self.content.push_str(text);
        self
    }

    pub(crate) const fn set_foreground(&mut self, foreground: Color) -> &mut Self {
        self.foreground = foreground;
        self
    }

    pub(crate) const fn set_background(&mut self, background: Color) -> &mut Self {
        self.background = background;
        self
    }

    pub(crate) fn width(&self) -> usize {
        self.content.width()
    }

    pub(crate) fn reset(&mut self) -> &mut Self {
        *self = Self::default();
        self
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::new(" ")
    }
}
