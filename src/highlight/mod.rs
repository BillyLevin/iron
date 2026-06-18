mod rust;
mod toml;

use std::ops::Range;

use ropey::Rope;

use crate::{
    highlight::{
        rust::RustLexer,
        toml::TomlLexer,
    },
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
                Language::Rust => Lexer::Rust(RustLexer::new(Source::new(source, start))),
                Language::Toml => Lexer::Toml(TomlLexer::new(Source::new(source, start))),
                Language::Text => Lexer::Default,
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
    Toml(TomlLexer),
    Default,
}

impl Lexer {
    fn next_token(&mut self) -> Option<(Token, Checkpoint)> {
        match *self {
            Self::Rust(ref mut lexer) => lexer.next_token(),
            Self::Toml(ref mut lexer) => lexer.next_token(),
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
    Title,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Checkpoint {
    Yes,
    No,
}

#[derive(Debug)]
struct Source {
    text: Rope,
    position: ByteIndex,
    current: Option<char>,
}

impl Source {
    fn new(text: Rope, start: ByteIndex) -> Self {
        let current = text.get_char(start.value()).ok();

        Self {
            text,
            position: start,
            current,
        }
    }

    fn next_char(&mut self) -> Option<char> {
        self.position += self.current?.len_utf8();
        self.current = self.text.get_char(self.position.value()).ok();
        self.current
    }

    fn peek(&self) -> Option<char> {
        self.text.chars_at(self.position.value()).nth(1)
    }

    fn peek_2(&self) -> Option<char> {
        self.text.chars_at(self.position.value()).nth(2)
    }

    fn eat_while(&mut self, mut condition: impl FnMut(char) -> bool) {
        while self.current.is_some_and(&mut condition) {
            self.next_char();
        }
    }

    /// Assert that the current character is `ch`, and advances to the next
    /// character. This should only be called if the condition is guaranteed
    /// to be true.
    fn assert(&mut self, ch: char) {
        assert!(self.eat_if(ch), "`current` must be '{ch}'");
    }

    fn eat_if(&mut self, ch: char) -> bool {
        if self.current.is_some_and(|c| c == ch) {
            self.next_char();
            true
        } else {
            false
        }
    }

    fn bytes_at(&self, index: ByteIndex) -> ropey::iter::Bytes<'_> {
        self.text.bytes_at(index.value())
    }

    fn chars_at(&self, index: ByteIndex) -> ropey::iter::Chars<'_> {
        self.text.chars_at(index.value())
    }
}
