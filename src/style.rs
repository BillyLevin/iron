use crossterm::style::Color;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Style {
    foreground: Option<Color>,
    background: Option<Color>,
}

impl Style {
    pub(crate) const BACKGROUND: Self = Self {
        foreground: None,
        background: Some(colors::LIGHT0),
    };
    pub(crate) const COMMAND_LIST: Self = Self {
        foreground: None,
        background: Some(colors::LIGHT1),
    };
    pub(crate) const COMMAND_LIST_BORDER: Self = Self {
        foreground: Some(colors::DARK1),
        background: None,
    };
    pub(crate) const COMMAND_LIST_INPUT_TEXT: Self = Self {
        foreground: Some(colors::DARK1),
        background: None,
    };
    pub(crate) const COMMAND_LIST_ITEM: Self = Self {
        foreground: Some(colors::DARK1),
        background: None,
    };
    pub(crate) const COMMAND_LIST_ITEM_SELECTED: Self = Self {
        foreground: Some(colors::LIGHT0),
        background: Some(colors::DARK1),
    };
    pub(crate) const GUTTER: Self = Self {
        foreground: Some(colors::LIGHT4),
        background: Some(colors::LIGHT0),
    };
    pub(crate) const GUTTER_SELECTED: Self = Self {
        foreground: Some(colors::FADED_YELLOW),
        background: Some(colors::LIGHT0),
    };
    pub(crate) const HINTS: Self = Self {
        foreground: Some(colors::DARK1),
        background: Some(colors::LIGHT1),
    };
    pub(crate) const STATUS_LINE: Self = Self {
        foreground: None,
        background: Some(colors::LIGHT1),
    };
    pub(crate) const STATUS_LINE_ERROR: Self = Self {
        foreground: Some(colors::FADED_RED),
        background: None,
    };
    pub(crate) const STATUS_LINE_MESSAGES: Self = Self {
        foreground: None,
        background: Some(colors::LIGHT0),
    };
    pub(crate) const STATUS_LINE_MODE: Self = Self {
        foreground: Some(colors::LIGHT0),
        background: Some(colors::FADED_AQUA),
    };
    pub(crate) const STATUS_LINE_TEXT: Self = Self {
        foreground: Some(colors::DARK1),
        background: None,
    };
    pub(crate) const TEXT: Self = Self {
        foreground: Some(colors::DARK1),
        background: None,
    };
    pub(crate) const TEXT_SELECTED: Self = Self {
        foreground: Some(colors::DARK1),
        background: Some(colors::LIGHT3),
    };

    pub(crate) const fn foreground(self) -> Option<Color> {
        self.foreground
    }

    pub(crate) const fn background(self) -> Option<Color> {
        self.background
    }
}

/// <https://github.com/ellisonleao/gruvbox.nvim/blob/main/lua/gruvbox.lua>.
mod colors {

    use crossterm::style::Color;

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const DARK0: Color = Color::Rgb {
        r: 29,
        g: 32,
        b: 33,
    };

    pub(super) const DARK1: Color = Color::Rgb {
        r: 60,
        g: 56,
        b: 54,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const DARK2: Color = Color::Rgb {
        r: 80,
        g: 73,
        b: 69,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const DARK3: Color = Color::Rgb {
        r: 102,
        g: 92,
        b: 84,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const DARK4: Color = Color::Rgb {
        r: 124,
        g: 111,
        b: 100,
    };

    pub(super) const LIGHT0: Color = Color::Rgb {
        r: 249,
        g: 245,
        b: 215,
    };

    pub(super) const LIGHT1: Color = Color::Rgb {
        r: 235,
        g: 219,
        b: 178,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const LIGHT2: Color = Color::Rgb {
        r: 213,
        g: 196,
        b: 161,
    };

    pub(super) const LIGHT3: Color = Color::Rgb {
        r: 189,
        g: 174,
        b: 147,
    };

    pub(super) const LIGHT4: Color = Color::Rgb {
        r: 168,
        g: 153,
        b: 132,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const BRIGHT_RED: Color = Color::Rgb {
        r: 251,
        g: 73,
        b: 52,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const BRIGHT_GREEN: Color = Color::Rgb {
        r: 184,
        g: 187,
        b: 38,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const BRIGHT_YELLOW: Color = Color::Rgb {
        r: 250,
        g: 189,
        b: 47,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const BRIGHT_BLUE: Color = Color::Rgb {
        r: 131,
        g: 165,
        b: 152,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const BRIGHT_PURPLE: Color = Color::Rgb {
        r: 211,
        g: 134,
        b: 155,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const BRIGHT_AQUA: Color = Color::Rgb {
        r: 142,
        g: 192,
        b: 124,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const BRIGHT_ORANGE: Color = Color::Rgb {
        r: 254,
        g: 128,
        b: 25,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const NEUTRAL_RED: Color = Color::Rgb {
        r: 204,
        g: 36,
        b: 29,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const NEUTRAL_GREEN: Color = Color::Rgb {
        r: 152,
        g: 151,
        b: 26,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const NEUTRAL_YELLOW: Color = Color::Rgb {
        r: 215,
        g: 153,
        b: 33,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const NEUTRAL_BLUE: Color = Color::Rgb {
        r: 69,
        g: 133,
        b: 136,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const NEUTRAL_PURPLE: Color = Color::Rgb {
        r: 177,
        g: 98,
        b: 134,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const NEUTRAL_AQUA: Color = Color::Rgb {
        r: 104,
        g: 157,
        b: 106,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const NEUTRAL_ORANGE: Color = Color::Rgb {
        r: 214,
        g: 93,
        b: 14,
    };

    pub(super) const FADED_RED: Color = Color::Rgb { r: 157, g: 0, b: 6 };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const FADED_GREEN: Color = Color::Rgb {
        r: 121,
        g: 116,
        b: 14,
    };

    pub(super) const FADED_YELLOW: Color = Color::Rgb {
        r: 181,
        g: 118,
        b: 20,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const FADED_BLUE: Color = Color::Rgb {
        r: 7,
        g: 102,
        b: 120,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const FADED_PURPLE: Color = Color::Rgb {
        r: 143,
        g: 63,
        b: 113,
    };

    pub(super) const FADED_AQUA: Color = Color::Rgb {
        r: 66,
        g: 123,
        b: 88,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const FADED_ORANGE: Color = Color::Rgb {
        r: 175,
        g: 58,
        b: 3,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const DARK_RED: Color = Color::Rgb {
        r: 121,
        g: 35,
        b: 41,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const LIGHT_RED: Color = Color::Rgb {
        r: 252,
        g: 150,
        b: 144,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const DARK_GREEN: Color = Color::Rgb {
        r: 90,
        g: 99,
        b: 58,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const LIGHT_GREEN: Color = Color::Rgb {
        r: 211,
        g: 214,
        b: 165,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const DARK_AQUA: Color = Color::Rgb {
        r: 62,
        g: 73,
        b: 52,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const LIGHT_AQUA: Color = Color::Rgb {
        r: 230,
        g: 233,
        b: 193,
    };

    #[expect(unused, reason = "unused colours may be useful in future")]
    pub(super) const GRAY: Color = Color::Rgb {
        r: 146,
        g: 131,
        b: 116,
    };
}
