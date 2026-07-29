use crate::{
    document::Grapheme,
    text::ByteIndex,
    ui::{
        NonZeroColumns,
        Position,
        WrapOutcome,
    },
};

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

        let (is_wrapped, position) = match self.wrap_behavior {
            WrapBehavior::NoWrap => (false, self.position),
            WrapBehavior::Wrap { max_width } => {
                let (position, outcome) = self.position.wrap(max_width);

                let is_wrapped = match outcome {
                    WrapOutcome::Wrapped => true,
                    WrapOutcome::NotWrapped => false,
                };

                (is_wrapped, position)
            }
        };

        self.position = position.advance(&grapheme);

        Some(VisualGrapheme {
            grapheme,
            is_wrapped,
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
    is_wrapped: bool,
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
