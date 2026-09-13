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
    /// # Returns
    ///
    /// - The inner rectangle if a border was drawn,
    /// - The original rectangle if the border was not drawn
    pub(crate) fn draw_border(&mut self, rectangle: Rectangle, style: Style) -> DrawBorderOutcome {
        let Some(inner_rectangle) = rectangle.clip_border() else {
            return DrawBorderOutcome::NotDrawn {
                original_rectangle: rectangle,
            };
        };

        assert!(
            rectangle.width() >= Columns::new(3),
            "rectangle must be at least 3 cells wide in order to have a border"
        );
        assert!(
            rectangle.height() >= Rows::new(3),
            "rectangle must be at least 3 cells high in order to have a border"
        );

        let top_left = rectangle.offset();
        self[top_left].set_content("┌").set_style(style);

        for offset in Columns::new(1)..=(rectangle.width() - 2_usize) {
            self[top_left.col_offset(offset)]
                .set_content("─")
                .set_style(style);
        }

        let top_right = top_left.col_offset(rectangle.width() - 1_usize);
        self[top_right].set_content("┐").set_style(style);

        for row in Rows::new(1)..(rectangle.height() - 1_usize) {
            let left = top_left.row_offset(row);
            let right = left.col_offset(rectangle.width() - 1);

            self[left].set_content("│").set_style(style);
            self[right].set_content("│").set_style(style);
        }

        let bottom_left = top_left.row_offset(rectangle.height() - 1_usize);
        self[bottom_left].set_content("└").set_style(style);

        for offset in Columns::new(1)..=(rectangle.width() - 2_usize) {
            self[bottom_left.col_offset(offset)]
                .set_content("─")
                .set_style(style);
        }

        let bottom_right = bottom_left.col_offset(rectangle.width() - 1_usize);
        self[bottom_right].set_content("┘").set_style(style);

        DrawBorderOutcome::Drawn { inner_rectangle }
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

        for grapheme in
            GraphemeLayoutIterator::new(span.text().graphemes(true), WrapBehavior::NoWrap)
        {
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

    pub(crate) fn render_lines(&mut self, lines: &[Line], rectangle: &Rectangle) {
        let (mut current_rectangle, mut rest_rectangle) = rectangle.split_at_row(Rows::new(1));

        for line in lines {
            self.render_spans(line.spans(), &current_rectangle, Alignment::Left);

            (current_rectangle, rest_rectangle) = rest_rectangle.split_at_row(Rows::new(1));
        }
    }
}

#[derive(Debug)]
pub(crate) enum DrawBorderOutcome {
    Drawn { inner_rectangle: Rectangle },
    NotDrawn { original_rectangle: Rectangle },
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

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::*;

    #[derive(PartialEq, Eq)]
    struct TestRenderedBuffer(Vec<String>);

    impl fmt::Debug for TestRenderedBuffer {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            writeln!(formatter)?;

            for row in &self.0 {
                writeln!(formatter, "{row}")?;
            }

            writeln!(formatter)
        }
    }

    impl From<&[&str]> for TestRenderedBuffer {
        fn from(rows: &[&str]) -> Self {
            Self(rows.iter().map(|row| String::from(*row)).collect())
        }
    }

    macro_rules! assert_buffer_eq {
        ($buffer:expr, $expected:expr $(,)?) => {
            assert_eq!(
                rendered_buffer($buffer),
                TestRenderedBuffer::from(&($expected)[..])
            );
        };
    }

    fn rendered_buffer(buffer: &Buffer) -> TestRenderedBuffer {
        let row_width = buffer.dimensions().width().value();

        TestRenderedBuffer(
            buffer
                .cells()
                .chunks(row_width)
                .map(|cells| {
                    let mut row = String::new();
                    let mut cell_index = 0;

                    while cell_index < cells.len() {
                        let cell = &cells[cell_index];
                        let cell_width = cell.width().value();

                        assert!(cell_width > 0, "buffer cells must have a non-zero width");

                        row.push_str(&cell.content().replace(' ', "·"));
                        cell_index += cell_width;
                    }

                    row
                })
                .collect(),
        )
    }

    #[test]
    fn draws_border_if_there_is_room() {
        let buffer_dimensions = Dimensions::new(Columns::new(10), Rows::new(10));
        let mut buffer = Buffer::new(buffer_dimensions);
        let buffer_rectangle = Rectangle::from_dimensions(buffer_dimensions);

        let rectangle_dimensions = Dimensions::new(
            buffer_dimensions.width() - Columns::new(2),
            buffer_dimensions.height() - Rows::new(2),
        );

        let rectangle = match buffer.draw_border(
            buffer_rectangle.at_center(rectangle_dimensions),
            Style::COMMAND_LIST_BORDER,
        ) {
            DrawBorderOutcome::Drawn { inner_rectangle } => inner_rectangle,
            DrawBorderOutcome::NotDrawn { original_rectangle } => original_rectangle,
        };

        buffer.render_lines(&[Line::new(vec![Span::new("Hello")])], &rectangle);

        assert_buffer_eq!(&buffer, [
            "··········",
            "·┌──────┐·",
            "·│Hello·│·",
            "·│······│·",
            "·│······│·",
            "·│······│·",
            "·│······│·",
            "·│······│·",
            "·└──────┘·",
            "··········",
        ]);
    }

    #[test]
    fn does_not_draw_border_if_there_is_no_room() {
        let buffer_dimensions = Dimensions::new(Columns::new(4), Rows::new(4));
        let mut buffer = Buffer::new(buffer_dimensions);
        let buffer_rectangle = Rectangle::from_dimensions(buffer_dimensions);

        let rectangle_dimensions = Dimensions::new(
            buffer_dimensions.width() - Columns::new(2),
            buffer_dimensions.height() - Rows::new(2),
        );

        let rectangle = match buffer.draw_border(
            buffer_rectangle.at_center(rectangle_dimensions),
            Style::COMMAND_LIST_BORDER,
        ) {
            DrawBorderOutcome::Drawn { inner_rectangle } => inner_rectangle,
            DrawBorderOutcome::NotDrawn { original_rectangle } => original_rectangle,
        };

        buffer.render_lines(&[Line::new(vec![Span::new("Hi")])], &rectangle);

        #[rustfmt::skip]
        assert_buffer_eq!(&buffer, [
            "····",
            "·Hi·",
            "····",
            "····",
        ]);
    }
}
