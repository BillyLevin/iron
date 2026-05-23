use std::ops::Range;

use ropey::RopeSlice;

use crate::{
    language::Language,
    text::ByteIndex,
};

#[derive(Debug)]
pub(crate) struct Highlighter<'src> {
    lexer: Lexer<'src>,
    current_token: Option<Token>,
}

impl<'src> Highlighter<'src> {
    pub(crate) fn new(source: RopeSlice<'src>, language: Language) -> Self {
        let mut lexer = match language {
            Language::Rust => Lexer::Rust(RustLexer::new(source)),
            Language::Toml | Language::Text => Lexer::Default,
        };

        let token = lexer.next_token();

        Self {
            lexer,
            current_token: token,
        }
    }

    pub(crate) fn advance_until(&mut self, index: ByteIndex) -> Option<&Token> {
        while let Some(ref token) = self.current_token
            && !token.range.contains(&index)
        {
            self.next();
        }

        self.current_token.as_ref()
    }

    fn next(&mut self) {
        self.current_token = self.lexer.next_token();
    }
}

#[derive(Debug)]
enum Lexer<'src> {
    Rust(RustLexer<'src>),
    Default,
}

impl Lexer<'_> {
    fn next_token(&mut self) -> Option<Token> {
        match *self {
            Lexer::Rust(ref mut lexer) => lexer.next_token(),
            Lexer::Default => None,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Identifier,
    Whitespace,
    Keyword,
    String,
}

#[derive(Debug)]
struct RustLexer<'src> {
    source: RopeSlice<'src>,
    current_position: ByteIndex,
    current: Option<char>,
}

impl<'src> RustLexer<'src> {
    fn new(source: RopeSlice<'src>) -> Self {
        let current = source.get_char(0).ok();

        Self {
            source,
            current_position: ByteIndex::new(0),
            current,
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        let start = self.current_position;

        self.current_char()
            .map(|ch| {
                match ch {
                    'a'..='z' | '_' => self.read_identifier(),
                    c if c.is_whitespace() => self.read_whitespace(),
                    '"' => self.read_string(),
                    _ => {
                        // TODO: everything else!
                        self.next_char();
                        TokenKind::Whitespace
                    }
                }
            })
            .map(|kind| {
                let range = start..self.current_position;
                let kind = self.check_keyword(range, kind);

                Token {
                    kind,
                    range: start..self.current_position,
                }
            })
    }

    const fn current_char(&self) -> Option<char> {
        self.current
    }

    fn next_char(&mut self) -> Option<char> {
        let current_ch = self.current_char()?;
        let next_position = self.current_position + current_ch.len_utf8();

        self.current_position = next_position;

        self.current = self.source.get_char(next_position.value()).ok();
        self.current
    }

    fn read_identifier(&mut self) -> TokenKind {
        while let Some(ch) = self.current_char()
            && matches!(ch, 'a'..='z' | 'A'..='Z' | '_' | '0'..='9')
        {
            self.next_char();
        }

        TokenKind::Identifier
    }

    fn read_whitespace(&mut self) -> TokenKind {
        while let Some(ch) = self.current_char()
            && ch.is_whitespace()
        {
            self.next_char();
        }

        TokenKind::Whitespace
    }

    fn check_keyword(&self, range: Range<ByteIndex>, kind: TokenKind) -> TokenKind {
        if matches!(kind, TokenKind::Identifier) {
            match self
                .source
                .slice(range.start.value()..range.end.value())
                .bytes()
                .collect::<Vec<u8>>()
                .as_slice()
            {
                b"_" | b"as" | b"async" | b"await" | b"break" | b"const" | b"continue"
                | b"crate" | b"dyn" | b"else" | b"enum" | b"extern" | b"false" | b"fn" | b"for"
                | b"if" | b"impl" | b"in" | b"let" | b"loop" | b"match" | b"mod" | b"move"
                | b"mut" | b"pub" | b"ref" | b"return" | b"self" | b"Self" | b"static"
                | b"struct" | b"super" | b"trait" | b"true" | b"type" | b"unsafe" | b"use"
                | b"where" | b"while" => TokenKind::Keyword,
                _ => kind,
            }
        } else {
            kind
        }
    }

    fn read_string(&mut self) -> TokenKind {
        assert_eq!(
            self.current_char(),
            Some('"'),
            "`read_string` should only be called if the current character is the start of a string"
        );
        self.next_char();

        while let Some(ch) = self.current_char()
            && ch != '"'
        {
            self.next_char();
        }

        self.next_char();

        TokenKind::String
    }
}

#[cfg(test)]
mod tests {
    use ropey::Rope;

    use super::*;

    #[test]
    fn identifier() {
        let source = Rope::from_str("foo bar");
        let mut lexer = RustLexer::new(source.slice(..));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(0)..ByteIndex::new(3)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(3)..ByteIndex::new(4)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(4)..ByteIndex::new(7)
            })
        );
    }

    #[test]
    fn keyword() {
        let source = Rope::from_str("use foo fn bar");
        let mut lexer = RustLexer::new(source.slice(..));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Keyword,
                range: ByteIndex::new(0)..ByteIndex::new(3)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(3)..ByteIndex::new(4)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(4)..ByteIndex::new(7)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(7)..ByteIndex::new(8)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Keyword,
                range: ByteIndex::new(8)..ByteIndex::new(10)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(10)..ByteIndex::new(11)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(11)..ByteIndex::new(14)
            })
        );
    }

    #[test]
    fn string() {
        let source = Rope::from_str(r#"foo "hello""#);
        let mut lexer = RustLexer::new(source.slice(..));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(0)..ByteIndex::new(3)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(3)..ByteIndex::new(4)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::String,
                range: ByteIndex::new(4)..ByteIndex::new(11)
            })
        );
    }
}
