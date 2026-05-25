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
    Type,
    Comment,
    Operator,
    Unknown,
    Character,
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
                    'A'..='Z' => self.read_type(),
                    '/' => self.read_slash(),
                    '\'' => self.read_single_quote(),
                    '@' => {
                        self.next_char();
                        TokenKind::Operator
                    }
                    '-' => {
                        self.next_char();

                        if let Some('>' | '=') = self.current_char() {
                            self.next_char();
                        }

                        TokenKind::Operator
                    }
                    '=' => {
                        self.next_char();

                        if let Some('>' | '=') = self.current_char() {
                            self.next_char();
                        }

                        TokenKind::Operator
                    }
                    '<' => {
                        self.next_char();

                        match self.current_char() {
                            Some('=') => {
                                self.next_char();
                            }

                            Some('<') => {
                                self.next_char();

                                if self.current_char() == Some('=') {
                                    self.next_char();
                                }
                            }
                            _ => {}
                        }

                        TokenKind::Operator
                    }
                    '>' => {
                        self.next_char();

                        match self.current_char() {
                            Some('=') => {
                                self.next_char();
                            }

                            Some('>') => {
                                self.next_char();

                                if self.current_char() == Some('=') {
                                    self.next_char();
                                }
                            }
                            _ => {}
                        }

                        TokenKind::Operator
                    }
                    '!' | '%' | '^' | '*' | '+' => {
                        self.next_char();

                        if self.current_char() == Some('=') {
                            self.next_char();
                        }

                        TokenKind::Operator
                    }
                    '&' => {
                        self.next_char();

                        if let Some('&' | '=') = self.current_char() {
                            self.next_char();
                        }

                        TokenKind::Operator
                    }
                    '|' => {
                        self.next_char();

                        if let Some('|' | '=') = self.current_char() {
                            self.next_char();
                        }

                        TokenKind::Operator
                    }
                    '.' => {
                        self.next_char();

                        if self.current_char() == Some('.') {
                            self.next_char();

                            if self.current_char() == Some('=') {
                                self.next_char();
                            }

                            TokenKind::Operator
                        } else {
                            TokenKind::Unknown
                        }
                    }
                    _ => {
                        // TODO: everything else!
                        self.next_char();
                        TokenKind::Unknown
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

    fn peek_char(&self) -> Option<char> {
        let position = self.current_position + self.current_char()?.len_utf8();

        self.source.get_char(position.value()).ok()
    }

    fn next_char(&mut self) -> Option<char> {
        let current_ch = self.current_char()?;
        let next_position = self.current_position + current_ch.len_utf8();

        self.current_position = next_position;

        self.current = self.source.get_char(next_position.value()).ok();
        self.current
    }

    /// Optionally eats the current char if it matches the given `ch`. Returns
    /// `true` if it ate the char.
    fn eat_if(&mut self, ch: char) -> bool {
        if self.current_char() == Some(ch) {
            self.next_char();
            true
        } else {
            false
        }
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

    fn read_type(&mut self) -> TokenKind {
        assert!(
            matches!(self.current_char(), Some('A'..='Z')),
            "types must start with a capital letter"
        );
        self.next_char();

        while let Some(ch) = self.current_char()
            && matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_')
        {
            self.next_char();
        }

        TokenKind::Type
    }

    fn read_slash(&mut self) -> TokenKind {
        assert!(
            matches!(self.current_char(), Some('/')),
            "`read_slash` must only be called when the current character is a slash"
        );
        self.next_char();

        match self.current_char() {
            // line/doc comment
            Some('/') => {
                self.next_char();

                if matches!(self.current_char(), Some('/')) {
                    self.next_char();
                }

                while let Some(ch) = self.current_char()
                    && ch != '\n'
                {
                    self.next_char();
                }

                TokenKind::Comment
            }
            // block comment
            Some('*') => {
                self.next_char();

                while let Some(ch) = self.current_char() {
                    let next = self.next_char();

                    if ch == '*' && matches!(next, Some('/')) {
                        self.next_char();
                        break;
                    }
                }

                TokenKind::Comment
            }

            // division assignment
            Some('=') => {
                self.next_char();
                TokenKind::Operator
            }

            Some(_) | None => TokenKind::Operator,
        }
    }

    fn read_single_quote(&mut self) -> TokenKind {
        assert!(
            matches!(self.current_char(), Some('\'')),
            "`read_single_quote` must only be called when the current character is a single quote"
        );
        self.next_char();

        let is_simple_char = self.peek_char() == Some('\'');

        if is_simple_char {
            self.next_char();
            assert!(
                self.eat_if('\''),
                "we verified above that `\\` is the current character"
            );
            TokenKind::Character
        } else {
            // TODO: could be:
            // - a lifetime
            // - a longer char literal (e.g. something like '\n')
            // - a char that was accidentally not terminated
            // - a string that was accidentally declared with single quotes
            // decide how/if we want to handle these cases especially.
            TokenKind::Operator
        }
    }
}

#[cfg(test)]
mod tests {
    use ropey::Rope;

    use super::*;

    #[test]
    fn identifiers() {
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
    fn keywords() {
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
    fn strings() {
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

    #[test]
    fn types() {
        let source = Rope::from_str("struct Foo");
        let mut lexer = RustLexer::new(source.slice(..));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Keyword,
                range: ByteIndex::new(0)..ByteIndex::new(6)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(6)..ByteIndex::new(7)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Type,
                range: ByteIndex::new(7)..ByteIndex::new(10)
            })
        );
    }

    #[test]
    fn comments() {
        let source = Rope::from_str(
            "// hello
// hi
use foo",
        );
        let mut lexer = RustLexer::new(source.slice(..));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Comment,
                range: ByteIndex::new(0)..ByteIndex::new(8)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(8)..ByteIndex::new(9)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Comment,
                range: ByteIndex::new(9)..ByteIndex::new(14)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(14)..ByteIndex::new(15)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Keyword,
                range: ByteIndex::new(15)..ByteIndex::new(18)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(18)..ByteIndex::new(19)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(19)..ByteIndex::new(22)
            })
        );
    }

    #[test]
    fn doc_comments() {
        let source = Rope::from_str(
            "/// hello
/// hi
use foo",
        );
        let mut lexer = RustLexer::new(source.slice(..));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Comment,
                range: ByteIndex::new(0)..ByteIndex::new(9)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(9)..ByteIndex::new(10)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Comment,
                range: ByteIndex::new(10)..ByteIndex::new(16)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(16)..ByteIndex::new(17)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Keyword,
                range: ByteIndex::new(17)..ByteIndex::new(20)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(20)..ByteIndex::new(21)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(21)..ByteIndex::new(24)
            })
        );
    }

    #[test]
    fn block_comments() {
        let source = Rope::from_str("use /* foo */ bar");
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
                kind: TokenKind::Comment,
                range: ByteIndex::new(4)..ByteIndex::new(13)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(13)..ByteIndex::new(14)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(14)..ByteIndex::new(17)
            })
        );
    }

    #[test]
    fn operators() {
        #[rustfmt::skip]
        let operators =  [
              "'", "->", "=>", "<=", "=", "==", "!",
              "!=", "%", "%=", "&", "&=", "&&", "|",
              "|=", "||", "^", "^=", "*", "*=", "-",
              "-=", "+", "+=", "/", "/=", ">", "<",
              ">=", ">>", "<<", ">>=", "<<=", "@",
              "..", "..=",
        ];

        let source = Rope::from_str(&operators.join(" "));
        let mut lexer = RustLexer::new(source.slice(..));

        let mut position = ByteIndex::new(0);

        let mut operators_iter = operators.iter().peekable();

        while let Some(operator) = operators_iter.next() {
            let is_last = operators_iter.peek().is_none();

            assert_eq!(
                lexer.next_token(),
                Some(Token {
                    kind: TokenKind::Operator,
                    range: position..position + operator.len()
                })
            );

            position += operator.len();

            if !is_last {
                assert_eq!(
                    lexer.next_token(),
                    Some(Token {
                        kind: TokenKind::Whitespace,
                        range: position..position + 1
                    })
                );
                position += 1;
            }
        }
    }

    #[test]
    fn chars() {
        let source = Rope::from_str("'h' 'i'");
        let mut lexer = RustLexer::new(source.slice(..));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Character,
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
                kind: TokenKind::Character,
                range: ByteIndex::new(4)..ByteIndex::new(7)
            })
        );
    }
}
