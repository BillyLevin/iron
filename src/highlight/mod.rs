mod rust;
mod toml;

use std::{
    collections::VecDeque,
    ops::Range,
};

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
    pending: VecDeque<(Token, Checkpoint)>,
    text: Rope,
}

impl Highlighter {
    pub(crate) fn new(source: Rope, start: ByteIndex, language: Language) -> Self {
        let text = source.clone();

        Self {
            lexer: match language {
                Language::Rust => Lexer::Rust(RustLexer::new(Source::new(source, start))),
                Language::Toml => Lexer::Toml(TomlLexer::new(Source::new(source, start))),
                Language::Text => Lexer::Default,
            },
            text,
            pending: VecDeque::new(),
        }
    }
}

impl Iterator for Highlighter {
    type Item = (Token, Checkpoint);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(token) = self.pending.pop_front() {
            return Some(token);
        }

        let (token, checkpoint) = self.lexer.next_token()?;

        if matches!(token.kind(), TokenKind::Comment) {
            self.pending = split_comment(self.text.clone(), &token, checkpoint);
            self.pending.pop_front()
        } else {
            Some((token, checkpoint))
        }
    }
}

/// Splits a `Comment` token into potentially multiple `Comment` and `Marker`
/// tokens if the comment contains any content that needs to be highlighted.
/// The first split token inherits the original token's checkpoint.
fn split_comment(
    text: Rope,
    token: &Token,
    checkpoint: Checkpoint,
) -> VecDeque<(Token, Checkpoint)> {
    assert_eq!(
        token.kind(),
        TokenKind::Comment,
        "`split_comment` must only be called with comment tokens"
    );

    let mut result = VecDeque::new();

    let mut source = Source::new(text, token.start());

    let mut comment_start = source.position;
    let mut next_checkpoint = checkpoint;

    while let Some(ch) = source.next_char()
        && source.position < token.end()
    {
        match ch {
            'T' => {
                let start = source.position;

                source.assert('T');
                if source.eat_if('O')
                    && source.eat_if('D')
                    && source.eat_if('O')
                    && source.eat_if(':')
                {
                    if comment_start < start {
                        result.push_back((
                            Token {
                                kind: TokenKind::Comment,
                                range: comment_start..start,
                            },
                            next_checkpoint,
                        ));
                        next_checkpoint = Checkpoint::No;
                    }

                    result.push_back((
                        Token {
                            kind: TokenKind::Marker,
                            range: start..source.position,
                        },
                        next_checkpoint,
                    ));
                    next_checkpoint = Checkpoint::No;

                    comment_start = source.position;
                }
            }
            'N' => {
                let start = source.position;

                source.assert('N');
                if source.eat_if('O')
                    && source.eat_if('T')
                    && source.eat_if('E')
                    && source.eat_if(':')
                {
                    if comment_start < start {
                        result.push_back((
                            Token {
                                kind: TokenKind::Comment,
                                range: comment_start..start,
                            },
                            next_checkpoint,
                        ));
                        next_checkpoint = Checkpoint::No;
                    }

                    result.push_back((
                        Token {
                            kind: TokenKind::Marker,
                            range: start..source.position,
                        },
                        next_checkpoint,
                    ));
                    next_checkpoint = Checkpoint::No;

                    comment_start = source.position;
                }
            }
            _ => {}
        }
    }

    if comment_start < source.position {
        result.push_back((
            Token {
                kind: TokenKind::Comment,
                range: comment_start..source.position,
            },
            next_checkpoint,
        ));
    }

    result
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
    Marker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_comment_works() {
        let source = Rope::from_str("// TODO: hello, NOTE: hi");

        let expected = Vec::from([
            (
                Token {
                    kind: TokenKind::Comment,
                    range: ByteIndex::new(0)..ByteIndex::new(3),
                },
                Checkpoint::Yes,
            ),
            (
                Token {
                    kind: TokenKind::Marker,
                    range: ByteIndex::new(3)..ByteIndex::new(8),
                },
                Checkpoint::No,
            ),
            (
                Token {
                    kind: TokenKind::Comment,
                    range: ByteIndex::new(8)..ByteIndex::new(16),
                },
                Checkpoint::No,
            ),
            (
                Token {
                    kind: TokenKind::Marker,
                    range: ByteIndex::new(16)..ByteIndex::new(21),
                },
                Checkpoint::No,
            ),
            (
                Token {
                    kind: TokenKind::Comment,
                    range: ByteIndex::new(21)..ByteIndex::new(24),
                },
                Checkpoint::No,
            ),
        ]);

        assert_eq!(
            Highlighter::new(source, ByteIndex::new(0), Language::Rust).collect::<Vec<_>>(),
            expected
        );
    }
}
