use ropey::Rope;

use crate::{
    highlight::{
        Checkpoint,
        Token,
        TokenKind,
    },
    text::ByteIndex,
};

#[derive(Debug)]
pub(super) struct TomlLexer {
    source: Rope,
    position: ByteIndex,
    current: Option<char>,
}

impl TomlLexer {
    pub(super) fn new(source: Rope, start: ByteIndex) -> Self {
        let current = source.get_char(start.value()).ok();

        Self {
            source,
            position: start,
            current,
        }
    }

    pub(super) fn next_token(&mut self) -> Option<(Token, Checkpoint)> {
        self.current.map(|ch| self.read_token(ch))
    }

    fn read_token(&mut self, ch: char) -> (Token, Checkpoint) {
        let start = self.position;

        let kind = match ch {
            c if c.is_whitespace() => self.read_whitespace(),
            '#' => self.read_comment(),
            '"' => self.read_string(),
            '+' | '-' | '0'..='9' => self.read_number(),
            '[' => self.read_table_header(),
            _ => {
                self.next_char();
                TokenKind::Unknown
            }
        };

        let token = Token {
            kind,
            range: start..self.position,
        };

        let checkpoint_outcome = if token.range.start == ByteIndex::new(0) {
            Checkpoint::Yes
        } else {
            Checkpoint::No
        };

        (token, checkpoint_outcome)
    }

    fn next_char(&mut self) -> Option<char> {
        self.position += self.current?.len_utf8();
        self.current = self.source.get_char(self.position.value()).ok();
        self.current
    }

    fn peek(&self) -> Option<char> {
        self.source.chars_at(self.position.value()).nth(1)
    }

    fn peek_2(&self) -> Option<char> {
        self.source.chars_at(self.position.value()).nth(2)
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

    fn read_comment(&mut self) -> TokenKind {
        self.assert('#');
        self.eat_while(|ch| ch != '\n');

        TokenKind::Comment
    }

    fn read_string(&mut self) -> TokenKind {
        self.assert('"');

        if self.current == Some('"') && self.peek() == Some('"') {
            self.assert('"');
            self.assert('"');
            return self.read_until_triple_quotes();
        }

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

    fn read_until_triple_quotes(&mut self) -> TokenKind {
        while let Some(ch) = self.current {
            if ch == '"' && self.peek() == Some('"') && self.peek_2() == Some('"') {
                self.assert('"');
                self.assert('"');
                self.assert('"');
                break;
            }

            self.next_char();
        }

        TokenKind::String
    }

    fn read_number(&mut self) -> TokenKind {
        let _ = self.eat_if('+') || self.eat_if('-');

        self.eat_while(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.');

        TokenKind::Number
    }

    fn read_table_header(&mut self) -> TokenKind {
        self.assert('[');

        let mut delim_count = 0_usize;

        self.eat_while(|ch| {
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

        self.eat_if(']');

        TokenKind::Title
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use ropey::Rope;

    use super::*;

    #[track_caller]
    fn assert_token(lexer: &mut TomlLexer, kind: TokenKind, range: Range<usize>) {
        assert_eq!(lexer.next_token().unwrap().0, Token {
            kind,
            range: ByteIndex::new(range.start)..ByteIndex::new(range.end),
        });
    }

    macro_rules! assert_tokens {
        ($source:expr, $(($kind:ident, $start:expr, $end:expr)), +$(,)?) => {{
            let source = Rope::from_str($source);
            let mut lexer = TomlLexer::new(source, ByteIndex::new(0));
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
            r#""I'm a string. \"You can quote me\". Name\tJos\xE9\nLocation\tSF.a""#,
            (String, 0, 67)
        );

        assert_tokens!(
            r#""""Here are two quotation marks: "". Simple enough.""""#,
            (String, 0, 54)
        );
    }

    #[test]
    fn numbers() {
        assert_tokens!(
            "2 +27 -439 4.56 +34.98 -334.30",
            (Number, 0, 1),
            (Whitespace, 1, 2),
            (Number, 2, 5),
            (Whitespace, 5, 6),
            (Number, 6, 10),
            (Whitespace, 10, 11),
            (Number, 11, 15),
            (Whitespace, 15, 16),
            (Number, 16, 22),
            (Whitespace, 22, 23),
            (Number, 23, 30),
        );
    }

    #[test]
    fn tables() {
        assert_tokens!("[hello-123]", (Title, 0, 11));
    }
}
