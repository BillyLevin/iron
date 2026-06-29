use crate::highlight::{
    Checkpoint,
    Source,
    Token,
    TokenKind,
};

#[derive(Debug)]
pub(super) struct TomlLexer {
    source: Source,
    context: Context,
}

impl TomlLexer {
    pub(super) const fn new(source: Source) -> Self {
        Self {
            source,
            context: Context::Key,
        }
    }

    pub(super) fn next_token(&mut self) -> Option<(Token, Checkpoint)> {
        self.source.current.map(|ch| self.read_token(ch))
    }

    fn read_token(&mut self, ch: char) -> (Token, Checkpoint) {
        let start = self.source.position;
        let checkpoint = if self.context == Context::Key {
            Checkpoint::Yes
        } else {
            Checkpoint::No
        };

        let kind = match ch {
            c if c.is_whitespace() => self.read_whitespace(),
            '#' => self.read_comment(),
            '"' => {
                let result = self.read_string();
                if self.context == Context::Key {
                    TokenKind::Property
                } else {
                    result
                }
            }
            '+' | '-' | '0'..='9' if matches!(self.context, Context::Value | Context::Array) => {
                self.read_number()
            }
            c if is_bare_key_part(c) && self.context == Context::Key => self.read_bare_key(),
            c if is_bare_key_part(c) && matches!(self.context, Context::Value | Context::Array) => {
                self.read_bare_value()
            }
            '[' => self.read_open_bracket(),
            ']' => self.read_close_bracket(),
            '{' => self.read_open_brace(),
            '}' => self.read_close_brace(),
            '=' => self.read_equals(),
            _ => {
                self.source.next_char();
                TokenKind::Unknown
            }
        };

        if self.is_value(kind) {
            self.context = Context::Key;
        }

        let token = Token {
            kind,
            range: start..self.source.position,
        };

        (token, checkpoint)
    }

    fn read_whitespace(&mut self) -> TokenKind {
        self.source.eat_while(char::is_whitespace);
        TokenKind::Whitespace
    }

    fn read_comment(&mut self) -> TokenKind {
        self.source.assert('#');
        self.source.eat_while(|ch| ch != '\n');

        TokenKind::Comment
    }

    fn read_string(&mut self) -> TokenKind {
        self.source.assert('"');

        if self.source.current == Some('"') && self.source.peek() == Some('"') {
            self.source.assert('"');
            self.source.assert('"');
            return self.read_until_triple_quotes();
        }

        let mut is_escaped = false;

        self.source.eat_while(|ch| {
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
        self.source.next_char();

        TokenKind::String
    }

    fn read_until_triple_quotes(&mut self) -> TokenKind {
        while let Some(ch) = self.source.current {
            if ch == '"' && self.source.peek() == Some('"') && self.source.peek_2() == Some('"') {
                self.source.assert('"');
                self.source.assert('"');
                self.source.assert('"');
                break;
            }

            self.source.next_char();
        }

        TokenKind::String
    }

    fn read_number(&mut self) -> TokenKind {
        let _ = self.source.eat_if('+') || self.source.eat_if('-');

        self.source
            .eat_while(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.');

        TokenKind::Number
    }

    fn read_open_bracket(&mut self) -> TokenKind {
        match self.context {
            Context::Key => self.read_table_header(),
            Context::Value | Context::Array => {
                self.source.assert('[');
                self.context = Context::Array;
                TokenKind::Punctuation
            }
        }
    }

    fn read_close_bracket(&mut self) -> TokenKind {
        self.source.assert(']');
        self.context = Context::Key;
        TokenKind::Punctuation
    }

    fn read_table_header(&mut self) -> TokenKind {
        self.source.assert('[');

        let mut delim_count = 0_usize;

        self.source.eat_while(|ch| {
            match ch {
                '[' => {
                    delim_count += 1;
                    true
                }
                ']' => {
                    if delim_count == 0 {
                        false
                    } else {
                        delim_count -= 1;
                        true
                    }
                }
                _ => true,
            }
        });

        self.source.eat_if(']');

        TokenKind::Title
    }

    fn read_equals(&mut self) -> TokenKind {
        self.source.assert('=');
        self.context = Context::Value;

        TokenKind::Operator
    }

    fn read_bare_key(&mut self) -> TokenKind {
        self.source.eat_while(is_bare_key_part);

        TokenKind::Property
    }

    fn read_bare_value(&mut self) -> TokenKind {
        self.source.eat_while(is_bare_key_part);

        // we'll just assume anything written here is a keyword
        TokenKind::Keyword
    }

    fn read_open_brace(&mut self) -> TokenKind {
        self.source.assert('{');
        self.context = Context::Key;

        TokenKind::Punctuation
    }

    fn read_close_brace(&mut self) -> TokenKind {
        self.source.assert('}');
        self.context = Context::Key;

        TokenKind::Punctuation
    }

    fn is_value(&self, kind: TokenKind) -> bool {
        self.context == Context::Value
            && match kind {
                TokenKind::Keyword | TokenKind::String | TokenKind::Number => true,
                TokenKind::Identifier
                | TokenKind::Whitespace
                | TokenKind::Type
                | TokenKind::Comment
                | TokenKind::Operator
                | TokenKind::Unknown
                | TokenKind::Character
                | TokenKind::Lifetime
                | TokenKind::FunctionName
                | TokenKind::Punctuation
                | TokenKind::Macro
                | TokenKind::Property
                | TokenKind::PropertyAccess
                | TokenKind::Constant
                | TokenKind::EnumMember
                | TokenKind::Title
                | TokenKind::Marker => false,
            }
    }
}

const fn is_bare_key_part(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    Key,
    Value,
    Array,
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use ropey::Rope;

    use super::*;
    use crate::text::ByteIndex;

    #[track_caller]
    fn assert_token(lexer: &mut TomlLexer, kind: TokenKind, range: Range<usize>) {
        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind,
            range: ByteIndex::new(range.start)..ByteIndex::new(range.end),
        });
    }

    macro_rules! assert_tokens {
        ($source:expr, $(($kind:ident, $start:expr, $end:expr)), +$(,)?) => {{
            let source = Source::new(Rope::from_str($source), ByteIndex::new(0));
            let mut lexer = TomlLexer::new(source);
            $(assert_token(&mut lexer, TokenKind::$kind, $start..$end);)+
            assert!(lexer.next_token().is_none());
        }};
    }

    #[test]
    fn comments() {
        assert_tokens!("# hello comment", (Comment, 0, 15));
    }

    #[test]
    fn strings() {
        assert_tokens!(
            r#"key = "I'm a string. \"You can quote me\". Name\tJos\xE9\nLocation\tSF.a""#,
            (Property, 0, 3),
            (Whitespace, 3, 4),
            (Operator, 4, 5),
            (Whitespace, 5, 6),
            (String, 6, 73)
        );

        assert_tokens!(
            r#"key = """Here are two quotation marks: "". Simple enough.""""#,
            (Property, 0, 3),
            (Whitespace, 3, 4),
            (Operator, 4, 5),
            (Whitespace, 5, 6),
            (String, 6, 60)
        );
    }

    #[test]
    fn numbers() {
        for num in ["2", "+27", "-439", "4.56", "+34.98", "-334.30"] {
            assert_tokens!(
                &format!("key = {num}"),
                (Property, 0, 3),
                (Whitespace, 3, 4),
                (Operator, 4, 5),
                (Whitespace, 5, 6),
                (Number, 6, 6 + num.len())
            );
        }
    }

    #[test]
    fn tables() {
        assert_tokens!(
            r#"[hello-123]
hello = "world"
thing = true"#,
            (Title, 0, 11),
            (Whitespace, 11, 12),
            (Property, 12, 17),
            (Whitespace, 17, 18),
            (Operator, 18, 19),
            (Whitespace, 19, 20),
            (String, 20, 27),
            (Whitespace, 27, 28),
            (Property, 28, 33),
            (Whitespace, 33, 34),
            (Operator, 34, 35),
            (Whitespace, 35, 36),
            (Keyword, 36, 40),
        );

        assert_tokens!(
            r#"[[package]]
name = "anstream"
dependencies = [
 "anstyle",
 "anstyle-parse",
 "anstyle-query",
 "anstyle-wincon",
 "colorchoice",
 "is_terminal_polyfill",
 "utf8parse",
]"#,
            (Title, 0, 11),
            (Whitespace, 11, 12),
            (Property, 12, 16),
            (Whitespace, 16, 17),
            (Operator, 17, 18),
            (Whitespace, 18, 19),
            (String, 19, 29),
            (Whitespace, 29, 30),
            (Property, 30, 42),
            (Whitespace, 42, 43),
            (Operator, 43, 44),
            (Whitespace, 44, 45),
            (Punctuation, 45, 46),
            (Whitespace, 46, 48),
            (String, 48, 57),
            (Unknown, 57, 58),
            (Whitespace, 58, 60),
            (String, 60, 75),
            (Unknown, 75, 76),
            (Whitespace, 76, 78),
            (String, 78, 93),
            (Unknown, 93, 94),
            (Whitespace, 94, 96),
            (String, 96, 112),
            (Unknown, 112, 113),
            (Whitespace, 113, 115),
            (String, 115, 128),
            (Unknown, 128, 129),
            (Whitespace, 129, 131),
            (String, 131, 153),
            (Unknown, 153, 154),
            (Whitespace, 154, 156),
            (String, 156, 167),
            (Unknown, 167, 168),
            (Whitespace, 168, 169),
            (Punctuation, 169, 170),
        );
    }
}
