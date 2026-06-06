use std::ops::Range;

use ropey::Rope;

use crate::{
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
    type Item = Token;

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
    fn next_token(&mut self) -> Option<Token> {
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
}

#[derive(Debug)]
struct RustLexer {
    source: Rope,
    position: ByteIndex,
    current: Option<char>,

    /// If the current token being read could impact the semantics of the next
    /// token, then this map describes the transformation from `[0]` to `[1]` of
    /// the token kind that should take place.
    expected_token_map: Option<[TokenKind; 2]>,
}

impl RustLexer {
    fn new(source: Rope, start: ByteIndex) -> Self {
        let current = source.get_char(start.value()).ok();

        Self {
            source,
            position: start,
            current,
            expected_token_map: None,
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        self.current.map(|ch| self.read_token(ch))
    }

    fn read_token(&mut self, ch: char) -> Token {
        let start = self.position;

        let kind = match ch {
            c if c.is_whitespace() => self.read_whitespace(),
            '_' => self.read_underscore(),
            'a'..='z' => self.read_lowercase_ident(),
            'A'..='Z' => self.read_uppercase_ident(),
            '0'..='9' => self.read_number(),
            '"' => self.read_string(),
            '/' => self.read_slash(),
            '-' => self.read_dash(),
            '=' => self.read_equals(),
            '<' => self.read_less_than(),
            '>' => self.read_greater_than(),
            '!' => self.read_bang(),
            '%' => self.read_percent(),
            '&' => self.read_and(),
            '|' => self.read_or(),
            '^' => self.read_caret(),
            '*' => self.read_star(),
            '+' => self.read_plus(),
            '@' => self.read_at(),
            '.' => self.read_dot(),
            '\'' => self.read_apostrophe(),
            ':' => self.read_colon(),
            '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' => {
                self.next_char();
                TokenKind::Punctuation
            }
            _ => {
                self.next_char();
                TokenKind::Unknown
            }
        };

        let range = start..self.position;
        let kind = self.check_expected_mapping(kind);
        let kind = self.check_keyword(kind, &range);

        Token { kind, range }
    }

    fn next_char(&mut self) -> Option<char> {
        self.position += self.current?.len_utf8();
        self.current = self.source.get_char(self.position.value()).ok();
        self.current
    }

    fn peek(&self) -> Option<char> {
        self.source
            .get_char((self.position + self.current?.len_utf8()).value())
            .ok()
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

    fn read_whitespace(&mut self) -> TokenKind {
        self.eat_while(char::is_whitespace);
        TokenKind::Whitespace
    }

    fn read_underscore(&mut self) -> TokenKind {
        self.eat_while(|ch| matches!(ch, '_' | '0'..='9'));

        match self.current {
            Some(ch) if ch.is_ascii_uppercase() => self.read_uppercase_ident(),
            Some(_) | None => self.read_lowercase_ident(),
        }
    }

    fn read_lowercase_ident(&mut self) -> TokenKind {
        self.eat_while(|ch| matches!(ch, 'a'..='z' | '_' | '0'..='9' ));

        if self.current == Some('!') && self.peek() != Some('=') {
            self.assert('!');
            TokenKind::Macro
        } else if self.current == Some(':') && self.peek() != Some(':') {
            self.assert(':');
            TokenKind::Property
        } else {
            TokenKind::Identifier
        }
    }

    fn read_uppercase_ident(&mut self) -> TokenKind {
        self.eat_while(|ch| ch.is_ascii_alphanumeric() || ch == '_');

        TokenKind::Type
    }

    fn read_number(&mut self) -> TokenKind {
        // NOTE: not technically correct to allow all letters but i haven't found a case
        // where this causes anything weird to happen with the highlights
        self.eat_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.'));

        TokenKind::Number
    }

    fn read_string(&mut self) -> TokenKind {
        self.assert('"');

        let mut is_escaped = false;
        self.eat_while(|ch| {
            match ch {
                '"' if !is_escaped => false,
                '\\' => {
                    is_escaped = !is_escaped;
                    true
                }
                _ => {
                    is_escaped = false;
                    true
                }
            }
        });

        // we don't `self.assert('"')` here because the string may have just never been
        // closed
        self.next_char();

        TokenKind::String
    }

    fn read_slash(&mut self) -> TokenKind {
        self.assert('/');

        match self.current {
            // line/doc comment
            Some('/') => {
                self.eat_while(|ch| ch != '\n');
                TokenKind::Comment
            }
            // block comment
            Some('*') => {
                self.assert('*');

                while let Some(ch) = self.current {
                    let next = self.next_char();

                    if ch == '*' && matches!(next, Some('/')) {
                        self.assert('/');
                        break;
                    }
                }

                TokenKind::Comment
            }

            // division assignment
            Some('=') => {
                self.assert('=');
                TokenKind::Operator
            }

            Some(_) | None => TokenKind::Operator,
        }
    }

    fn read_dash(&mut self) -> TokenKind {
        self.assert('-');

        if let Some(ch @ ('>' | '=')) = self.current {
            self.assert(ch);
        }

        TokenKind::Operator
    }

    fn read_equals(&mut self) -> TokenKind {
        self.assert('=');

        if let Some(ch @ ('>' | '=')) = self.current {
            self.assert(ch);
        }

        TokenKind::Operator
    }

    fn read_less_than(&mut self) -> TokenKind {
        self.assert('<');

        match self.current {
            Some('=') => self.assert('='),
            Some('<') => {
                self.assert('<');
                self.eat_if('=');
            }
            Some(_) | None => {}
        }

        TokenKind::Operator
    }

    fn read_greater_than(&mut self) -> TokenKind {
        self.assert('>');

        match self.current {
            Some('=') => self.assert('='),
            Some('>') => {
                self.assert('>');
                self.eat_if('=');
            }
            Some(_) | None => {}
        }

        TokenKind::Operator
    }

    fn read_bang(&mut self) -> TokenKind {
        self.assert('!');
        self.eat_if('=');

        TokenKind::Operator
    }

    fn read_percent(&mut self) -> TokenKind {
        self.assert('%');
        self.eat_if('=');

        TokenKind::Operator
    }

    fn read_and(&mut self) -> TokenKind {
        self.assert('&');

        if let Some(ch @ ('&' | '=')) = self.current {
            self.assert(ch);
        }

        TokenKind::Operator
    }

    fn read_or(&mut self) -> TokenKind {
        self.assert('|');

        if let Some(ch @ ('|' | '=')) = self.current {
            self.assert(ch);
        }

        TokenKind::Operator
    }

    fn read_caret(&mut self) -> TokenKind {
        self.assert('^');
        self.eat_if('=');

        TokenKind::Operator
    }

    fn read_star(&mut self) -> TokenKind {
        self.assert('*');
        self.eat_if('=');

        TokenKind::Operator
    }

    fn read_plus(&mut self) -> TokenKind {
        self.assert('+');
        self.eat_if('=');

        TokenKind::Operator
    }

    fn read_at(&mut self) -> TokenKind {
        self.assert('@');

        TokenKind::Operator
    }

    fn read_dot(&mut self) -> TokenKind {
        self.assert('.');

        if self.eat_if('.') {
            self.eat_if('=');
            TokenKind::Operator
        } else {
            TokenKind::Punctuation
        }
    }

    fn read_apostrophe(&mut self) -> TokenKind {
        self.assert('\'');

        let is_maybe_lifetime = if self.peek() == Some('\'') {
            false
        } else {
            matches!(self.current, Some('a'..='z' | '_'))
        };

        if !is_maybe_lifetime {
            while let Some(ch) = self.current {
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
        self.eat_while(|ch| ch.is_ascii_alphanumeric() || ch == '_');

        TokenKind::Lifetime
    }

    fn read_colon(&mut self) -> TokenKind {
        self.assert(':');
        self.eat_if(':');

        TokenKind::Punctuation
    }

    fn check_keyword(&mut self, kind: TokenKind, range: &Range<ByteIndex>) -> TokenKind {
        if !matches!(kind, TokenKind::Identifier | TokenKind::Type) {
            return kind;
        }

        assert!(
            self.position >= range.start,
            "`position` should never decrement"
        );
        let bytes: Vec<u8> = self
            .source
            .bytes_at(range.start.value())
            .take((self.position - range.start).value())
            .collect();

        match bytes.as_slice() {
            b"_" | b"as" | b"async" | b"await" | b"break" | b"const" | b"continue" | b"crate"
            | b"dyn" | b"else" | b"enum" | b"extern" | b"false" | b"for" | b"if" | b"impl"
            | b"in" | b"let" | b"loop" | b"match" | b"mod" | b"move" | b"mut" | b"pub" | b"ref"
            | b"return" | b"self" | b"Self" | b"static" | b"struct" | b"super" | b"trait"
            | b"true" | b"type" | b"unsafe" | b"use" | b"where" | b"while" => TokenKind::Keyword,
            b"fn" => {
                // TODO: probably a better place to do this mutation
                self.expected_token_map = Some([TokenKind::Identifier, TokenKind::FunctionName]);

                TokenKind::Keyword
            }

            _ => kind,
        }
    }

    /// If the token kind has an expected semantic mapping, then we apply it
    /// here.
    fn check_expected_mapping(&mut self, kind: TokenKind) -> TokenKind {
        // we "skip" whitespace
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
}

#[cfg(test)]
mod tests {
    use ropey::Rope;

    use super::*;

    #[test]
    fn identifiers() {
        let source = Rope::from_str("foo bar __12baz");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
                kind: TokenKind::Identifier,
                range: ByteIndex::new(8)..ByteIndex::new(15)
            })
        );
    }

    #[test]
    fn keywords() {
        let source = Rope::from_str("use foo impl bar");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let source = Rope::from_str(r#"foo "hello" "h\"i""#);
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let source = Rope::from_str("struct Foo struct __12Foo");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
                kind: TokenKind::Keyword,
                range: ByteIndex::new(11)..ByteIndex::new(17)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(17)..ByteIndex::new(18)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Type,
                range: ByteIndex::new(18)..ByteIndex::new(25)
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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
              "-=", "+", "+=", "/", "/=", "<", ">",
              ">=", ">>", "<<", ">>=", "<<=", "@",
              "..", "..=",
        ];

        let source = Rope::from_str(&operators.join(" "));
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        let mut position = ByteIndex::new(0);

        let mut operators_iter = operators.iter().peekable();

        while let Some(operator) = operators_iter.next() {
            let is_last = operators_iter.peek().is_none();

            assert_eq!(
                lexer.next_token(),
                Some(Token {
                    kind: TokenKind::Operator,
                    range: position..position + operator.len()
                }),
                "operator `{operator}` read incorrectly"
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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
                kind: TokenKind::Operator,
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
                kind: TokenKind::Operator,
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
                kind: TokenKind::Operator,
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
                kind: TokenKind::Operator,
                range: ByteIndex::new(27)..ByteIndex::new(28)
            })
        );
    }

    #[test]
    fn function_name() {
        let source = Rope::from_str("fn hello");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let punctuation = ["(", ")", "[", "]", "{", "}", "::", ":", ".", ",", ";"];
        let source = Rope::from_str(&punctuation.join(" "));
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        let mut position = ByteIndex::new(0);
        let mut punctuation_iter = punctuation.iter().peekable();

        while let Some(symbol) = punctuation_iter.next() {
            let is_last = punctuation_iter.peek().is_none();

            assert_eq!(
                lexer.next_token(),
                Some(Token {
                    kind: TokenKind::Punctuation,
                    range: position..position + symbol.len()
                }),
                "symbol `{symbol}` read incorrectly"
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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

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

    #[test]
    fn properties() {
        let source = Rope::from_str("{ foo: bar }");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Punctuation,
                range: ByteIndex::new(0)..ByteIndex::new(1)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Whitespace,
                range: ByteIndex::new(1)..ByteIndex::new(2)
            })
        );

        assert_eq!(
            lexer.next_token(),
            Some(Token {
                kind: TokenKind::Property,
                range: ByteIndex::new(2)..ByteIndex::new(6)
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
                kind: TokenKind::Identifier,
                range: ByteIndex::new(7)..ByteIndex::new(10)
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
                kind: TokenKind::Punctuation,
                range: ByteIndex::new(11)..ByteIndex::new(12)
            })
        );
    }
}
