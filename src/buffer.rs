use std::{
    cmp,
    ops,
};

use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    grapheme_layout::{
        GraphemeLayoutIterator,
        WrapBehavior,
    },
    style::{
        Style,
        StyleAttributes,
    },
    text::text_width,
    ui::{
        Alignment,
        Columns,
        Dimensions,
        Line,
        Position,
        Rectangle,
        Rows,
        Span,
        spans_width,
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
            cells: vec![Cell::default(); dimensions.area()],
            dimensions,
        }
    }

    pub(crate) fn resize(&mut self, dimensions: Dimensions) {
        self.cells.resize(dimensions.area(), Cell::default());
        self.dimensions = dimensions;
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

    /// Clears all contents and sets the [`Style`] of all cells within the given
    /// [`Rectangle`].
    pub(crate) fn clear_and_style_rectangle(&mut self, rectangle: &Rectangle, style: Style) {
        for position in rectangle
            .offset()
            .area_iter(rectangle.width(), rectangle.height())
        {
            self[position].reset().set_style(style);
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
    pub(crate) fn draw_border(&mut self, rectangle: &Rectangle, style: Style) -> Rectangle {
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
            .set_style(style);

        for row in (1..rectangle.height().value() - 1).map(Rows::new) {
            let left = top_left.row_offset(row);
            let right = top_left.offset(Position::new(rectangle.width() - Columns::new(1), row));

            self[left].set_content("│").set_style(style);
            self[right].set_content("│").set_style(style);
        }

        let bottom_left = top_left.row_offset(rectangle.height() - Rows::new(1));
        self[bottom_left]
            .set_content(&format!("└{}┘", "─".repeat(rectangle.width().value() - 2)))
            .set_style(style);

        rectangle.clip_border()
    }

    /// Renders each given [`Span`] **in order** inside the `rectangle`. See
    /// [`Buffer::render_span`] for more details on how spans get rendered.
    pub(crate) fn render_spans(
        &mut self,
        spans: &[Span],
        rectangle: &Rectangle,
        alignment: Alignment,
    ) {
        let mut position = match alignment {
            Alignment::Left => rectangle.offset(),
            Alignment::Right => {
                let offset = rectangle.offset();

                let start_column = cmp::max(
                    rectangle.right().saturating_sub(spans_width(spans)),
                    offset.left(),
                );
                Position::new(start_column, offset.top())
            }
        };

        for span in spans {
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
    pub(crate) fn render_span(
        &mut self,
        span: &Span,
        position: &Position,
        rectangle: &Rectangle,
    ) -> Position {
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
            cell.set_style(span.style());

            end_position = grapheme.end_position();
        }

        position.offset(end_position)
    }

    const fn position_index(&self, position: Position) -> usize {
        position.top().value() * self.dimensions.width().value() + position.left().value()
    }

    pub(crate) fn render_lines(&mut self, lines: Vec<Line>, rectangle: &Rectangle) {
        let (mut current_rectangle, mut rest_rectangle) = rectangle.split_at_row(Rows::new(1));

        for line in lines {
            self.render_spans(line.spans(), &current_rectangle, Alignment::Left);

            (current_rectangle, rest_rectangle) = rest_rectangle.split_at_row(Rows::new(1));
        }
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
    underline_color: Color,
    attributes: StyleAttributes,
}

impl Cell {
    pub(crate) fn new(content: &str) -> Self {
        Self {
            content: String::from(content),
            foreground: Color::Reset,
            background: Color::Reset,
            underline_color: Color::Reset,
            attributes: StyleAttributes::empty(),
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

    pub(crate) const fn underline_color(&self) -> Color {
        self.underline_color
    }

    pub(crate) const fn attributes(&self) -> StyleAttributes {
        self.attributes
    }

    pub(crate) fn set_content(&mut self, text: &str) -> &mut Self {
        self.content.clear();
        self.content.push_str(text);
        self
    }

    pub(crate) const fn set_style(&mut self, style: Style) -> &mut Self {
        if let Some(foreground) = style.foreground() {
            self.foreground = foreground;
        }

        if let Some(background) = style.background() {
            self.background = background;
        }

        if let Some(underline_color) = style.underline_color() {
            self.underline_color = underline_color;
        }

        self.attributes = style.attributes();

        self
    }

    pub(crate) fn width(&self) -> Columns {
        text_width(&self.content)
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
