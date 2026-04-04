use std::{
    collections::HashMap,
    fmt,
};

use crossterm::event::{
    KeyCode,
    KeyEvent,
    KeyModifiers,
};

macro_rules! key {
    ($key:literal) => {
        KeyBinding::from(KeyCode::Char($key))
    };

    ($key:ident) => {
        KeyBinding::from(KeyCode::$key)
    };
}

#[derive(Debug)]
pub(crate) enum KeyMap {
    BindingPart { map: HashMap<KeyBinding, Self> },
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
        map.register(&[key!('a')], Action::AppendText);
        map.register(&[key!('A')], Action::AppendTextLineEnd);
        map.register(&[key!('e')], Action::MoveWordEnd);

        map.register(&[key!('g'), key!('e')], Action::GoToLastLine);
        map.register(&[key!('g'), key!('g')], Action::GoToFirstLine);

        map.register(&[key!('d'), key!('w')], Action::DeleteWord);
        map.register(&[key!('d'), key!('$')], Action::DeleteToLineEnd);
        map.register(&[key!('d'), key!('0')], Action::DeleteToLineStart);
        map.register(&[key!('d'), key!('^')], Action::DeleteToLineFirstNonBlank);
        map.register(&[key!('d'), key!('d')], Action::DeleteLine);
        map.register(&[key!('d'), key!('i'), key!('w')], Action::DeleteWholeWord);
        map.register(&[key!('d'), key!('b')], Action::DeleteToPrevWordStart);
        map.register(&[key!('d'), key!('e')], Action::DeleteToWordEnd);

        map.register(&[key!('c'), key!('w')], Action::ChangeWord);
        map.register(&[key!('c'), key!('$')], Action::ChangeToLineEnd);
        map.register(&[key!('c'), key!('0')], Action::ChangeToLineStart);
        map.register(&[key!('c'), key!('^')], Action::ChangeToLineFirstNonBlank);
        map.register(&[key!('c'), key!('c')], Action::ChangeLine);

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

    fn register(&mut self, keys: &[KeyBinding], action: Action) {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct KeyBinding {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl From<KeyCode> for KeyBinding {
    fn from(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::empty(),
        }
    }
}

impl From<KeyEvent> for KeyBinding {
    fn from(event: KeyEvent) -> Self {
        let mut modifiers = event.modifiers;

        // we don't need to differentiate between shift/no shift for capital letters
        if modifiers.contains(KeyModifiers::SHIFT)
            && let KeyCode::Char(ch) = event.code
            && ch.is_uppercase()
        {
            modifiers.remove(KeyModifiers::SHIFT);
        }

        Self {
            code: event.code,
            modifiers,
        }
    }
}

impl fmt::Display for KeyBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            write!(f, "C-")?;
        }

        if self.modifiers.contains(KeyModifiers::ALT) {
            write!(f, "A-")?;
        }

        if self.modifiers.contains(KeyModifiers::SHIFT) {
            write!(f, "S-")?;
        }

        if self.modifiers.contains(KeyModifiers::META) {
            write!(f, "M-")?;
        }

        write!(f, "{}", self.code)
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
    GoToFirstLine,
    DeleteWord,
    ChangeWord,
    DeleteToLineEnd,
    ChangeToLineEnd,
    DeleteToLineStart,
    DeleteToLineFirstNonBlank,
    DeleteLine,
    DeleteWholeWord,
    DeleteToPrevWordStart,
    AppendText,
    AppendTextLineEnd,
    MoveWordEnd,
    DeleteToWordEnd,
    ChangeToLineStart,
    ChangeToLineFirstNonBlank,
    ChangeLine,
}

impl Action {
    /// The desired cursor column should be reset on all potential cursor
    /// movements that aren't vertical (i.e. `j`/`k` commands) in order to
    /// prevent the cursor from jumping to unexpected columns when
    /// navigating.
    pub(crate) const fn should_reset_desired_column(self) -> bool {
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
            | Self::GoToLastLine
            | Self::GoToFirstLine
            | Self::InsertChar(_)
            | Self::DeleteGrapheme
            | Self::InsertNewline
            | Self::DeleteWord
            | Self::ChangeWord
            | Self::DeleteToLineEnd
            | Self::ChangeToLineEnd
            | Self::DeleteToLineStart
            | Self::DeleteToLineFirstNonBlank
            | Self::DeleteLine
            | Self::DeleteWholeWord
            | Self::DeleteToPrevWordStart
            | Self::AppendText
            | Self::AppendTextLineEnd
            | Self::MoveWordEnd
            | Self::DeleteToWordEnd
            | Self::ChangeToLineStart
            | Self::ChangeToLineFirstNonBlank
            | Self::ChangeLine => true,

            Self::MoveDown | Self::MoveUp | Self::SwitchToInsertMode | Self::SwitchToNormalMode => {
                false
            }
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::MoveDown => "Move cursor down",
            Self::MoveUp => "Move cursor up",
            Self::MoveRight => "Move cursor right",
            Self::MoveLeft => "Move cursor left",
            Self::MoveNextWordStart => "Move to next word start",
            Self::MovePrevWordStart => "Move to previous word start",
            Self::MoveLineStart => "Move to start of line",
            Self::MoveLineEnd => "Move to end of line",
            Self::MoveLineFirstNonBlank => "Move to first non-blank character",
            Self::SwitchToInsertMode => "Switch to insert mode",
            Self::SwitchToNormalMode => "Switch to normal mode",
            Self::InsertChar(_) => "Insert character",
            Self::DeleteGrapheme => "Delete character",
            Self::InsertNewline => "Insert newline",
            Self::MoveNextParagraph => "Move to next paragraph",
            Self::MovePrevParagraph => "Move to previous paragraph",
            Self::GoToLastLine => "Go to last line",
            Self::GoToFirstLine => "Go to first line",
            Self::DeleteWord => "Delete word",
            Self::ChangeWord => "Change word",
            Self::DeleteToLineEnd => "Delete to end of line",
            Self::ChangeToLineEnd => "Change to end of line",
            Self::DeleteToLineStart => "Delete to start of line",
            Self::DeleteToLineFirstNonBlank => "Delete to first non-blank character",
            Self::DeleteLine => "Delete line",
            Self::DeleteWholeWord => "Delete whole word",
            Self::DeleteToPrevWordStart => "Delete to previous word start",
            Self::AppendText => "Append text",
            Self::AppendTextLineEnd => "Append text at end of line",
            Self::MoveWordEnd => "Move to end of word",
            Self::DeleteToWordEnd => "Delete to end of word",
            Self::ChangeToLineStart => "Change to start of line",
            Self::ChangeToLineFirstNonBlank => "Change to first non-blank character",
            Self::ChangeLine => "Change line",
        }
    }
}

#[derive(Debug, Default, derive_more::IntoIterator)]
pub(crate) struct KeySequence {
    #[into_iterator(owned, ref)]
    keys: Vec<KeyBinding>,
}

impl KeySequence {
    pub(crate) fn clear(&mut self) {
        self.keys.clear();
    }

    pub(crate) fn push(&mut self, key: KeyBinding) {
        self.keys.push(key);
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}
