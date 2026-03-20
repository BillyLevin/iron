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
}
