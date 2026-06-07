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
}

#[derive(Debug)]
struct RustLexer {
    source: Rope,
    position: ByteIndex,
    current: Option<char>,
    delimiter_stack: Vec<Delimiter>,
    last_significant: SignificantKind,
}

impl RustLexer {
    fn new(source: Rope, start: ByteIndex) -> Self {
        let current = source.get_char(start.value()).ok();

        Self {
            source,
            position: start,
            current,
            delimiter_stack: Vec::new(),
            last_significant: SignificantKind::Other,
        }
    }

    fn next_token(&mut self) -> Option<(Token, Checkpoint)> {
        self.current.map(|ch| self.read_token(ch))
    }

    fn read_token(&mut self, ch: char) -> (Token, Checkpoint) {
        let start = self.position;

        let kind = match ch {
            c if c.is_whitespace() => self.read_whitespace(),
            '_' => self.read_underscore(start),
            'a'..='z' => self.read_lowercase_ident(start),
            'A'..='Z' => self.read_uppercase_ident(start),
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

        let token = Token {
            kind,
            range: start..self.position,
        };

        let checkpoint_outcome = self.update_context(&token);

        (token, checkpoint_outcome)
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

    fn read_underscore(&mut self, start: ByteIndex) -> TokenKind {
        self.eat_while(|ch| matches!(ch, '_' | '0'..='9'));

        match self.current {
            Some(ch) if ch.is_ascii_uppercase() => self.read_uppercase_ident(start),
            Some(_) | None => self.read_lowercase_ident(start),
        }
    }

    fn read_lowercase_ident(&mut self, start: ByteIndex) -> TokenKind {
        self.eat_while(|ch| matches!(ch, 'a'..='z' | '_' | '0'..='9'));

        if self.current == Some('!') && self.peek() != Some('=') {
            self.assert('!');
            TokenKind::Macro
        } else if self.current == Some(':') && self.peek() != Some(':') {
            // TODO: shouldn't skip past it
            self.assert(':');
            TokenKind::Property
        } else if self.maybe_property() && matches!(self.next_non_whitespace(), Some(',' | '}')) {
            TokenKind::Property
        } else if self.is_keyword(start..self.position) {
            TokenKind::Keyword
        } else if self.current == Some('(') {
            TokenKind::FunctionName
        } else {
            TokenKind::Identifier
        }
    }

    fn read_uppercase_ident(&mut self, start: ByteIndex) -> TokenKind {
        self.eat_while(|ch| ch.is_ascii_alphanumeric() || ch == '_');

        if self.is_keyword(start..self.position) {
            TokenKind::Keyword
        } else {
            TokenKind::Type
        }
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

    fn is_keyword(&self, range: Range<ByteIndex>) -> bool {
        let bytes: Vec<u8> = self
            .source
            .bytes_at(range.start.value())
            .take((range.end - range.start).value())
            .collect();

        matches!(
            bytes.as_slice(),
            b"_" | b"as"
                | b"async"
                | b"await"
                | b"break"
                | b"const"
                | b"continue"
                | b"crate"
                | b"dyn"
                | b"else"
                | b"enum"
                | b"extern"
                | b"false"
                | b"fn"
                | b"for"
                | b"if"
                | b"impl"
                | b"in"
                | b"let"
                | b"loop"
                | b"match"
                | b"mod"
                | b"move"
                | b"mut"
                | b"pub"
                | b"ref"
                | b"return"
                | b"self"
                | b"Self"
                | b"static"
                | b"struct"
                | b"super"
                | b"trait"
                | b"true"
                | b"type"
                | b"unsafe"
                | b"use"
                | b"where"
                | b"while"
        )
    }

    fn update_context(&mut self, token: &Token) -> Checkpoint {
        let mut result = if token.range.start == ByteIndex::new(0) {
            Checkpoint::Yes
        } else {
            Checkpoint::No
        };

        match token.kind() {
            TokenKind::Punctuation => {
                match self.token_bytes(token).as_slice() {
                    b"{" => {
                        self.delimiter_stack.push(Delimiter::Brace);
                        self.last_significant = SignificantKind::OpenDelimiter(Delimiter::Brace);
                    }
                    b"[" => {
                        self.delimiter_stack.push(Delimiter::Bracket);
                        self.last_significant = SignificantKind::OpenDelimiter(Delimiter::Bracket);
                    }
                    b"(" => {
                        self.delimiter_stack.push(Delimiter::Paren);
                        self.last_significant = SignificantKind::OpenDelimiter(Delimiter::Paren);
                    }
                    b"}" | b"]" | b")" => {
                        self.delimiter_stack.pop();

                        if self.delimiter_stack.is_empty() {
                            result = Checkpoint::Yes;
                        }
                    }
                    b"," => {
                        self.last_significant = SignificantKind::Comma;
                    }
                    _ => {}
                }
            }
            TokenKind::Identifier
            | TokenKind::Keyword
            | TokenKind::String
            | TokenKind::Type
            | TokenKind::Operator
            | TokenKind::Unknown
            | TokenKind::Character
            | TokenKind::Lifetime
            | TokenKind::FunctionName
            | TokenKind::Number
            | TokenKind::Macro
            | TokenKind::Property => self.last_significant = SignificantKind::Other,

            TokenKind::Whitespace | TokenKind::Comment => {}
        }

        result
    }

    fn token_bytes(&self, token: &Token) -> Vec<u8> {
        let bytes: Vec<u8> = self
            .source
            .bytes_at(token.range.start.value())
            .take((token.range.end - token.range.start).value())
            .collect();

        bytes
    }

    fn in_braces(&self) -> bool {
        self.delimiter_stack.last().copied() == Some(Delimiter::Brace)
    }

    fn next_non_whitespace(&self) -> Option<char> {
        self.source
            .chars_at(self.position.value())
            .find(|ch| !ch.is_whitespace())
    }

    fn maybe_property(&self) -> bool {
        self.in_braces()
            && match self.last_significant {
                SignificantKind::Comma | SignificantKind::OpenDelimiter(Delimiter::Brace) => true,
                SignificantKind::OpenDelimiter(_) | SignificantKind::Other => false,
            }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delimiter {
    /// `(` or `)`.
    Paren,
    /// `{` or `}`.
    Brace,
    /// `[` or `]`.
    Bracket,
}

#[derive(Debug)]
enum SignificantKind {
    Comma,
    OpenDelimiter(Delimiter),
    Other,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Checkpoint {
    Yes,
    No,
}

#[cfg(test)]
mod tests {
    use ropey::Rope;

    use super::*;

    #[test]
    fn identifiers() {
        let source = Rope::from_str("foo bar __12baz");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(0)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(3)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(4)..ByteIndex::new(7)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(7)..ByteIndex::new(8)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(8)..ByteIndex::new(15)
        });
    }

    #[test]
    fn keywords() {
        let source = Rope::from_str("use foo impl bar");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(0)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(3)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(4)..ByteIndex::new(7)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(7)..ByteIndex::new(8)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(8)..ByteIndex::new(12)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(12)..ByteIndex::new(13)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(13)..ByteIndex::new(16)
        });
    }

    #[test]
    fn strings() {
        let source = Rope::from_str(r#"foo "hello" "h\"i""#);
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(0)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(3)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::String,
            range: ByteIndex::new(4)..ByteIndex::new(11)
        });
    }

    #[test]
    fn types() {
        let source = Rope::from_str("struct Foo struct __12Foo");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(0)..ByteIndex::new(6)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(6)..ByteIndex::new(7)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Type,
            range: ByteIndex::new(7)..ByteIndex::new(10)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(10)..ByteIndex::new(11)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(11)..ByteIndex::new(17)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(17)..ByteIndex::new(18)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Type,
            range: ByteIndex::new(18)..ByteIndex::new(25)
        });
    }

    #[test]
    fn comments() {
        let source = Rope::from_str(
            "// hello
// hi
use foo",
        );
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Comment,
            range: ByteIndex::new(0)..ByteIndex::new(8)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(8)..ByteIndex::new(9)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Comment,
            range: ByteIndex::new(9)..ByteIndex::new(14)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(14)..ByteIndex::new(15)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(15)..ByteIndex::new(18)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(18)..ByteIndex::new(19)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(19)..ByteIndex::new(22)
        });
    }

    #[test]
    fn doc_comments() {
        let source = Rope::from_str(
            "/// hello
/// hi
use foo",
        );
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Comment,
            range: ByteIndex::new(0)..ByteIndex::new(9)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(9)..ByteIndex::new(10)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Comment,
            range: ByteIndex::new(10)..ByteIndex::new(16)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(16)..ByteIndex::new(17)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(17)..ByteIndex::new(20)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(20)..ByteIndex::new(21)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(21)..ByteIndex::new(24)
        });
    }

    #[test]
    fn block_comments() {
        let source = Rope::from_str("use /* foo */ bar");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(0)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(3)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Comment,
            range: ByteIndex::new(4)..ByteIndex::new(13)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(13)..ByteIndex::new(14)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(14)..ByteIndex::new(17)
        });
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
                lexer.next_token().unwrap().0,
                Token {
                    kind: TokenKind::Operator,
                    range: position..position + operator.len()
                },
                "operator `{operator}` read incorrectly"
            );

            position += operator.len();

            if !is_last {
                assert_eq!(lexer.next_token().unwrap().0, Token {
                    kind: TokenKind::Whitespace,
                    range: position..position + 1
                });
                position += 1;
            }
        }
    }

    #[test]
    fn chars() {
        let source = Rope::from_str("'h' 'i'");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Character,
            range: ByteIndex::new(0)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(3)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Character,
            range: ByteIndex::new(4)..ByteIndex::new(7)
        });
    }

    #[test]
    fn long_chars() {
        let source = Rope::from_str("'\\''");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Character,
            range: ByteIndex::new(0)..ByteIndex::new(4)
        });
    }

    #[test]
    fn lifetimes() {
        let source = Rope::from_str("impl<'src> Highlighter<'src>");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(0)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Operator,
            range: ByteIndex::new(4)..ByteIndex::new(5)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Lifetime,
            range: ByteIndex::new(5)..ByteIndex::new(9)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Operator,
            range: ByteIndex::new(9)..ByteIndex::new(10)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(10)..ByteIndex::new(11)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Type,
            range: ByteIndex::new(11)..ByteIndex::new(22)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Operator,
            range: ByteIndex::new(22)..ByteIndex::new(23)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Lifetime,
            range: ByteIndex::new(23)..ByteIndex::new(27)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Operator,
            range: ByteIndex::new(27)..ByteIndex::new(28)
        });
    }

    #[test]
    fn function_name() {
        let source = Rope::from_str("fn hello()");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Keyword,
            range: ByteIndex::new(0)..ByteIndex::new(2)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(2)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::FunctionName,
            range: ByteIndex::new(3)..ByteIndex::new(8)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Punctuation,
            range: ByteIndex::new(8)..ByteIndex::new(9)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Punctuation,
            range: ByteIndex::new(9)..ByteIndex::new(10)
        });
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
                lexer.next_token().unwrap().0,
                Token {
                    kind: TokenKind::Punctuation,
                    range: position..position + symbol.len()
                },
                "symbol `{symbol}` read incorrectly"
            );

            position += symbol.len();

            if !is_last {
                assert_eq!(lexer.next_token().unwrap().0, Token {
                    kind: TokenKind::Whitespace,
                    range: position..position + 1
                });
                position += 1;
            }
        }
    }

    #[test]
    fn ints() {
        let source = Rope::from_str("123 45 6_usize");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Number,
            range: ByteIndex::new(0)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(3)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Number,
            range: ByteIndex::new(4)..ByteIndex::new(6)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(6)..ByteIndex::new(7)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Number,
            range: ByteIndex::new(7)..ByteIndex::new(14)
        });
    }

    #[test]
    fn floats() {
        let source = Rope::from_str("123.45_f32 45.03 6_700.67_f64");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Number,
            range: ByteIndex::new(0)..ByteIndex::new(10)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(10)..ByteIndex::new(11)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Number,
            range: ByteIndex::new(11)..ByteIndex::new(16)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(16)..ByteIndex::new(17)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Number,
            range: ByteIndex::new(17)..ByteIndex::new(29)
        });
    }

    #[test]
    fn macros() {
        let source = Rope::from_str("foo! foo!=bar");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Macro,
            range: ByteIndex::new(0)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(4)..ByteIndex::new(5)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(5)..ByteIndex::new(8)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Operator,
            range: ByteIndex::new(8)..ByteIndex::new(10)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(10)..ByteIndex::new(13)
        });
    }

    #[test]
    fn properties() {
        let source = Rope::from_str("{ foo: bar }");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Punctuation,
            range: ByteIndex::new(0)..ByteIndex::new(1)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(1)..ByteIndex::new(2)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Property,
            range: ByteIndex::new(2)..ByteIndex::new(6)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(6)..ByteIndex::new(7)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Identifier,
            range: ByteIndex::new(7)..ByteIndex::new(10)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(10)..ByteIndex::new(11)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Punctuation,
            range: ByteIndex::new(11)..ByteIndex::new(12)
        });
    }

    #[test]
    fn shorthand_properties() {
        let source = Rope::from_str("Foo  { foo }");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Type,
            range: ByteIndex::new(0)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(3)..ByteIndex::new(5)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Punctuation,
            range: ByteIndex::new(5)..ByteIndex::new(6)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(6)..ByteIndex::new(7)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Property,
            range: ByteIndex::new(7)..ByteIndex::new(10)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Whitespace,
            range: ByteIndex::new(10)..ByteIndex::new(11)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Punctuation,
            range: ByteIndex::new(11)..ByteIndex::new(12)
        });
    }

    #[test]
    fn function_call() {
        let source = Rope::from_str("foo()");
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::FunctionName,
            range: ByteIndex::new(0)..ByteIndex::new(3)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Punctuation,
            range: ByteIndex::new(3)..ByteIndex::new(4)
        });

        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind: TokenKind::Punctuation,
            range: ByteIndex::new(4)..ByteIndex::new(5)
        });
    }
}
