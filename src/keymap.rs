use std::{
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

use crate::{
    cursor::MovementAction,
    document::Mode,
};

macro_rules! key {
    ($key:literal) => {
        KeyBinding::new(KeyCode::Char($key))
    };

    ($key:ident) => {
        KeyBinding::new(KeyCode::$key)
    };
}

macro_rules! action {
    (Movement($action:ident)) => {
        DocumentAction::Movement(MovementAction::$action)
    };

    (Edit($action:ident)) => {
        DocumentAction::Edit(EditAction::$action)
    };

    (Behavior($action:ident)) => {
        DocumentAction::Behavior(BehaviorAction::$action)
    };
}

macro_rules! keymap {
    (@node { $($inner:tt)* }) => {
        keymap!($($inner)*)
    };

    (@node $kind:ident $action:ident) => {
        KeyMap::Action(action!($kind($action)))
    };

    ($($key:tt => $node:tt $(($action:ident))?),* $(,)?) => {
        KeyMap::BindingPart {
            map: &[$((key!($key), keymap!(@node $node $($action)?))),*],
        }
    };
}

static NORMAL: KeyMap = keymap! {
    'j' => Movement(MoveDown),
    'k' => Movement(MoveUp),
    'l' => Movement(MoveRight),
    'h' => Movement(MoveLeft),
    'w' => Movement(MoveNextWordStart),
    'b' => Movement(MovePrevWordStart),
    'i' => Behavior(SwitchToInsertMode),
    'v' => Behavior(SwitchToVisualMode),
    '$' => Movement(MoveLineEnd),
    '0' => Movement(MoveLineStart),
    '^' => Movement(MoveLineFirstNonBlank),
    '}' => Movement(MoveNextParagraph),
    '{' => Movement(MovePrevParagraph),
    'a' => Behavior(AppendText),
    'A' => Edit(AppendTextLineEnd),
    'e' => Movement(MoveWordEnd),
    'o' => Edit(OpenLineBelow),
    'O' => Edit(OpenLineAbove),
    ':' => Behavior(OpenCommandList),
    'z' => {
        'z' => Behavior(VerticallyCenter),
    },
    'g' => {
        'e' => Movement(GoToLastLine),
        'g' => Movement(GoToNthOrFirstLine),
    },
    'G' => Movement(GoToNthOrLastLine),
    'd' => {
        'w' => Edit(DeleteWord),
        '$' => Edit(DeleteToLineEnd),
        '0' => Edit(DeleteToLineStart),
        '^' => Edit(DeleteToLineFirstNonBlank),
        'd' => Edit(DeleteLine),
        'i' => {
            'w' => Edit(DeleteWholeWord),
        },
        'b' => Edit(DeleteToPrevWordStart),
        'e' => Edit(DeleteToWordEnd),
        'j' => Edit(DeleteDown),
        'k' => Edit(DeleteUp),
    },
    'c' => {
        'w' => Edit(ChangeWord),
        '$' => Edit(ChangeToLineEnd),
        '0' => Edit(ChangeToLineStart),
        '^' => Edit(ChangeToLineFirstNonBlank),
        'c' => Edit(ChangeLine),
        'i' => {
            'w' => Edit(ChangeWholeWord),
        },
        'b' => Edit(ChangeToPrevWordStart),
        'e' => Edit(ChangeToWordEnd),
    },
    ' ' => {
        'p' => Behavior(OpenFilePicker),
        'e' => Behavior(OpenDiagnosticList),
    },
    Esc => Behavior(ClearInput),
    '%' => Movement(GoToPairMatch),
};

static INSERT: KeyMap = keymap! {
    Backspace => Edit(DeleteGrapheme),
    Esc => Behavior(SwitchToNormalMode),
    Enter => Edit(InsertNewline),
    Tab => Edit(InsertTab),
};

static VISUAL: KeyMap = keymap! {
    Esc => Behavior(SwitchToNormalMode),
    'j' => Movement(MoveDown),
    'k' => Movement(MoveUp),
    'l' => Movement(MoveRight),
    'h' => Movement(MoveLeft),
    'w' => Movement(MoveNextWordStart),
    'b' => Movement(MovePrevWordStart),
    '$' => Movement(MoveLineEnd),
    '0' => Movement(MoveLineStart),
    '^' => Movement(MoveLineFirstNonBlank),
    '}' => Movement(MoveNextParagraph),
    '{' => Movement(MovePrevParagraph),
    'e' => Movement(MoveWordEnd),
    'o' => Movement(ReverseSelection),
    'g' => {
        'e' => Movement(GoToLastLine),
        'g' => Movement(GoToNthOrFirstLine),
    },
    'G' => Movement(GoToNthOrLastLine),
    'd' => Edit(DeleteSelection),
    'c' => Edit(ChangeSelection),
    'i' => {
        'w' => Movement(SelectCurrentWord),
    },
    ':' => Behavior(OpenCommandList),
    'z' => {
        'z' => Behavior(VerticallyCenter),
    },
    '%' => Movement(GoToPairMatch),
};

#[derive(Debug)]
pub(crate) enum KeyMap {
    BindingPart { map: &'static [(KeyBinding, Self)] },
    Action(DocumentAction),
}

impl KeyMap {
    pub(crate) const fn for_mode(mode: Mode) -> &'static Self {
        match mode {
            Mode::Normal => &NORMAL,
            Mode::Insert => &INSERT,
            Mode::Visual => &VISUAL,
        }
    }

    pub(crate) fn get(&self, keys: &[KeyBinding]) -> Option<&Self> {
        let mut current = self;

        for key in keys {
            current = match *current {
                Self::BindingPart { map } => {
                    map.iter()
                        .find_map(|&(ref binding, ref keymap)| (binding == key).then_some(keymap))
                }
                Self::Action(_) => None,
            }?;
        }

        Some(current)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyBinding {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyBinding {
    const fn new(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    pub(crate) const fn code(&self) -> KeyCode {
        self.code
    }

    pub(crate) const fn modifiers(&self) -> KeyModifiers {
        self.modifiers
    }
}

impl From<KeyEvent> for KeyBinding {
    fn from(event: KeyEvent) -> Self {
        let mut modifiers = event.modifiers;

        // we don't need to differentiate between shift/no shift for capital
        // letters
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
    OpenFilePicker,
    AppendText,
    OpenDiagnosticList,
    VerticallyCenter,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum DocumentAction {
    Movement(MovementAction),
    Edit(EditAction),
    Behavior(BehaviorAction),
}

impl DocumentAction {
    /// The desired cursor column should be reset on all potential cursor
    /// movements that aren't vertical (i.e. `j`/`k` commands) in order to
    /// prevent the cursor from jumping to unexpected columns when
    /// navigating.
    pub(crate) const fn should_reset_desired_column(self) -> bool {
        match self {
            Self::Movement(movement_action) => movement_action.should_reset_desired_column(),

            Self::Edit(
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
            )
            | Self::Behavior(BehaviorAction::AppendText) => true,

            Self::Behavior(
                BehaviorAction::SwitchToInsertMode
                | BehaviorAction::SwitchToNormalMode
                | BehaviorAction::SwitchToVisualMode
                | BehaviorAction::OpenCommandList
                | BehaviorAction::ClearInput
                | BehaviorAction::OpenFilePicker
                | BehaviorAction::OpenDiagnosticList
                | BehaviorAction::VerticallyCenter,
            ) => false,
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
            Self::Behavior(BehaviorAction::AppendText) => "Append text",
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
            Self::Behavior(BehaviorAction::VerticallyCenter) => "Center cursor vertically",
            Self::Behavior(BehaviorAction::OpenFilePicker) => "Open file picker",
            Self::Movement(MovementAction::GoToPairMatch) => "Go to pair match",
            Self::Behavior(BehaviorAction::OpenDiagnosticList) => "Open line diagnostics",
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
    fn no_duplicate_bindings() {
        fn check(keymap: &KeyMap, path: &[KeyBinding]) {
            let KeyMap::BindingPart { map } = *keymap else {
                return;
            };

            for (index, &(key, ref node)) in map.iter().enumerate() {
                let path = [path, &[key]].concat();

                assert!(
                    map[..index].iter().all(|&(other, _)| other != key),
                    "duplicate binding for {path:?}",
                );

                check(node, &path);
            }
        }

        for mode in [Mode::Normal, Mode::Insert, Mode::Visual] {
            check(KeyMap::for_mode(mode), &[]);
        }
    }

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
