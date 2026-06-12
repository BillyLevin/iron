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
}
