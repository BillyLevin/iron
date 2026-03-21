use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug)]
pub(crate) struct KeyMap {
    map: HashMap<KeyEvent, Action>,
}

impl KeyMap {
    fn new(mappings: &[(KeyEvent, Action)]) -> Self {
        Self {
            map: mappings.iter().copied().collect(),
        }
    }

    pub(crate) fn normal() -> Self {
        Self::new(&[
            (KeyEvent::from(KeyCode::Char('j')), Action::MoveDown),
            (KeyEvent::from(KeyCode::Char('k')), Action::MoveUp),
            (KeyEvent::from(KeyCode::Char('l')), Action::MoveRight),
            (KeyEvent::from(KeyCode::Char('h')), Action::MoveLeft),
            (
                KeyEvent::from(KeyCode::Char('w')),
                Action::MoveNextWordStart,
            ),
            (
                KeyEvent::from(KeyCode::Char('b')),
                Action::MovePrevWordStart,
            ),
            (
                KeyEvent::from(KeyCode::Char('i')),
                Action::SwitchToInsertMode,
            ),
            (KeyEvent::from(KeyCode::Char('$')), Action::MoveLineEnd),
            (KeyEvent::from(KeyCode::Char('0')), Action::MoveLineStart),
            (
                KeyEvent::from(KeyCode::Char('^')),
                Action::MoveLineFirstNonBlank,
            ),
            (
                KeyEvent::from(KeyCode::Char('}')),
                Action::MoveNextParagraph,
            ),
        ])
    }

    pub(crate) fn insert() -> Self {
        Self::new(&[
            (KeyEvent::from(KeyCode::Backspace), Action::DeleteGrapheme),
            (KeyEvent::from(KeyCode::Esc), Action::SwitchToNormalMode),
            (KeyEvent::from(KeyCode::Enter), Action::InsertNewline),
        ])
    }

    pub(crate) fn get(&self, key_event: KeyEvent) -> Option<Action> {
        self.map.get(&key_event).copied()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Action {
    MoveDown,
    MoveUp,
    MoveRight,
    MoveLeft,
    MoveNextWordStart,
    MovePrevWordStart,
    MoveLineStart,
    MoveLineEnd,
    MoveLineFirstNonBlank,
    SwitchToInsertMode,
    SwitchToNormalMode,
    InsertChar(char),
    DeleteGrapheme,
    InsertNewline,
    MoveNextParagraph,
}

impl Action {
    pub(crate) const fn is_non_vertical_movement(self) -> bool {
        match self {
            Self::MoveRight
            | Self::MoveLeft
            | Self::MoveNextWordStart
            | Self::MovePrevWordStart
            | Self::MoveLineStart
            | Self::MoveLineEnd
            | Self::MoveLineFirstNonBlank
            | Self::MoveNextParagraph => true,

            Self::MoveDown
            | Self::MoveUp
            | Self::SwitchToInsertMode
            | Self::SwitchToNormalMode
            | Self::InsertChar(_)
            | Self::DeleteGrapheme
            | Self::InsertNewline => false,
        }
    }
}
