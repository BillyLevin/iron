mod rust;

use std::ops::Range;

use ropey::Rope;

use crate::{
    highlight::rust::RustLexer,
    language::Language,
    text::ByteIndex,
};

#[derive(Debug)]
pub(crate) struct Highlighter {
    lexer: Lexer,
}

impl Highlighter {
    pub(crate) fn new(source: Rope, start: ByteIndex, language: Language) -> Self {
        Self {
            lexer: match language {
                Language::Rust => Lexer::Rust(RustLexer::new(source, start)),
                Language::Toml | Language::Text => Lexer::Default,
            },
        }
    }
}

impl Iterator for Highlighter {
    type Item = (Token, Checkpoint);

    fn next(&mut self) -> Option<Self::Item> {
        self.lexer.next_token()
    }
}

#[derive(Debug)]
enum Lexer {
    Rust(RustLexer),
    Default,
}

impl Lexer {
    fn next_token(&mut self) -> Option<(Token, Checkpoint)> {
        match *self {
            Self::Rust(ref mut lexer) => lexer.next_token(),
            Self::Default => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Token {
    kind: TokenKind,
    range: Range<ByteIndex>,
}

impl Token {
    pub(crate) const fn kind(&self) -> TokenKind {
        self.kind
    }

    pub(crate) fn contains(&self, index: ByteIndex) -> bool {
        self.range.contains(&index)
    }

    pub(crate) const fn start(&self) -> ByteIndex {
        self.range.start
    }

    pub(crate) const fn end(&self) -> ByteIndex {
        self.range.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Identifier,
    Whitespace,
    Keyword,
    String,
    Type,
    Comment,
    Operator,
    Unknown,
    Character,
    Lifetime,
    FunctionName,
    Punctuation,
    Number,
    Macro,
    Property,
    PropertyAccess,
    Constant,
    EnumMember,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Checkpoint {
    Yes,
    No,
}
