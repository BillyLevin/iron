use std::{
    assert_matches,
    ops::Range,
};

use ropey::RopeSlice;

use crate::{
    language::Language,
    text::ByteIndex,
};

#[derive(Debug)]
pub(crate) struct Highlighter<'src> {
    lexer: Lexer<'src>,
}

impl<'src> Highlighter<'src> {
    pub(crate) fn new(source: RopeSlice<'src>, start: ByteIndex, language: Language) -> Self {
        Self {
            lexer: match language {
                Language::Rust => Lexer::Rust(RustLexer::new(source, start)),
                Language::Toml | Language::Text => Lexer::Default,
            },
        }
    }
}

impl Iterator for Highlighter<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        self.lexer.next_token()
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
}

#[derive(Debug)]
struct RustLexer<'src> {
    source: RopeSlice<'src>,
    current_position: ByteIndex,
    current: Option<char>,

    /// If the current token being read could impact the semantics of the next
    /// token, then this map describes the transformation from `[0]` to `[1]` of
    /// the token kind that should take place.
    expected_token_map: Option<[TokenKind; 2]>,
}

impl<'src> RustLexer<'src> {
    fn new(source: RopeSlice<'src>, start: ByteIndex) -> Self {
        let current = source.get_char(start.value()).ok();

        Self {
            source,
            current_position: start,
            current,
            expected_token_map: None,
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        let start = self.current_position;

        self.current_char()
            .map(|ch| {
                match ch {
                    'a'..='z' | '_' => self.read_identifier_or_macro(),
                    '0'..='9' => self.read_number(),
                    c if c.is_whitespace() => self.read_whitespace(),
                    '"' => self.read_string(),
                    'A'..='Z' => self.read_type(),
                    '/' => self.read_slash(),
                    '\'' => self.read_single_quote(),
                    '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' => {
                        self.next_char();
                        TokenKind::Punctuation
                    }
                    ':' => {
                        self.next_char();

                        if self.current_char() == Some(':') {
                            self.next_char();
                        }

                        TokenKind::Punctuation
                    }
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
                                TokenKind::Operator
                            }

                            Some('<') => {
                                self.next_char();

                                if self.current_char() == Some('=') {
                                    self.next_char();
                                }

                                TokenKind::Operator
                            }
                            _ => TokenKind::Punctuation,
                        }
                    }
                    '>' => {
                        self.next_char();

                        match self.current_char() {
                            Some('=') => {
                                self.next_char();
                                TokenKind::Operator
                            }

                            Some('>') => {
                                self.next_char();

                                if self.current_char() == Some('=') {
                                    self.next_char();
                                }

                                TokenKind::Operator
                            }
                            _ => TokenKind::Punctuation,
                        }
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
                            TokenKind::Punctuation
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
                let kind = self.check_expected_mapping(kind);
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

    fn read_identifier_or_macro(&mut self) -> TokenKind {
        while let Some(ch) = self.current_char()
            && matches!(ch, 'a'..='z' | 'A'..='Z' | '_' | '0'..='9')
        {
            self.next_char();
        }

        if self.current_char() == Some('!') && self.peek_char() != Some('=') {
            self.next_char();
            TokenKind::Macro
        } else {
            TokenKind::Identifier
        }
    }

    fn read_whitespace(&mut self) -> TokenKind {
        while let Some(ch) = self.current_char()
            && ch.is_whitespace()
        {
            self.next_char();
        }

        TokenKind::Whitespace
    }

    fn check_keyword(&mut self, range: Range<ByteIndex>, kind: TokenKind) -> TokenKind {
        if matches!(kind, TokenKind::Identifier) {
            match self
                .source
                .slice(range.start.value()..range.end.value())
                .bytes()
                .collect::<Vec<u8>>()
                .as_slice()
            {
                b"_" | b"as" | b"async" | b"await" | b"break" | b"const" | b"continue"
                | b"crate" | b"dyn" | b"else" | b"enum" | b"extern" | b"false" | b"for" | b"if"
                | b"impl" | b"in" | b"let" | b"loop" | b"match" | b"mod" | b"move" | b"mut"
                | b"pub" | b"ref" | b"return" | b"self" | b"Self" | b"static" | b"struct"
                | b"super" | b"trait" | b"true" | b"type" | b"unsafe" | b"use" | b"where"
                | b"while" => TokenKind::Keyword,
                b"fn" => {
                    // TODO: probably a better place to do this mutation
                    self.expected_token_map =
                        Some([TokenKind::Identifier, TokenKind::FunctionName]);

                    TokenKind::Keyword
                }
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
        assert_matches!(
            self.current_char(),
            Some('A'..='Z'),
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
        assert_matches!(
            self.current_char(),
            Some('/'),
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
            self.eat_if('\''),
            "`read_single_quote` must only be called when the current character is a single quote"
        );

        let is_maybe_lifetime = if self.peek_char() == Some('\'') {
            false
        } else {
            matches!(self.current_char(), Some('a'..='z' | '_'))
        };

        if !is_maybe_lifetime {
            while let Some(ch) = self.current_char() {
                match ch {
                    '\'' => {
                        self.next_char();
                        return TokenKind::Character;
                    }
                    // escaping something; let's skip the slash and the next char
                    '\\' => {
                        self.next_char();
                        self.next_char();
                    }
                    _ => {
                        self.next_char();
                    }
                }
            }
        }

        // we'll assume it's a lifetime
        // TODO: it could also be intended as a string but they accidentally put it in
        // single quotes: do we want to highlight that differently?
        while let Some(ch) = self.current_char()
            && matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_')
        {
            self.next_char();
        }

        TokenKind::Lifetime
    }

    /// If the token kind has an expected semantic mapping, then we apply it
    /// here.
    fn check_expected_mapping(&mut self, kind: TokenKind) -> TokenKind {
        if kind == TokenKind::Whitespace {
            return kind;
        }

        let token_map = self.expected_token_map.take();

        if let Some(map) = token_map
            && map[0] == kind
        {
            map[1]
        } else {
            kind
        }
    }

    fn read_number(&mut self) -> TokenKind {
        while let Some(ch) = self.current_char()
        // TODO: obviously not correct but i'll stick with the easy approach until i find
        // a case where this breaks!
            && matches!(ch, 'a'..='z' | 'A'..='Z' | '_' | '0'..='9' | '.' )
        {
            self.next_char();
        }

        TokenKind::Number
    }
}

#[cfg(test)]
mod tests {
    use ropey::Rope;

    use super::*;

    #[test]
    fn identifiers() {
        let source = Rope::from_str("foo bar");
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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
        let source = Rope::from_str("use foo impl bar");
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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
                range: ByteIndex::new(8)..ByteIndex::new(12)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(12)..ByteIndex::new(13)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(13)..ByteIndex::new(16)
            })
        );
    }

    #[test]
    fn strings() {
        let source = Rope::from_str(r#"foo "hello""#);
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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
              "->", "=>", "<=", "=", "==", "!",
              "!=", "%", "%=", "&", "&=", "&&", "|",
              "|=", "||", "^", "^=", "*", "*=", "-",
              "-=", "+", "+=", "/", "/=",
              ">=", ">>", "<<", ">>=", "<<=", "@",
              "..", "..=",
        ];

        let source = Rope::from_str(&operators.join(" "));
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

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

    #[test]
    fn long_chars() {
        let source = Rope::from_str("'\\''");
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Character,
                range: ByteIndex::new(0)..ByteIndex::new(4)
            })
        );
    }

    #[test]
    fn lifetimes() {
        let source = Rope::from_str("impl<'src> Highlighter<'src>");
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Keyword,
                range: ByteIndex::new(0)..ByteIndex::new(4)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Punctuation,
                range: ByteIndex::new(4)..ByteIndex::new(5)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Lifetime,
                range: ByteIndex::new(5)..ByteIndex::new(9)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Punctuation,
                range: ByteIndex::new(9)..ByteIndex::new(10)
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
                kind: TokenKind::Type,
                range: ByteIndex::new(11)..ByteIndex::new(22)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Punctuation,
                range: ByteIndex::new(22)..ByteIndex::new(23)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Lifetime,
                range: ByteIndex::new(23)..ByteIndex::new(27)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Punctuation,
                range: ByteIndex::new(27)..ByteIndex::new(28)
            })
        );
    }

    #[test]
    fn function_name() {
        let source = Rope::from_str("fn hello");
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Keyword,
                range: ByteIndex::new(0)..ByteIndex::new(2)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(2)..ByteIndex::new(3)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::FunctionName,
                range: ByteIndex::new(3)..ByteIndex::new(8)
            })
        );
    }

    #[test]
    fn punctuation() {
        let punctuation = [
            "(", ")", "[", "]", "{", "}", "<", ">", "<", ">", "::", ":", ".", ",", ";",
        ];
        let source = Rope::from_str(&punctuation.join(" "));
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

        let mut position = ByteIndex::new(0);
        let mut punctuation_iter = punctuation.iter().peekable();

        while let Some(symbol) = punctuation_iter.next() {
            let is_last = punctuation_iter.peek().is_none();

            assert_eq!(
                lexer.next_token(),
                Some(Token {
                    kind: TokenKind::Punctuation,
                    range: position..position + symbol.len()
                })
            );

            position += symbol.len();

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
    fn ints() {
        let source = Rope::from_str("123 45 6_usize");
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Number,
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
                kind: TokenKind::Number,
                range: ByteIndex::new(4)..ByteIndex::new(6)
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
                kind: TokenKind::Number,
                range: ByteIndex::new(7)..ByteIndex::new(14)
            })
        );
    }

    #[test]
    fn floats() {
        let source = Rope::from_str("123.45_f32 45.03 6_700.67_f64");
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Number,
                range: ByteIndex::new(0)..ByteIndex::new(10)
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
                kind: TokenKind::Number,
                range: ByteIndex::new(11)..ByteIndex::new(16)
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
                kind: TokenKind::Number,
                range: ByteIndex::new(17)..ByteIndex::new(29)
            })
        );
    }

    #[test]
    fn macros() {
        let source = Rope::from_str("foo! foo!=bar");
        let mut lexer = RustLexer::new(source.slice(..), ByteIndex::new(0));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Macro,
                range: ByteIndex::new(0)..ByteIndex::new(4)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(4)..ByteIndex::new(5)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(5)..ByteIndex::new(8)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Operator,
                range: ByteIndex::new(8)..ByteIndex::new(10)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Identifier,
                range: ByteIndex::new(10)..ByteIndex::new(13)
            })
        );
    }
}
