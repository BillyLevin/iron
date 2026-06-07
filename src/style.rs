use bitflags::bitflags;
use crossterm::style::Color;

use crate::highlight::TokenKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Style {
    foreground: Option<Color>,
    background: Option<Color>,
    attributes: StyleAttributes,
}

impl Style {
    pub(crate) const BACKGROUND: Self = Self::new().with_bg(colors::LIGHT0);
    pub(crate) const COMMAND_LIST: Self = Self::new().with_bg(colors::LIGHT1);
    pub(crate) const COMMAND_LIST_BORDER: Self = Self::new().with_fg(colors::DARK1);
    pub(crate) const COMMAND_LIST_INPUT_TEXT: Self = Self::new().with_fg(colors::DARK1);
    pub(crate) const COMMAND_LIST_ITEM: Self = Self::new().with_fg(colors::DARK1);
    pub(crate) const COMMAND_LIST_ITEM_SELECTED: Self =
        Self::new().with_fg(colors::LIGHT0).with_bg(colors::DARK1);
    pub(crate) const DIFF_ADDED: Self = Self::new().with_fg(colors::FADED_GREEN);
    pub(crate) const DIFF_REMOVED: Self = Self::new().with_fg(colors::FADED_RED);
    pub(crate) const GUTTER: Self = Self::new().with_fg(colors::LIGHT4).with_bg(colors::LIGHT0);
    pub(crate) const GUTTER_SELECTED: Self = Self::new()
        .with_fg(colors::FADED_YELLOW)
        .with_bg(colors::LIGHT0);
    pub(crate) const HINTS: Self = Self::new().with_fg(colors::DARK1).with_bg(colors::LIGHT1);
    pub(crate) const STATUS_LINE: Self = Self::new().with_bg(colors::LIGHT1);
    pub(crate) const STATUS_LINE_ERROR: Self = Self::new().with_fg(colors::FADED_RED);
    pub(crate) const STATUS_LINE_MESSAGES: Self = Self::new().with_bg(colors::LIGHT0);
    pub(crate) const STATUS_LINE_MODE: Self = Self::new()
        .with_fg(colors::LIGHT0)
        .with_bg(colors::FADED_AQUA);
    pub(crate) const STATUS_LINE_TEXT: Self = Self::new().with_fg(colors::DARK1);
    pub(crate) const TEXT: Self = Self::new().with_fg(colors::DARK1);
    pub(crate) const TEXT_SELECTED: Self = Self::new().with_bg(colors::LIGHT3);

    pub(crate) const fn new() -> Self {
        Self {
            foreground: None,
            background: None,
            attributes: StyleAttributes::empty(),
        }
    }

    pub(crate) const fn with_fg(self, foreground: Color) -> Self {
        Self {
            foreground: Some(foreground),
            ..self
        }
    }

    pub(crate) const fn with_bg(self, background: Color) -> Self {
        Self {
            background: Some(background),
            ..self
        }
    }

    pub(crate) const fn with_attributes(self, attributes: StyleAttributes) -> Self {
        Self { attributes, ..self }
    }

    pub(crate) const fn foreground(self) -> Option<Color> {
        self.foreground
    }

    pub(crate) const fn background(self) -> Option<Color> {
        self.background
    }

    pub(crate) const fn attributes(&self) -> StyleAttributes {
        self.attributes
    }

    /// Merges `other` into `self`. For each style in `other`:
    /// - if style is `Some`, overwrite
    /// - otherwise, do not overwrite
    pub(crate) fn merge(self, other: Self) -> Self {
        Self {
            foreground: other.foreground.or(self.foreground),
            background: other.background.or(self.background),
            attributes: self.attributes | other.attributes,
        }
    }
}

impl From<TokenKind> for Style {
    fn from(kind: TokenKind) -> Self {
        match kind {
            TokenKind::Identifier | TokenKind::Unknown => Self::TEXT,
            TokenKind::Keyword => Self::new().with_fg(colors::FADED_RED),
            TokenKind::Lifetime | TokenKind::Macro => Self::new().with_fg(colors::FADED_AQUA),
            TokenKind::String => {
                Self::new()
                    .with_fg(colors::FADED_GREEN)
                    .with_attributes(StyleAttributes::Italic)
            }
            TokenKind::Type => Self::new().with_fg(colors::FADED_YELLOW),
            TokenKind::Comment => {
                Self::new()
                    .with_fg(colors::GRAY)
                    .with_attributes(StyleAttributes::Italic)
            }
            TokenKind::Operator | TokenKind::Punctuation => {
                Self::new().with_fg(colors::FADED_ORANGE)
            }
            TokenKind::Character | TokenKind::Number => Self::new().with_fg(colors::FADED_PURPLE),
            TokenKind::FunctionName => {
                Self::new()
                    .with_fg(colors::FADED_GREEN)
                    .with_attributes(StyleAttributes::Bold)
            }
            TokenKind::Property | TokenKind::PropertyAccess => {
                Self::new().with_fg(colors::FADED_BLUE)
            }
            TokenKind::Whitespace => Self::new(),
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct StyleAttributes: u8 {
        const Bold =  0b0000_0001;
        const Italic = 0b0000_0010;
        const Underlined = 0b0000_0100;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_original_when_other_is_empty() {
        let base = Style {
            foreground: Some(Color::Red),
            background: Some(Color::Black),
            attributes: StyleAttributes::Bold,
        };

        let other = Style {
            foreground: None,
            background: None,
            attributes: StyleAttributes::empty(),
        };

        let merged = base.merge(other);

        assert_eq!(merged, Style {
            foreground: Some(Color::Red),
            background: Some(Color::Black),
            attributes: StyleAttributes::Bold,
        });
    }

    #[test]
    fn merge_overwrites_foreground_when_present() {
        let base = Style {
            foreground: Some(Color::Red),
            background: Some(Color::Black),
            attributes: StyleAttributes::Italic,
        };

        let other = Style {
            foreground: Some(Color::Blue),
            background: None,
            attributes: StyleAttributes::empty(),
        };

        let merged = base.merge(other);

        assert_eq!(merged, Style {
            foreground: Some(Color::Blue),
            background: Some(Color::Black),
            attributes: StyleAttributes::Italic,
        });
    }

    #[test]
    fn merge_overwrites_background_when_present() {
        let base = Style {
            foreground: Some(Color::White),
            background: Some(Color::Black),
            attributes: StyleAttributes::Underlined,
        };

        let other = Style {
            foreground: None,
            background: Some(Color::Green),
            attributes: StyleAttributes::empty(),
        };

        let merged = base.merge(other);

        assert_eq!(merged, Style {
            foreground: Some(Color::White),
            background: Some(Color::Green),
            attributes: StyleAttributes::Underlined
        });
    }

    #[test]
    fn merge_overwrites_both_fields_when_present() {
        let base = Style {
            foreground: Some(Color::Red),
            background: Some(Color::Black),
            attributes: StyleAttributes::Bold,
        };

        let other = Style {
            foreground: Some(Color::Blue),
            background: Some(Color::White),
            attributes: StyleAttributes::Italic,
        };

        let merged = base.merge(other);

        assert_eq!(merged, Style {
            foreground: Some(Color::Blue),
            background: Some(Color::White),
            attributes: StyleAttributes::Bold | StyleAttributes::Italic,
        });
    }

    #[test]
    fn merge_can_fill_missing_fields() {
        let base = Style {
            foreground: None,
            background: Some(Color::Black),
            attributes: StyleAttributes::empty(),
        };

        let other = Style {
            foreground: Some(Color::Green),
            background: None,
            attributes: StyleAttributes::Bold,
        };

        let merged = base.merge(other);

        assert_eq!(merged, Style {
            foreground: Some(Color::Green),
            background: Some(Color::Black),
            attributes: StyleAttributes::Bold,
        });
    }

    #[test]
    fn merge_with_all_none_results_in_no_changes() {
        let base = Style {
            foreground: None,
            background: None,
            attributes: StyleAttributes::empty(),
        };

        let other = Style {
            foreground: None,
            background: None,
            attributes: StyleAttributes::empty(),
        };

        let merged = base.merge(other);

        assert_eq!(merged, Style {
            foreground: None,
            background: None,
            attributes: StyleAttributes::empty(),
        });
    }

    #[test]
    fn merge_combines_attributes() {
        let base = Style {
            foreground: None,
            background: None,
            attributes: StyleAttributes::Bold,
        };

        let other = Style {
            foreground: None,
            background: None,
            attributes: StyleAttributes::Italic | StyleAttributes::Underlined,
        };

        let merged = base.merge(other);

        assert_eq!(merged, Style {
            foreground: None,
            background: None,
            attributes: StyleAttributes::Bold
                | StyleAttributes::Italic
                | StyleAttributes::Underlined
        });
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

    pub(super) const FADED_BLUE: Color = Color::Rgb {
        r: 7,
        g: 102,
        b: 120,
    };

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

    pub(super) const GRAY: Color = Color::Rgb {
        r: 146,
        g: 131,
        b: 116,
    };
}
