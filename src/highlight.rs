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
    PropertyAccess,
    Constant,
    EnumMember,
}

#[derive(Debug)]
struct RustLexer {
    source: Rope,
    position: ByteIndex,
    current: Option<char>,
    delimiter_stack: Vec<Delimiter>,
    last_significant: SignificantKind,
    in_use_declaration: bool,
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
            in_use_declaration: false,
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
            '#' => self.read_pound(),
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
        } else if self.is_property() {
            TokenKind::Property
        } else if self.is_keyword(start..self.position) {
            TokenKind::Keyword
        } else if self.current == Some('(') {
            if self.last_significant == SignificantKind::OpenAttribute {
                TokenKind::Macro
            } else {
                TokenKind::FunctionName
            }
        } else if self.last_significant == SignificantKind::Dot {
            TokenKind::PropertyAccess
        } else {
            TokenKind::Identifier
        }
    }

    fn read_uppercase_ident(&mut self, start: ByteIndex) -> TokenKind {
        let mut is_uppercase = true;

        self.eat_while(|ch| {
            let result = ch.is_ascii_alphanumeric() || ch == '_';
            if ch.is_ascii_lowercase() {
                is_uppercase = false;
            }

            result
        });

        if self.is_keyword(start..self.position) {
            TokenKind::Keyword
        } else if is_uppercase {
            TokenKind::Constant
        } else if self.is_enum_member() {
            TokenKind::EnumMember
        } else {
            TokenKind::Type
        }
    }

    fn read_number(&mut self) -> TokenKind {
        // TODO: could create `self.eat_while_with_peek` helper if this comes up again
        while let Some(ch) = self.current {
            // NOTE: not technically correct to allow all letters but i haven't found a case
            // where this causes anything weird to happen with the highlights
            if !(ch.is_ascii_alphanumeric() || ch == '_' || (ch == '.' && self.peek() != Some('.')))
            {
                break;
            }

            self.next_char();
        }

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

    fn read_pound(&mut self) -> TokenKind {
        self.assert('#');
        // opening an attribute - handled in `update_context`
        self.eat_if('[');

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
                    delim @ (b"}" | b"]" | b")") => {
                        self.delimiter_stack.pop();
                        self.last_significant = SignificantKind::CloseDelimiter(match delim {
                            b"}" => Delimiter::Brace,
                            b"]" => Delimiter::Bracket,
                            b")" => Delimiter::Paren,
                            _ => unreachable!(),
                        });

                        if self.delimiter_stack.is_empty() {
                            result = Checkpoint::Yes;
                        }
                    }
                    b"," => {
                        self.last_significant = SignificantKind::Comma;
                    }
                    b"." => {
                        self.last_significant = SignificantKind::Dot;
                    }
                    b":" => {
                        self.last_significant = SignificantKind::Colon;
                    }
                    b";" => {
                        self.in_use_declaration = false;
                        self.last_significant = SignificantKind::SemiColon;
                    }
                    b"#[" => {
                        self.last_significant = SignificantKind::OpenAttribute;
                    }
                    b"::" => {
                        self.last_significant = SignificantKind::PathSeparator;
                    }
                    _ => {}
                }
            }
            TokenKind::Keyword => {
                if matches!(self.token_bytes(token).as_slice(), b"use") {
                    self.in_use_declaration = true;
                }

                self.last_significant = SignificantKind::Keyword;
            }
            TokenKind::Identifier
            | TokenKind::String
            | TokenKind::Type
            | TokenKind::Operator
            | TokenKind::Unknown
            | TokenKind::Character
            | TokenKind::Lifetime
            | TokenKind::FunctionName
            | TokenKind::Number
            | TokenKind::Macro
            | TokenKind::Property
            | TokenKind::PropertyAccess
            | TokenKind::Constant
            | TokenKind::Whitespace
            | TokenKind::Comment
            | TokenKind::EnumMember => {}
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
                SignificantKind::Comma
                | SignificantKind::OpenDelimiter(Delimiter::Brace)
                | SignificantKind::CloseDelimiter(Delimiter::Paren)
                | SignificantKind::Keyword => true,
                SignificantKind::OpenDelimiter(_)
                | SignificantKind::CloseDelimiter(_)
                | SignificantKind::Dot
                | SignificantKind::Other
                | SignificantKind::Colon
                | SignificantKind::SemiColon
                | SignificantKind::OpenAttribute
                | SignificantKind::PathSeparator => false,
            }
    }

    fn is_property(&self) -> bool {
        if !self.maybe_property() {
            return false;
        }

        if self.current == Some(':') && self.peek() != Some(':') {
            return true;
        }

        matches!(self.next_non_whitespace(), Some(',' | '}'))
    }

    fn is_enum_member(&self) -> bool {
        if self.in_use_declaration {
            return false;
        }

        let next_is_path_separator = self.current == Some(':') && self.peek() == Some(':');

        // last item in a path is probably an enum member, assuming we're not in a use
        // declaration
        if self.last_significant == SignificantKind::PathSeparator && !next_is_path_separator {
            return true;
        }

        // something like `Foo()`
        if self.current == Some('(') {
            return true;
        }

        self.in_braces()
            && matches!(
                self.last_significant,
                SignificantKind::OpenDelimiter(Delimiter::Brace) | SignificantKind::Comma
            )
            && !next_is_path_separator
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignificantKind {
    Comma,
    OpenDelimiter(Delimiter),
    CloseDelimiter(Delimiter),
    Dot,
    Colon,
    SemiColon,
    Other,
    OpenAttribute,
    PathSeparator,
    Keyword,
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

    #[track_caller]
    fn assert_token(lexer: &mut RustLexer, kind: TokenKind, range: Range<usize>) {
        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind,
            range: ByteIndex::new(range.start)..ByteIndex::new(range.end),
        });
    }

    macro_rules! assert_tokens {
        ($source:expr, $(($kind:ident, $start:expr, $end:expr)), +$(,)?) => {{
            let source = Rope::from_str($source);
            let mut lexer = RustLexer::new(source, ByteIndex::new(0));
            $(assert_token(&mut lexer, TokenKind::$kind, $start..$end);)+
            assert!(lexer.next_token().is_none());
        }};
    }

    #[test]
    fn identifiers() {
        assert_tokens!(
            "foo bar __12baz",
            (Identifier, 0, 3),
            (Whitespace, 3, 4),
            (Identifier, 4, 7),
            (Whitespace, 7, 8),
            (Identifier, 8, 15),
        );
    }

    #[test]
    fn keywords() {
        assert_tokens!(
            "use foo impl bar",
            (Keyword, 0, 3),
            (Whitespace, 3, 4),
            (Identifier, 4, 7),
            (Whitespace, 7, 8),
            (Keyword, 8, 12),
            (Whitespace, 12, 13),
            (Identifier, 13, 16),
        );
    }

    #[test]
    fn strings() {
        assert_tokens!(
            r#"foo "hello" "h\"i""#,
            (Identifier, 0, 3),
            (Whitespace, 3, 4),
            (String, 4, 11),
            (Whitespace, 11, 12),
            (String, 12, 18),
        );
    }

    #[test]
    fn types() {
        assert_tokens!(
            "struct Foo struct __12Foo",
            (Keyword, 0, 6),
            (Whitespace, 6, 7),
            (Type, 7, 10),
            (Whitespace, 10, 11),
            (Keyword, 11, 17),
            (Whitespace, 17, 18),
            (Type, 18, 25),
        );
    }

    #[test]
    fn comments() {
        assert_tokens!(
            "// hello
// hi
use foo",
            (Comment, 0, 8),
            (Whitespace, 8, 9),
            (Comment, 9, 14),
            (Whitespace, 14, 15),
            (Keyword, 15, 18),
            (Whitespace, 18, 19),
            (Identifier, 19, 22),
        );
    }

    #[test]
    fn doc_comments() {
        assert_tokens!(
            "/// hello
/// hi
use foo",
            (Comment, 0, 9),
            (Whitespace, 9, 10),
            (Comment, 10, 16),
            (Whitespace, 16, 17),
            (Keyword, 17, 20),
            (Whitespace, 20, 21),
            (Identifier, 21, 24),
        );
    }

    #[test]
    fn block_comments() {
        assert_tokens!(
            "use /* foo */ bar",
            (Keyword, 0, 3),
            (Whitespace, 3, 4),
            (Comment, 4, 13),
            (Whitespace, 13, 14),
            (Identifier, 14, 17),
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

        let mut position = 0;

        for (index, operator) in operators.iter().enumerate() {
            assert_token(
                &mut lexer,
                TokenKind::Operator,
                position..position + operator.len(),
            );
            position += operator.len();

            if index + 1 != operators.len() {
                assert_token(&mut lexer, TokenKind::Whitespace, position..position + 1);
                position += 1;
            }
        }
    }

    #[test]
    fn chars() {
        assert_tokens!(
            "'h' 'i'",
            (Character, 0, 3),
            (Whitespace, 3, 4),
            (Character, 4, 7),
        );
    }

    #[test]
    fn long_chars() {
        assert_tokens!("'\\''", (Character, 0, 4));
    }

    #[test]
    fn lifetimes() {
        assert_tokens!(
            "impl<'src> Highlighter<'src>",
            (Keyword, 0, 4),
            (Operator, 4, 5),
            (Lifetime, 5, 9),
            (Operator, 9, 10),
            (Whitespace, 10, 11),
            (Type, 11, 22),
            (Operator, 22, 23),
            (Lifetime, 23, 27),
            (Operator, 27, 28),
        );
    }

    #[test]
    fn function_name() {
        assert_tokens!(
            "fn hello()",
            (Keyword, 0, 2),
            (Whitespace, 2, 3),
            (FunctionName, 3, 8),
            (Punctuation, 8, 9),
            (Punctuation, 9, 10),
        );
    }

    #[test]
    fn punctuation() {
        let punctuation = ["(", ")", "[", "]", "{", "}", "::", ":", ".", ",", ";"];
        let source = Rope::from_str(&punctuation.join(" "));
        let mut lexer = RustLexer::new(source, ByteIndex::new(0));

        let mut position = 0;
        for (index, symbol) in punctuation.iter().enumerate() {
            assert_token(
                &mut lexer,
                TokenKind::Punctuation,
                position..position + symbol.len(),
            );
            position += symbol.len();

            if index + 1 != punctuation.len() {
                assert_token(&mut lexer, TokenKind::Whitespace, position..position + 1);
                position += 1;
            }
        }
    }

    #[test]
    fn ints() {
        assert_tokens!(
            "123 45 6_usize",
            (Number, 0, 3),
            (Whitespace, 3, 4),
            (Number, 4, 6),
            (Whitespace, 6, 7),
            (Number, 7, 14),
        );
    }

    #[test]
    fn floats() {
        assert_tokens!(
            "123.45_f32 45.03 6_700.67_f64",
            (Number, 0, 10),
            (Whitespace, 10, 11),
            (Number, 11, 16),
            (Whitespace, 16, 17),
            (Number, 17, 29),
        );
    }

    #[test]
    fn range_isnt_number() {
        assert_tokens!(
            "0..foo",
            (Number, 0, 1),
            (Operator, 1, 3),
            (Identifier, 3, 6)
        );
    }

    #[test]
    fn macros() {
        assert_tokens!(
            "foo! foo!=bar",
            (Macro, 0, 4),
            (Whitespace, 4, 5),
            (Identifier, 5, 8),
            (Operator, 8, 10),
            (Identifier, 10, 13),
        );
    }

    #[test]
    fn properties() {
        assert_tokens!(
            "{ foo: bar }",
            (Punctuation, 0, 1),
            (Whitespace, 1, 2),
            (Property, 2, 5),
            (Punctuation, 5, 6),
            (Whitespace, 6, 7),
            (Identifier, 7, 10),
            (Whitespace, 10, 11),
            (Punctuation, 11, 12),
        );

        assert_tokens!(
            "{ pub foo: bar }",
            (Punctuation, 0, 1),
            (Whitespace, 1, 2),
            (Keyword, 2, 5),
            (Whitespace, 5, 6),
            (Property, 6, 9),
            (Punctuation, 9, 10),
            (Whitespace, 10, 11),
            (Identifier, 11, 14),
            (Whitespace, 14, 15),
            (Punctuation, 15, 16),
        );

        assert_tokens!(
            "{ pub(crate) foo: bar }",
            (Punctuation, 0, 1),
            (Whitespace, 1, 2),
            (Keyword, 2, 5),
            (Punctuation, 5, 6),
            (Keyword, 6, 11),
            (Punctuation, 11, 12),
            (Whitespace, 12, 13),
            (Property, 13, 16),
            (Punctuation, 16, 17),
            (Whitespace, 17, 18),
            (Identifier, 18, 21),
            (Whitespace, 21, 22),
            (Punctuation, 22, 23),
        );
    }

    #[test]
    fn shorthand_properties() {
        assert_tokens!(
            "Foo  { foo }",
            (Type, 0, 3),
            (Whitespace, 3, 5),
            (Punctuation, 5, 6),
            (Whitespace, 6, 7),
            (Property, 7, 10),
            (Whitespace, 10, 11),
            (Punctuation, 11, 12),
        );
    }

    #[test]
    fn function_call() {
        assert_tokens!(
            "foo()",
            (FunctionName, 0, 3),
            (Punctuation, 3, 4),
            (Punctuation, 4, 5),
        );
    }

    #[test]
    fn property_access() {
        assert_tokens!(
            "self.foo.bar.baz()",
            (Keyword, 0, 4),
            (Punctuation, 4, 5),
            (PropertyAccess, 5, 8),
            (Punctuation, 8, 9),
            (PropertyAccess, 9, 12),
            (Punctuation, 12, 13),
            (FunctionName, 13, 16),
            (Punctuation, 16, 17),
            (Punctuation, 17, 18),
        );
    }

    #[test]
    fn consts() {
        assert_tokens!(
            "const HELLO",
            (Keyword, 0, 5),
            (Whitespace, 5, 6),
            (Constant, 6, 11)
        );

        assert_tokens!("HELLO", (Constant, 0, 5));
    }

    #[test]
    fn attributes() {
        assert_tokens!(
            "#[test]",
            (Punctuation, 0, 2),
            (Identifier, 2, 6),
            (Punctuation, 6, 7),
        );

        assert_tokens!(
            "#[cfg(test)]",
            (Punctuation, 0, 2),
            (Macro, 2, 5),
            (Punctuation, 5, 6),
            (Identifier, 6, 10),
            (Punctuation, 10, 11),
            (Punctuation, 11, 12),
        );
    }

    #[test]
    fn enum_members() {
        assert_tokens!(
            "enum Foo { Bar, Baz }",
            (Keyword, 0, 4),
            (Whitespace, 4, 5),
            (Type, 5, 8),
            (Whitespace, 8, 9),
            (Punctuation, 9, 10),
            (Whitespace, 10, 11),
            (EnumMember, 11, 14),
            (Punctuation, 14, 15),
            (Whitespace, 15, 16),
            (EnumMember, 16, 19),
            (Whitespace, 19, 20),
            (Punctuation, 20, 21),
        );

        assert_tokens!(
            "match foo { Some(_) => {}, None => {} }",
            (Keyword, 0, 5),
            (Whitespace, 5, 6),
            (Identifier, 6, 9),
            (Whitespace, 9, 10),
            (Punctuation, 10, 11),
            (Whitespace, 11, 12),
            (EnumMember, 12, 16),
            (Punctuation, 16, 17),
            (Keyword, 17, 18),
            (Punctuation, 18, 19),
            (Whitespace, 19, 20),
            (Operator, 20, 22),
            (Whitespace, 22, 23),
            (Punctuation, 23, 24),
            (Punctuation, 24, 25),
            (Punctuation, 25, 26),
            (Whitespace, 26, 27),
            (EnumMember, 27, 31),
            (Whitespace, 31, 32),
            (Operator, 32, 34),
            (Whitespace, 34, 35),
            (Punctuation, 35, 36),
            (Punctuation, 36, 37),
            (Whitespace, 37, 38),
            (Punctuation, 38, 39),
        );

        assert_tokens!(
            "&Foo::Bar",
            (Operator, 0, 1),
            (Type, 1, 4),
            (Punctuation, 4, 6),
            (EnumMember, 6, 9),
        );

        // this is testing that `Foo` is not marked as an `EnumMember`
        assert_tokens!(
            "{ type Foo = (Bar, Baz) }",
            (Punctuation, 0, 1),
            (Whitespace, 1, 2),
            (Keyword, 2, 6),
            (Whitespace, 6, 7),
            (Type, 7, 10),
            (Whitespace, 10, 11),
            (Operator, 11, 12),
            (Whitespace, 12, 13),
            (Punctuation, 13, 14),
            (Type, 14, 17),
            (Punctuation, 17, 18),
            (Whitespace, 18, 19),
            (Type, 19, 22),
            (Punctuation, 22, 23),
            (Whitespace, 23, 24),
            (Punctuation, 24, 25),
        );

        // this is testing that `Bar` is not marked as an `EnumMember`
        assert_tokens!(
            "use foo::Bar;",
            (Keyword, 0, 3),
            (Whitespace, 3, 4),
            (Identifier, 4, 7),
            (Punctuation, 7, 9),
            (Type, 9, 12),
            (Punctuation, 12, 13),
        );
    }
}
