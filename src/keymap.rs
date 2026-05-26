use std::{
    collections::HashMap,
    fmt,
    num::{
        NonZero,
        NonZeroUsize,
    },
};

use crossterm::event::{
    KeyCode,
    KeyEvent,
    KeyModifiers,
};

use crate::document::Mode;

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

        map.register(
            &[key!('j')],
            DocumentAction::Movement(MovementAction::MoveDown),
        );
        map.register(
            &[key!('k')],
            DocumentAction::Movement(MovementAction::MoveUp),
        );
        map.register(
            &[key!('l')],
            DocumentAction::Movement(MovementAction::MoveRight),
        );
        map.register(
            &[key!('h')],
            DocumentAction::Movement(MovementAction::MoveLeft),
        );
        map.register(
            &[key!('w')],
            DocumentAction::Movement(MovementAction::MoveNextWordStart),
        );
        map.register(
            &[key!('b')],
            DocumentAction::Movement(MovementAction::MovePrevWordStart),
        );
        map.register(
            &[key!('i')],
            DocumentAction::Behavior(BehaviorAction::SwitchToInsertMode),
        );
        map.register(
            &[key!('v')],
            DocumentAction::Behavior(BehaviorAction::SwitchToVisualMode),
        );
        map.register(
            &[key!('$')],
            DocumentAction::Movement(MovementAction::MoveLineEnd),
        );
        map.register(
            &[key!('0')],
            DocumentAction::Movement(MovementAction::MoveLineStart),
        );
        map.register(
            &[key!('^')],
            DocumentAction::Movement(MovementAction::MoveLineFirstNonBlank),
        );
        map.register(
            &[key!('}')],
            DocumentAction::Movement(MovementAction::MoveNextParagraph),
        );
        map.register(
            &[key!('{')],
            DocumentAction::Movement(MovementAction::MovePrevParagraph),
        );
        map.register(&[key!('a')], DocumentAction::Edit(EditAction::AppendText));
        map.register(
            &[key!('A')],
            DocumentAction::Edit(EditAction::AppendTextLineEnd),
        );
        map.register(
            &[key!('e')],
            DocumentAction::Movement(MovementAction::MoveWordEnd),
        );
        map.register(
            &[key!('o')],
            DocumentAction::Edit(EditAction::OpenLineBelow),
        );
        map.register(
            &[key!('O')],
            DocumentAction::Edit(EditAction::OpenLineAbove),
        );
        map.register(
            &[key!(':')],
            DocumentAction::Behavior(BehaviorAction::OpenCommandList),
        );

        map.register(
            &[key!('g'), key!('e')],
            DocumentAction::Movement(MovementAction::GoToLastLine),
        );
        map.register(
            &[key!('g'), key!('g')],
            DocumentAction::Movement(MovementAction::GoToNthOrFirstLine),
        );
        map.register(
            &[key!('G')],
            DocumentAction::Movement(MovementAction::GoToNthOrLastLine),
        );

        map.register(
            &[key!('d'), key!('w')],
            DocumentAction::Edit(EditAction::DeleteWord),
        );
        map.register(
            &[key!('d'), key!('$')],
            DocumentAction::Edit(EditAction::DeleteToLineEnd),
        );
        map.register(
            &[key!('d'), key!('0')],
            DocumentAction::Edit(EditAction::DeleteToLineStart),
        );
        map.register(
            &[key!('d'), key!('^')],
            DocumentAction::Edit(EditAction::DeleteToLineFirstNonBlank),
        );
        map.register(
            &[key!('d'), key!('d')],
            DocumentAction::Edit(EditAction::DeleteLine),
        );
        map.register(
            &[key!('d'), key!('i'), key!('w')],
            DocumentAction::Edit(EditAction::DeleteWholeWord),
        );
        map.register(
            &[key!('d'), key!('b')],
            DocumentAction::Edit(EditAction::DeleteToPrevWordStart),
        );
        map.register(
            &[key!('d'), key!('e')],
            DocumentAction::Edit(EditAction::DeleteToWordEnd),
        );
        map.register(
            &[key!('d'), key!('j')],
            DocumentAction::Edit(EditAction::DeleteDown),
        );
        map.register(
            &[key!('d'), key!('k')],
            DocumentAction::Edit(EditAction::DeleteUp),
        );

        map.register(
            &[key!('c'), key!('w')],
            DocumentAction::Edit(EditAction::ChangeWord),
        );
        map.register(
            &[key!('c'), key!('$')],
            DocumentAction::Edit(EditAction::ChangeToLineEnd),
        );
        map.register(
            &[key!('c'), key!('0')],
            DocumentAction::Edit(EditAction::ChangeToLineStart),
        );
        map.register(
            &[key!('c'), key!('^')],
            DocumentAction::Edit(EditAction::ChangeToLineFirstNonBlank),
        );
        map.register(
            &[key!('c'), key!('c')],
            DocumentAction::Edit(EditAction::ChangeLine),
        );
        map.register(
            &[key!('c'), key!('i'), key!('w')],
            DocumentAction::Edit(EditAction::ChangeWholeWord),
        );
        map.register(
            &[key!('c'), key!('b')],
            DocumentAction::Edit(EditAction::ChangeToPrevWordStart),
        );
        map.register(
            &[key!('c'), key!('e')],
            DocumentAction::Edit(EditAction::ChangeToWordEnd),
        );

        map.register(
            &[key!(Esc)],
            DocumentAction::Behavior(BehaviorAction::ClearInput),
        );

        map
    }

    pub(crate) fn insert() -> Self {
        let mut map = Self::new();

        map.register(
            &[key!(Backspace)],
            DocumentAction::Edit(EditAction::DeleteGrapheme),
        );
        map.register(
            &[key!(Esc)],
            DocumentAction::Behavior(BehaviorAction::SwitchToNormalMode),
        );
        map.register(
            &[key!(Enter)],
            DocumentAction::Edit(EditAction::InsertNewline),
        );
        map.register(&[key!(Tab)], DocumentAction::Edit(EditAction::InsertTab));

        map
    }

    pub(crate) fn visual() -> Self {
        let mut map = Self::new();

        map.register(
            &[key!(Esc)],
            DocumentAction::Behavior(BehaviorAction::SwitchToNormalMode),
        );

        map.register(
            &[key!('j')],
            DocumentAction::Movement(MovementAction::MoveDown),
        );
        map.register(
            &[key!('k')],
            DocumentAction::Movement(MovementAction::MoveUp),
        );
        map.register(
            &[key!('l')],
            DocumentAction::Movement(MovementAction::MoveRight),
        );
        map.register(
            &[key!('h')],
            DocumentAction::Movement(MovementAction::MoveLeft),
        );
        map.register(
            &[key!('w')],
            DocumentAction::Movement(MovementAction::MoveNextWordStart),
        );
        map.register(
            &[key!('b')],
            DocumentAction::Movement(MovementAction::MovePrevWordStart),
        );
        map.register(
            &[key!('$')],
            DocumentAction::Movement(MovementAction::MoveLineEnd),
        );
        map.register(
            &[key!('0')],
            DocumentAction::Movement(MovementAction::MoveLineStart),
        );
        map.register(
            &[key!('^')],
            DocumentAction::Movement(MovementAction::MoveLineFirstNonBlank),
        );
        map.register(
            &[key!('}')],
            DocumentAction::Movement(MovementAction::MoveNextParagraph),
        );
        map.register(
            &[key!('{')],
            DocumentAction::Movement(MovementAction::MovePrevParagraph),
        );
        map.register(
            &[key!('e')],
            DocumentAction::Movement(MovementAction::MoveWordEnd),
        );
        map.register(
            &[key!('o')],
            DocumentAction::Movement(MovementAction::ReverseSelection),
        );

        map.register(
            &[key!('g'), key!('e')],
            DocumentAction::Movement(MovementAction::GoToLastLine),
        );
        map.register(
            &[key!('g'), key!('g')],
            DocumentAction::Movement(MovementAction::GoToNthOrFirstLine),
        );
        map.register(
            &[key!('G')],
            DocumentAction::Movement(MovementAction::GoToNthOrLastLine),
        );

        map.register(
            &[key!('d')],
            DocumentAction::Edit(EditAction::DeleteSelection),
        );
        map.register(
            &[key!('c')],
            DocumentAction::Edit(EditAction::ChangeSelection),
        );

        map.register(
            &[key!('i'), key!('w')],
            DocumentAction::Movement(MovementAction::SelectCurrentWord),
        );

        map.register(
            &[key!(':')],
            DocumentAction::Behavior(BehaviorAction::OpenCommandList),
        );

        map
    }

    pub(crate) fn get(&self, keys: &[KeyBinding]) -> Option<&Self> {
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
pub(crate) enum MovementAction {
    MoveDown,
    MoveUp,
    MoveRight,
    MoveLeft,
    MoveNextWordStart,
    MovePrevWordStart,
    MoveLineStart,
    MoveLineEnd,
    MoveLineFirstNonBlank,
    MoveNextParagraph,
    MovePrevParagraph,
    GoToLastLine,
    GoToNthOrFirstLine,
    GoToNthOrLastLine,
    MoveWordEnd,
    SelectCurrentWord,
    ReverseSelection,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum EditAction {
    InsertChar(char),
    DeleteGrapheme,
    InsertNewline,
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
    DeleteToWordEnd,
    ChangeToLineStart,
    ChangeToLineFirstNonBlank,
    ChangeLine,
    ChangeWholeWord,
    ChangeToPrevWordStart,
    ChangeToWordEnd,
    DeleteSelection,
    ChangeSelection,
    OpenLineBelow,
    OpenLineAbove,
    DeleteDown,
    DeleteUp,
    InsertTab,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum BehaviorAction {
    SwitchToInsertMode,
    SwitchToNormalMode,
    SwitchToVisualMode,
    OpenCommandList,
    ClearInput,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum DocumentAction {
    Movement(MovementAction),
    Edit(EditAction),
    Behavior(BehaviorAction),
}

// #[derive(Debug, Clone, Copy)]
// pub(crate) enum DocumentAction {
//     MoveDown,
//     MoveUp,
//     MoveRight,
//     MoveLeft,
//     MoveNextWordStart,
//     MovePrevWordStart,
//     MoveLineStart,
//     MoveLineEnd,
//     MoveLineFirstNonBlank,
//     SwitchToInsertMode,
//     SwitchToNormalMode,
//     SwitchToVisualMode,
//     InsertChar(char),
//     DeleteGrapheme,
//     InsertNewline,
//     MoveNextParagraph,
//     MovePrevParagraph,
//     GoToLastLine,
//     GoToNthOrFirstLine,
//     GoToNthOrLastLine,
//     DeleteWord,
//     ChangeWord,
//     DeleteToLineEnd,
//     ChangeToLineEnd,
//     DeleteToLineStart,
//     DeleteToLineFirstNonBlank,
//     DeleteLine,
//     DeleteWholeWord,
//     DeleteToPrevWordStart,
//     AppendText,
//     AppendTextLineEnd,
//     MoveWordEnd,
//     DeleteToWordEnd,
//     ChangeToLineStart,
//     ChangeToLineFirstNonBlank,
//     ChangeLine,
//     ChangeWholeWord,
//     ChangeToPrevWordStart,
//     ChangeToWordEnd,
//     DeleteSelection,
//     ChangeSelection,
//     ReverseSelection,
//     OpenLineBelow,
//     OpenLineAbove,
//     SelectCurrentWord,
//     DeleteDown,
//     DeleteUp,
//     OpenCommandList,
//     ClearInput,
//     InsertTab,
// }

impl DocumentAction {
    /// The desired cursor column should be reset on all potential cursor
    /// movements that aren't vertical (i.e. `j`/`k` commands) in order to
    /// prevent the cursor from jumping to unexpected columns when
    /// navigating.
    pub(crate) const fn should_reset_desired_column(self) -> bool {
        match self {
            Self::Movement(
                MovementAction::MoveRight
                | MovementAction::MoveLeft
                | MovementAction::MoveNextWordStart
                | MovementAction::MovePrevWordStart
                | MovementAction::MoveLineStart
                | MovementAction::MoveLineEnd
                | MovementAction::MoveLineFirstNonBlank
                | MovementAction::MoveNextParagraph
                | MovementAction::MovePrevParagraph
                | MovementAction::GoToLastLine
                | MovementAction::GoToNthOrLastLine
                | MovementAction::GoToNthOrFirstLine
                | MovementAction::MoveWordEnd
                | MovementAction::ReverseSelection
                | MovementAction::SelectCurrentWord,
            )
            | Self::Edit(
                EditAction::InsertChar(_)
                | EditAction::DeleteGrapheme
                | EditAction::InsertNewline
                | EditAction::DeleteWord
                | EditAction::ChangeWord
                | EditAction::DeleteToLineEnd
                | EditAction::ChangeToLineEnd
                | EditAction::DeleteToLineStart
                | EditAction::DeleteToLineFirstNonBlank
                | EditAction::DeleteLine
                | EditAction::DeleteWholeWord
                | EditAction::DeleteToPrevWordStart
                | EditAction::AppendText
                | EditAction::AppendTextLineEnd
                | EditAction::DeleteToWordEnd
                | EditAction::ChangeToLineStart
                | EditAction::ChangeToLineFirstNonBlank
                | EditAction::ChangeLine
                | EditAction::ChangeWholeWord
                | EditAction::ChangeToPrevWordStart
                | EditAction::ChangeToWordEnd
                | EditAction::DeleteSelection
                | EditAction::ChangeSelection
                | EditAction::OpenLineBelow
                | EditAction::OpenLineAbove
                | EditAction::DeleteDown
                | EditAction::DeleteUp
                | EditAction::InsertTab,
            ) => true,

            Self::Behavior(
                BehaviorAction::SwitchToInsertMode
                | BehaviorAction::SwitchToNormalMode
                | BehaviorAction::SwitchToVisualMode
                | BehaviorAction::OpenCommandList
                | BehaviorAction::ClearInput,
            )
            | Self::Movement(MovementAction::MoveDown | MovementAction::MoveUp) => false,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Movement(MovementAction::MoveDown) => "Move cursor down",
            Self::Movement(MovementAction::MoveUp) => "Move cursor up",
            Self::Movement(MovementAction::MoveRight) => "Move cursor right",
            Self::Movement(MovementAction::MoveLeft) => "Move cursor left",
            Self::Movement(MovementAction::MoveNextWordStart) => "Move to next word start",
            Self::Movement(MovementAction::MovePrevWordStart) => "Move to previous word start",
            Self::Movement(MovementAction::MoveLineStart) => "Move to start of line",
            Self::Movement(MovementAction::MoveLineEnd) => "Move to end of line",
            Self::Movement(MovementAction::MoveLineFirstNonBlank) => {
                "Move to first non-blank character"
            }
            Self::Behavior(BehaviorAction::SwitchToInsertMode) => "Switch to insert mode",
            Self::Behavior(BehaviorAction::SwitchToNormalMode) => "Switch to normal mode",
            Self::Behavior(BehaviorAction::SwitchToVisualMode) => "Switch to visual mode",
            Self::Edit(EditAction::InsertChar(_)) => "Insert character",
            Self::Edit(EditAction::DeleteGrapheme) => "Delete character",
            Self::Edit(EditAction::InsertNewline) => "Insert newline",
            Self::Movement(MovementAction::MoveNextParagraph) => "Move to next paragraph",
            Self::Movement(MovementAction::MovePrevParagraph) => "Move to previous paragraph",
            Self::Movement(MovementAction::GoToLastLine) => "Go to last line",
            Self::Movement(MovementAction::GoToNthOrFirstLine) => "Go to nth or first line",
            Self::Movement(MovementAction::GoToNthOrLastLine) => "Go to nth or last line",
            Self::Edit(EditAction::DeleteWord) => "Delete word",
            Self::Edit(EditAction::ChangeWord) => "Change word",
            Self::Edit(EditAction::DeleteToLineEnd) => "Delete to end of line",
            Self::Edit(EditAction::ChangeToLineEnd) => "Change to end of line",
            Self::Edit(EditAction::DeleteToLineStart) => "Delete to start of line",
            Self::Edit(EditAction::DeleteToLineFirstNonBlank) => {
                "Delete to first non-blank character"
            }
            Self::Edit(EditAction::DeleteLine) => "Delete line",
            Self::Edit(EditAction::DeleteWholeWord) => "Delete whole word",
            Self::Edit(EditAction::DeleteToPrevWordStart) => "Delete to previous word start",
            Self::Edit(EditAction::AppendText) => "Append text",
            Self::Edit(EditAction::AppendTextLineEnd) => "Append text at end of line",
            Self::Movement(MovementAction::MoveWordEnd) => "Move to end of word",
            Self::Edit(EditAction::DeleteToWordEnd) => "Delete to end of word",
            Self::Edit(EditAction::ChangeToLineStart) => "Change to start of line",
            Self::Edit(EditAction::ChangeToLineFirstNonBlank) => {
                "Change to first non-blank character"
            }
            Self::Edit(EditAction::ChangeLine) => "Change line",
            Self::Edit(EditAction::ChangeWholeWord) => "Change whole word",
            Self::Edit(EditAction::ChangeToPrevWordStart) => "Change to previous word start",
            Self::Edit(EditAction::ChangeToWordEnd) => "Change to end of word",
            Self::Edit(EditAction::DeleteSelection) => "Delete selection",
            Self::Edit(EditAction::ChangeSelection) => "Change selection",
            Self::Movement(MovementAction::ReverseSelection) => "Reverse selection",
            Self::Edit(EditAction::OpenLineBelow) => "Open line below",
            Self::Edit(EditAction::OpenLineAbove) => "Open line above",
            Self::Movement(MovementAction::SelectCurrentWord) => "Select current word",
            Self::Edit(EditAction::DeleteDown) => "Delete down",
            Self::Edit(EditAction::DeleteUp) => "Delete up",
            Self::Behavior(BehaviorAction::OpenCommandList) => "Open command list",
            Self::Behavior(BehaviorAction::ClearInput) => "Clear current input",
            Self::Edit(EditAction::InsertTab) => "Insert tab",
        }
    }
}

#[derive(Debug)]
pub(crate) struct KeySequence {
    keys: Vec<KeyBinding>,
    mode: Mode,
}

impl KeySequence {
    pub(crate) const fn new(mode: Mode) -> Self {
        Self {
            keys: Vec::new(),
            mode,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.keys.clear();
    }

    pub(crate) fn push(&mut self, key: KeyBinding) {
        self.keys.push(key);
    }

    pub(crate) const fn mode(&self) -> Mode {
        self.mode
    }

    pub(crate) const fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub(crate) fn parse(&self) -> (Vec<KeyBinding>, Option<NonZeroUsize>) {
        match self.mode {
            // TODO: ew
            Mode::Insert => (self.keys.clone(), None),
            Mode::Normal | Mode::Visual => {
                let mut keys = Vec::new();
                let mut count = None;

                for key in &self.keys {
                    match (key.code, key.modifiers) {
                        (KeyCode::Char(digit @ '0'..='9'), KeyModifiers::NONE) => {
                            let digit =
                                digit.to_digit(10).expect("`digit` is a valid digit") as usize;

                            if let Some(new_count) =
                                NonZeroUsize::new((count.map_or(0, NonZero::get) * 10) + digit)
                            {
                                count = Some(new_count);
                            } else {
                                keys.push(*key);
                            }
                        }
                        _ => keys.push(*key),
                    }
                }

                (keys, count)
            }
        }
    }
}

impl fmt::Display for KeySequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (keys, count) = self.parse();

        if let Some(count) = count {
            write!(f, "{count}")?;
        }

        for key in keys {
            write!(f, "{key}")?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_sequence() {
        let keys = KeySequence {
            keys: vec![key!('2'), key!('d'), key!('w')],
            mode: Mode::Normal,
        };

        assert_eq!(
            keys.parse(),
            (
                vec![key!('d'), key!('w')],
                Some(NonZeroUsize::new(2).unwrap())
            )
        );
    }

    #[test]
    fn parse_key_sequence_multiple_digits() {
        let keys = KeySequence {
            keys: vec![key!('2'), key!('7'), key!('d'), key!('w')],
            mode: Mode::Normal,
        };

        assert_eq!(
            keys.parse(),
            (
                vec![key!('d'), key!('w')],
                Some(NonZeroUsize::new(27).unwrap())
            )
        );
    }

    #[test]
    fn parse_key_sequence_leading_zero() {
        let keys = KeySequence {
            keys: vec![key!('0'), key!('2'), key!('g')],
            mode: Mode::Normal,
        };

        assert_eq!(
            keys.parse(),
            (
                vec![key!('0'), key!('g')],
                Some(NonZeroUsize::new(2).unwrap())
            )
        );
    }

    #[test]
    fn parse_key_sequence_multiple_leading_zero() {
        let keys = KeySequence {
            keys: vec![key!('0'), key!('0'), key!('7'), key!('g')],
            mode: Mode::Normal,
        };

        assert_eq!(
            keys.parse(),
            (
                vec![key!('0'), key!('0'), key!('g')],
                Some(NonZeroUsize::new(7).unwrap())
            )
        );
    }

    #[test]
    fn parse_key_sequence_non_leading_zero() {
        let keys = KeySequence {
            keys: vec![key!('1'), key!('0'), key!('c')],
            mode: Mode::Normal,
        };

        assert_eq!(
            keys.parse(),
            (vec![key!('c')], Some(NonZeroUsize::new(10).unwrap()))
        );
    }

    #[test]
    fn parse_key_sequence_interleaved_digits() {
        let keys = KeySequence {
            keys: vec![key!('g'), key!('1'), key!('0'), key!('w')],
            mode: Mode::Normal,
        };

        assert_eq!(
            keys.parse(),
            (
                vec![key!('g'), key!('w')],
                Some(NonZeroUsize::new(10).unwrap())
            )
        );
    }

    #[test]
    fn parse_key_sequence_leading_but_interleaved_zero() {
        let keys = KeySequence {
            keys: vec![
                key!('g'),
                key!('0'),
                key!('8'),
                key!('9'),
                key!('0'),
                key!('w'),
                key!('4'),
            ],
            mode: Mode::Normal,
        };

        assert_eq!(
            keys.parse(),
            (
                vec![key!('g'), key!('0'), key!('w')],
                Some(NonZeroUsize::new(8904).unwrap())
            )
        );
    }

    #[test]
    fn parse_key_sequence_visual_mode_has_count() {
        let keys = KeySequence {
            keys: vec![
                key!('g'),
                key!('0'),
                key!('8'),
                key!('9'),
                key!('0'),
                key!('w'),
                key!('4'),
            ],
            mode: Mode::Visual,
        };

        assert_eq!(
            keys.parse(),
            (
                vec![key!('g'), key!('0'), key!('w')],
                Some(NonZeroUsize::new(8904).unwrap())
            )
        );
    }

    #[test]
    fn parse_key_sequence_insert_mode_has_no_count() {
        let keys = KeySequence {
            keys: vec![
                key!('g'),
                key!('0'),
                key!('8'),
                key!('9'),
                key!('0'),
                key!('w'),
                key!('4'),
            ],
            mode: Mode::Insert,
        };

        assert_eq!(
            keys.parse(),
            (
                vec![
                    key!('g'),
                    key!('0'),
                    key!('8'),
                    key!('9'),
                    key!('0'),
                    key!('w'),
                    key!('4'),
                ],
                None,
            )
        );
    }
}
