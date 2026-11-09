use ropey::RopeSlice;

use crate::{
    text::{
        ByteIndex,
        Grapheme,
        LineIndex,
        RopeSliceExt as _,
    },
    ui::{
        NonZeroColumns,
        Position,
        WrapOutcome,
    },
};

/// Byte index corresponding to the start of a visual row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RowStart(ByteIndex);

impl RowStart {
    /// The given `index` should already have been determined to be the start of
    /// a row.
    pub(crate) const fn new_unchecked(index: ByteIndex) -> Self {
        Self(index)
    }

    pub(crate) fn from_line(text: RopeSlice<'_>, line: LineIndex) -> Self {
        Self(text.line_start_byte(line))
    }

    pub(crate) const fn byte_index(self) -> ByteIndex {
        self.0
    }
}

pub(crate) struct GraphemeLayoutIterator<'text, Graphemes>
where
    Graphemes: Iterator<Item = &'text str>,
{
    graphemes: Graphemes,
    position: Position,
    byte_index: ByteIndex,
    wrap_behavior: WrapBehavior,
}
impl<'text, Graphemes> GraphemeLayoutIterator<'text, Graphemes>
where
    Graphemes: Iterator<Item = &'text str>,
{
    pub(crate) fn new(graphemes: Graphemes, wrap_behavior: WrapBehavior) -> Self {
        Self {
            graphemes,
            position: Position::default(),
            byte_index: ByteIndex::new(0),
            wrap_behavior,
        }
    }

    pub(crate) fn starting_at(self, start: RowStart) -> Self {
        Self {
            byte_index: start.byte_index(),
            ..self
        }
    }
}

impl<'text, Graphemes> Iterator for GraphemeLayoutIterator<'text, Graphemes>
where
    Graphemes: Iterator<Item = &'text str>,
{
    type Item = VisualGrapheme<'text>;

    fn next(&mut self) -> Option<Self::Item> {
        let grapheme = self.graphemes.next()?;

        let byte_index = self.byte_index;
        self.byte_index += grapheme.len();

        let grapheme = Grapheme::from(grapheme);

        let (position, wrap_status) = match self.wrap_behavior {
            WrapBehavior::NoWrap => (self.position, WrapOutcome::NotWrapped),
            WrapBehavior::Wrap { max_width } => self.position.wrap(&grapheme, max_width),
        };

        self.position = position.advance(&grapheme);

        Some(VisualGrapheme {
            grapheme,
            wrap_status,
            position,
            end_position: self.position,
            byte_index,
        })
    }
}

#[derive(Debug)]
pub(crate) enum WrapBehavior {
    Wrap { max_width: NonZeroColumns },
    NoWrap,
}

#[derive(Debug)]
pub(crate) struct VisualGrapheme<'text> {
    grapheme: Grapheme<'text>,
    wrap_status: WrapOutcome,
    position: Position,
    end_position: Position,
    byte_index: ByteIndex,
}

impl VisualGrapheme<'_> {
    pub(crate) const fn position(&self) -> Position {
        self.position
    }

    pub(crate) const fn end_position(&self) -> Position {
        self.end_position
    }

    pub(crate) const fn wrap_status(&self) -> WrapOutcome {
        self.wrap_status
    }

    pub(crate) const fn grapheme(&self) -> &Grapheme<'_> {
        &self.grapheme
    }

    pub(crate) const fn byte_index(&self) -> ByteIndex {
        self.byte_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        Columns,
        Rows,
    };

    #[test]
    fn wide_grapheme_wraps_before_it_exceeds_the_maximum_width() {
        let max_width = NonZeroColumns::new(Columns::new(2)).expect("width is not zero");
        let graphemes =
            GraphemeLayoutIterator::new(["a", "😀"].into_iter(), WrapBehavior::Wrap {
                max_width,
            });

        assert_eq!(
            graphemes
                .map(|grapheme| grapheme.position())
                .collect::<Vec<_>>(),
            vec![
                Position::new(Columns::new(0), Rows::new(0)),
                Position::new(Columns::new(0), Rows::new(1))
            ]
        );
    }
}
