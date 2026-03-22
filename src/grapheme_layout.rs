use crate::{
    document::{
        Grapheme,
        Position,
        WrapOutcome,
    },
    terminal::Columns,
    text::ByteIndex,
};

pub(crate) struct GraphemeLayoutIterator<'text, Graphemes>
where
    Graphemes: Iterator<Item = &'text str>,
{
    graphemes: Graphemes,
    max_width: Columns,
    position: Position,
    byte_index: ByteIndex,
}
impl<'text, Graphemes> GraphemeLayoutIterator<'text, Graphemes>
where
    Graphemes: Iterator<Item = &'text str>,
{
    pub(crate) fn new(graphemes: Graphemes, max_width: Columns) -> Self {
        Self {
            graphemes,
            max_width,
            position: Position::default(),
            byte_index: ByteIndex::new(0),
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

        let (position, outcome) = self.position.wrap(self.max_width);

        let is_wrapped = match outcome {
            WrapOutcome::Wrapped => true,
            WrapOutcome::NotWrapped => false,
        };

        self.position = position.advance(&grapheme);

        Some(VisualGrapheme {
            grapheme,
            is_wrapped,
            position,
            byte_index,
        })
    }
}

#[derive(Debug)]
pub(crate) struct VisualGrapheme<'text> {
    grapheme: Grapheme<'text>,
    is_wrapped: bool,
    position: Position,
    byte_index: ByteIndex,
}

impl VisualGrapheme<'_> {
    pub(crate) const fn position(&self) -> Position {
        self.position
    }

    pub(crate) const fn is_wrapped(&self) -> bool {
        self.is_wrapped
    }

    pub(crate) const fn grapheme(&self) -> &Grapheme<'_> {
        &self.grapheme
    }

    pub(crate) const fn byte_index(&self) -> ByteIndex {
        self.byte_index
    }
}
