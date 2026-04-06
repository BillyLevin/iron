use std::ops;

use crossterm::style::Color;
use unicode_width::UnicodeWidthStr as _;

use crate::{
    document::Position,
    terminal::Dimensions,
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

    pub(crate) fn clear(&mut self) {
        self.cells.fill_with(Cell::default);
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
