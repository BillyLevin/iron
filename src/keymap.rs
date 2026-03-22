use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};

macro_rules! key {
    ($key:literal) => {
        ::crossterm::event::KeyEvent::from(KeyCode::Char($key))
    };

    ($key:ident) => {
        ::crossterm::event::KeyEvent::from(KeyCode::$key)
    };
}

#[derive(Debug)]
pub(crate) enum KeyMap {
    BindingPart { map: HashMap<KeyEvent, Self> },
    Action(Action),
}

impl KeyMap {
    fn new() -> Self {
        Self::BindingPart {
            map: HashMap::new(),
        }
    }

    pub(crate) fn normal() -> Self {
        let mut map = Self::new();

        map.register(&[key!('j')], Action::MoveDown);
        map.register(&[key!('k')], Action::MoveUp);
        map.register(&[key!('l')], Action::MoveRight);
        map.register(&[key!('h')], Action::MoveLeft);
        map.register(&[key!('w')], Action::MoveNextWordStart);
        map.register(&[key!('b')], Action::MovePrevWordStart);
        map.register(&[key!('i')], Action::SwitchToInsertMode);
        map.register(&[key!('$')], Action::MoveLineEnd);
        map.register(&[key!('0')], Action::MoveLineStart);
        map.register(&[key!('^')], Action::MoveLineFirstNonBlank);
        map.register(&[key!('}')], Action::MoveNextParagraph);
        map.register(&[key!('{')], Action::MovePrevParagraph);
        map.register(&[key!('G')], Action::GoToLastLine);

        map
    }

    pub(crate) fn insert() -> Self {
        let mut map = Self::new();

        map.register(&[key!(Backspace)], Action::DeleteGrapheme);
        map.register(&[key!(Esc)], Action::SwitchToNormalMode);
        map.register(&[key!(Enter)], Action::InsertNewline);

        map
    }

    pub(crate) fn get(&self, keys: &KeySequence) -> Option<&Self> {
        let mut current = self;

        for key in keys {
            current = match *current {
                Self::BindingPart { ref map } => map.get(key),
                Self::Action(_) => None,
            }?;
        }

        Some(current)
    }

    fn register(&mut self, keys: &[KeyEvent], action: Action) {
        match *keys {
            [] => {}
            [key] => {
                match *self {
                    Self::BindingPart { ref mut map } => {
                        map.insert(key, Self::Action(action));
                    }
                    // TODO: should we error rather than silently ignore? can the types be
                    // reworked so that this is impossible in the first place?
                    Self::Action(_) => {}
                }
            }
            [key, ref rest @ ..] => {
                match *self {
                    Self::BindingPart { ref mut map } => {
                        map.entry(key)
                            .or_insert_with(Self::new)
                            .register(rest, action);
                    }
                    // TODO: should we error rather than silently ignore? can the types be
                    // reworked so that this is impossible in the first place?
                    Self::Action(_) => {}
                }
            }
        }
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
    MovePrevParagraph,
    GoToLastLine,
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
            | Self::MoveNextParagraph
            | Self::MovePrevParagraph
            | Self::GoToLastLine => true,

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

#[derive(Debug, Default, derive_more::IntoIterator)]
pub(crate) struct KeySequence {
    #[into_iterator(owned, ref)]
    keys: Vec<KeyEvent>,
}

impl KeySequence {
    pub(crate) fn clear(&mut self) {
        self.keys.clear();
    }

    pub(crate) fn push(&mut self, key: KeyEvent) {
        self.keys.push(key);
    }
}
