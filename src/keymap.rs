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
    Action(DocumentAction),
}

impl KeyMap {
    fn new() -> Self {
        Self::BindingPart {
            map: HashMap::new(),
        }
    }

    pub(crate) fn normal() -> Self {
        let mut map = Self::new();

        map.register(&[key!('j')], DocumentAction::MoveDown);
        map.register(&[key!('k')], DocumentAction::MoveUp);
        map.register(&[key!('l')], DocumentAction::MoveRight);
        map.register(&[key!('h')], DocumentAction::MoveLeft);
        map.register(&[key!('w')], DocumentAction::MoveNextWordStart);
        map.register(&[key!('b')], DocumentAction::MovePrevWordStart);
        map.register(&[key!('i')], DocumentAction::SwitchToInsertMode);
        map.register(&[key!('v')], DocumentAction::SwitchToVisualMode);
        map.register(&[key!('$')], DocumentAction::MoveLineEnd);
        map.register(&[key!('0')], DocumentAction::MoveLineStart);
        map.register(&[key!('^')], DocumentAction::MoveLineFirstNonBlank);
        map.register(&[key!('}')], DocumentAction::MoveNextParagraph);
        map.register(&[key!('{')], DocumentAction::MovePrevParagraph);
        map.register(&[key!('a')], DocumentAction::AppendText);
        map.register(&[key!('A')], DocumentAction::AppendTextLineEnd);
        map.register(&[key!('e')], DocumentAction::MoveWordEnd);
        map.register(&[key!('o')], DocumentAction::OpenLineBelow);
        map.register(&[key!('O')], DocumentAction::OpenLineAbove);
        map.register(&[key!(':')], DocumentAction::OpenCommandList);

        map.register(&[key!('g'), key!('e')], DocumentAction::GoToLastLine);
        map.register(&[key!('g'), key!('g')], DocumentAction::GoToFirstLine);
        map.register(&[key!('G')], DocumentAction::GoToLastLine);

        map.register(&[key!('d'), key!('w')], DocumentAction::DeleteWord);
        map.register(&[key!('d'), key!('$')], DocumentAction::DeleteToLineEnd);
        map.register(&[key!('d'), key!('0')], DocumentAction::DeleteToLineStart);
        map.register(
            &[key!('d'), key!('^')],
            DocumentAction::DeleteToLineFirstNonBlank,
        );
        map.register(&[key!('d'), key!('d')], DocumentAction::DeleteLine);
        map.register(
            &[key!('d'), key!('i'), key!('w')],
            DocumentAction::DeleteWholeWord,
        );
        map.register(
            &[key!('d'), key!('b')],
            DocumentAction::DeleteToPrevWordStart,
        );
        map.register(&[key!('d'), key!('e')], DocumentAction::DeleteToWordEnd);
        map.register(&[key!('d'), key!('j')], DocumentAction::DeleteDown);
        map.register(&[key!('d'), key!('k')], DocumentAction::DeleteUp);

        map.register(&[key!('c'), key!('w')], DocumentAction::ChangeWord);
        map.register(&[key!('c'), key!('$')], DocumentAction::ChangeToLineEnd);
        map.register(&[key!('c'), key!('0')], DocumentAction::ChangeToLineStart);
        map.register(
            &[key!('c'), key!('^')],
            DocumentAction::ChangeToLineFirstNonBlank,
        );
        map.register(&[key!('c'), key!('c')], DocumentAction::ChangeLine);
        map.register(
            &[key!('c'), key!('i'), key!('w')],
            DocumentAction::ChangeWholeWord,
        );
        map.register(
            &[key!('c'), key!('b')],
            DocumentAction::ChangeToPrevWordStart,
        );
        map.register(&[key!('c'), key!('e')], DocumentAction::ChangeToWordEnd);

        map
    }

    pub(crate) fn insert() -> Self {
        let mut map = Self::new();

        map.register(&[key!(Backspace)], DocumentAction::DeleteGrapheme);
        map.register(&[key!(Esc)], DocumentAction::SwitchToNormalMode);
        map.register(&[key!(Enter)], DocumentAction::InsertNewline);

        map
    }

    pub(crate) fn visual() -> Self {
        let mut map = Self::new();

        map.register(&[key!(Esc)], DocumentAction::SwitchToNormalMode);

        map.register(&[key!('j')], DocumentAction::MoveDown);
        map.register(&[key!('k')], DocumentAction::MoveUp);
        map.register(&[key!('l')], DocumentAction::MoveRight);
        map.register(&[key!('h')], DocumentAction::MoveLeft);
        map.register(&[key!('w')], DocumentAction::MoveNextWordStart);
        map.register(&[key!('b')], DocumentAction::MovePrevWordStart);
        map.register(&[key!('$')], DocumentAction::MoveLineEnd);
        map.register(&[key!('0')], DocumentAction::MoveLineStart);
        map.register(&[key!('^')], DocumentAction::MoveLineFirstNonBlank);
        map.register(&[key!('}')], DocumentAction::MoveNextParagraph);
        map.register(&[key!('{')], DocumentAction::MovePrevParagraph);
        map.register(&[key!('e')], DocumentAction::MoveWordEnd);
        map.register(&[key!('o')], DocumentAction::ReverseSelection);

        map.register(&[key!('g'), key!('e')], DocumentAction::GoToLastLine);
        map.register(&[key!('g'), key!('g')], DocumentAction::GoToFirstLine);
        map.register(&[key!('G')], DocumentAction::GoToLastLine);

        map.register(&[key!('d')], DocumentAction::DeleteSelection);
        map.register(&[key!('c')], DocumentAction::ChangeSelection);

        map.register(&[key!('i'), key!('w')], DocumentAction::SelectCurrentWord);

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

    fn register(&mut self, keys: &[KeyBinding], action: DocumentAction) {
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

impl KeyBinding {
    pub(crate) const fn code(&self) -> KeyCode {
        self.code
    }

    pub(crate) const fn modifiers(&self) -> KeyModifiers {
        self.modifiers
    }
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
pub(crate) enum DocumentAction {
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
    SwitchToVisualMode,
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
    ChangeWholeWord,
    ChangeToPrevWordStart,
    ChangeToWordEnd,
    DeleteSelection,
    ChangeSelection,
    ReverseSelection,
    OpenLineBelow,
    OpenLineAbove,
    SelectCurrentWord,
    DeleteDown,
    DeleteUp,
    OpenCommandList,
}

impl DocumentAction {
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
            | Self::ChangeLine
            | Self::ChangeWholeWord
            | Self::ChangeToPrevWordStart
            | Self::ChangeToWordEnd
            | Self::DeleteSelection
            | Self::ChangeSelection
            | Self::ReverseSelection
            | Self::OpenLineBelow
            | Self::OpenLineAbove
            | Self::SelectCurrentWord
            | Self::DeleteDown
            | Self::DeleteUp => true,

            Self::MoveDown
            | Self::MoveUp
            | Self::SwitchToInsertMode
            | Self::SwitchToNormalMode
            | Self::SwitchToVisualMode
            | Self::OpenCommandList => false,
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
            Self::SwitchToVisualMode => "Switch to visual mode",
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
            Self::ChangeWholeWord => "Change whole word",
            Self::ChangeToPrevWordStart => "Change to previous word start",
            Self::ChangeToWordEnd => "Change to end of word",
            Self::DeleteSelection => "Delete selection",
            Self::ChangeSelection => "Change selection",
            Self::ReverseSelection => "Reverse selection",
            Self::OpenLineBelow => "Open line below",
            Self::OpenLineAbove => "Open line above",
            Self::SelectCurrentWord => "Select current word",
            Self::DeleteDown => "Delete down",
            Self::DeleteUp => "Delete up",
            Self::OpenCommandList => "Open command list",
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
