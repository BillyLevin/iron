use std::{
    fs::File,
    io::{self, BufReader},
    path::PathBuf,
};

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use crate::{
    buffer::Buffer,
    terminal::{Columns, Dimensions, Rows},
};

#[derive(Debug)]
pub(crate) struct Document {
    text: Rope,
}

impl Document {
    pub(crate) fn new(file_path: &PathBuf) -> io::Result<Self> {
        Ok(Self {
            text: Rope::from_reader(BufReader::new(File::open(file_path)?))?,
        })
    }

    pub(crate) fn render(&self, buffer: &mut Buffer, dimensions: &Dimensions) {
        let graphemes = self.text.chunks().flat_map(|chunk| chunk.graphemes(true));
        let mut position = Position::default();

        for grapheme in graphemes {
            if position.top() >= dimensions.height() {
                break;
            }

            let grapheme = Grapheme::from(grapheme);

            buffer[position].set_content(match grapheme {
                Grapheme::LineBreak => " ",
                Grapheme::Text(text) => text,
            });

            position = position.advance(&grapheme);
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Position {
    left: Columns,
    top: Rows,
}

impl Position {
    pub(crate) const fn top(&self) -> &Rows {
        &self.top
    }

    pub(crate) const fn left(&self) -> &Columns {
        &self.left
    }

    fn advance(self, grapheme: &Grapheme) -> Self {
        match grapheme {
            Grapheme::LineBreak => Self {
                left: Columns::from(0usize),
                top: self.top + Rows::from(1usize),
            },
            Grapheme::Text(text) => Self {
                left: self.left + Columns::from(text.width()),
                top: self.top,
            },
        }
    }
}

#[derive(Debug)]
enum Grapheme<'grapheme> {
    LineBreak,
    Text(&'grapheme str),
}

impl<'grapheme> From<&'grapheme str> for Grapheme<'grapheme> {
    fn from(value: &'grapheme str) -> Self {
        match value {
            "\n" | "\r\n" => Self::LineBreak,
            _ => Self::Text(value),
        }
    }
}
