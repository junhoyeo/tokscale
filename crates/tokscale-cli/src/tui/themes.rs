use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalColorMode {
    FullColor,
    Compatible,
}

impl TerminalColorMode {
    pub(crate) fn from_env<I, K, V>(env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut term = String::new();
        let mut term_program = String::new();
        let mut colorterm = String::new();
        let mut no_color = false;

        for (key, value) in env {
            let key = key.as_ref();
            let value = value.as_ref();
            match key {
                "TERM" => term = value.to_ascii_lowercase(),
                "TERM_PROGRAM" => term_program = value.to_ascii_lowercase(),
                "COLORTERM" => colorterm = value.to_ascii_lowercase(),
                "NO_COLOR" => no_color = true,
                _ => {}
            }
        }

        if no_color || term == "dumb" || term_program == "apple_terminal" {
            return Self::Compatible;
        }

        if matches!(colorterm.as_str(), "truecolor" | "24bit")
            || term.contains("truecolor")
            || term.contains("24bit")
        {
            return Self::FullColor;
        }

        Self::FullColor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeName {
    Green,
    Halloween,
    Teal,
    Blue,
    Pink,
    Purple,
    Orange,
    Monochrome,
    YlGnBu,
}

impl ThemeName {
    pub fn all() -> &'static [ThemeName] {
        &[
            ThemeName::Green,
            ThemeName::Halloween,
            ThemeName::Teal,
            ThemeName::Blue,
            ThemeName::Pink,
            ThemeName::Purple,
            ThemeName::Orange,
            ThemeName::Monochrome,
            ThemeName::YlGnBu,
        ]
    }

    pub fn next(self) -> ThemeName {
        let themes = Self::all();
        let idx = themes.iter().position(|&t| t == self).unwrap_or(0);
        themes[(idx + 1) % themes.len()]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ThemeName::Green => "green",
            ThemeName::Halloween => "halloween",
            ThemeName::Teal => "teal",
            ThemeName::Blue => "blue",
            ThemeName::Pink => "pink",
            ThemeName::Purple => "purple",
            ThemeName::Orange => "orange",
            ThemeName::Monochrome => "monochrome",
            ThemeName::YlGnBu => "ylgnbu",
        }
    }
}

impl std::str::FromStr for ThemeName {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "green" => Ok(ThemeName::Green),
            "halloween" => Ok(ThemeName::Halloween),
            "teal" => Ok(ThemeName::Teal),
            "blue" => Ok(ThemeName::Blue),
            "pink" => Ok(ThemeName::Pink),
            "purple" => Ok(ThemeName::Purple),
            "orange" => Ok(ThemeName::Orange),
            "monochrome" => Ok(ThemeName::Monochrome),
            "ylgnbu" => Ok(ThemeName::YlGnBu),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: ThemeName,
    pub colors: [Color; 5],
    pub background: Color,
    pub foreground: Color,
    pub border: Color,
    pub highlight: Color,
    pub muted: Color,
    pub accent: Color,
    pub selection: Color,
}

impl Theme {
    pub fn from_name_for_current_terminal(name: ThemeName) -> Self {
        Self::from_name_with_color_mode(name, TerminalColorMode::from_env(std::env::vars()))
    }

    pub(crate) fn from_name_with_color_mode(
        name: ThemeName,
        color_mode: TerminalColorMode,
    ) -> Self {
        let colors = match name {
            // Colors match frontend contribution graph palettes (higher grade = darker = more activity)
            ThemeName::Green => [
                Color::Rgb(22, 27, 34),    // grade0: empty
                Color::Rgb(155, 233, 168), // grade1: #9be9a8
                Color::Rgb(64, 196, 99),   // grade2: #40c463
                Color::Rgb(48, 161, 78),   // grade3: #30a14e
                Color::Rgb(33, 110, 57),   // grade4: #216e39
            ],
            ThemeName::Halloween => [
                Color::Rgb(22, 27, 34),   // grade0: empty
                Color::Rgb(255, 238, 74), // grade1: #FFEE4A
                Color::Rgb(255, 197, 1),  // grade2: #FFC501
                Color::Rgb(254, 150, 0),  // grade3: #FE9600
                Color::Rgb(3, 0, 28),     // grade4: #03001C
            ],
            ThemeName::Teal => [
                Color::Rgb(22, 27, 34),    // grade0: empty
                Color::Rgb(126, 229, 229), // grade1: #7ee5e5
                Color::Rgb(45, 197, 197),  // grade2: #2dc5c5
                Color::Rgb(13, 158, 158),  // grade3: #0d9e9e
                Color::Rgb(14, 109, 109),  // grade4: #0e6d6d
            ],
            ThemeName::Blue => [
                Color::Rgb(22, 27, 34),    // grade0: empty
                Color::Rgb(121, 184, 255), // grade1: #79b8ff
                Color::Rgb(56, 139, 253),  // grade2: #388bfd
                Color::Rgb(31, 111, 235),  // grade3: #1f6feb
                Color::Rgb(13, 65, 157),   // grade4: #0d419d
            ],
            ThemeName::Pink => [
                Color::Rgb(22, 27, 34),    // grade0: empty
                Color::Rgb(240, 181, 210), // grade1: #f0b5d2
                Color::Rgb(217, 97, 160),  // grade2: #d961a0
                Color::Rgb(191, 75, 138),  // grade3: #bf4b8a
                Color::Rgb(153, 40, 110),  // grade4: #99286e
            ],
            ThemeName::Purple => [
                Color::Rgb(22, 27, 34),    // grade0: empty
                Color::Rgb(205, 180, 255), // grade1: #cdb4ff
                Color::Rgb(163, 113, 247), // grade2: #a371f7
                Color::Rgb(137, 87, 229),  // grade3: #8957e5
                Color::Rgb(110, 64, 201),  // grade4: #6e40c9
            ],
            ThemeName::Orange => [
                Color::Rgb(22, 27, 34),    // grade0: empty
                Color::Rgb(255, 214, 153), // grade1: #ffd699
                Color::Rgb(255, 179, 71),  // grade2: #ffb347
                Color::Rgb(255, 140, 0),   // grade3: #ff8c00
                Color::Rgb(204, 85, 0),    // grade4: #cc5500
            ],
            ThemeName::Monochrome => [
                Color::Rgb(22, 27, 34),    // grade0: empty
                Color::Rgb(158, 158, 158), // grade1: #9e9e9e
                Color::Rgb(117, 117, 117), // grade2: #757575
                Color::Rgb(66, 66, 66),    // grade3: #424242
                Color::Rgb(33, 33, 33),    // grade4: #212121
            ],
            ThemeName::YlGnBu => [
                Color::Rgb(22, 27, 34),    // grade0: empty
                Color::Rgb(161, 218, 180), // grade1: #a1dab4
                Color::Rgb(65, 182, 196),  // grade2: #41b6c4
                Color::Rgb(44, 127, 184),  // grade3: #2c7fb8
                Color::Rgb(37, 52, 148),   // grade4: #253494
            ],
        };

        let mut theme = Self {
            name,
            colors,
            background: Color::Rgb(13, 17, 23),
            foreground: Color::Rgb(201, 209, 217),
            border: Color::Rgb(48, 54, 61),
            highlight: colors[4],
            muted: Color::Rgb(139, 148, 158),
            accent: Color::Cyan,
            selection: Color::Rgb(48, 54, 61),
        };

        if color_mode == TerminalColorMode::Compatible {
            theme.colors = [
                Color::Black,
                Color::DarkGray,
                Color::Gray,
                Color::White,
                Color::Cyan,
            ];
            theme.background = Color::Black;
            theme.foreground = Color::White;
            theme.border = Color::DarkGray;
            theme.highlight = Color::Cyan;
            theme.muted = Color::DarkGray;
            theme.accent = Color::Cyan;
            theme.selection = Color::DarkGray;
        }

        theme
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn apple_terminal_uses_compatible_color_mode() {
        let mode = TerminalColorMode::from_env(env(&[
            ("TERM_PROGRAM", "Apple_Terminal"),
            ("TERM", "xterm-256color"),
        ]));

        assert_eq!(mode, TerminalColorMode::Compatible);
    }

    #[test]
    fn vscode_truecolor_keeps_full_color_mode() {
        let mode = TerminalColorMode::from_env(env(&[
            ("TERM_PROGRAM", "vscode"),
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
        ]));

        assert_eq!(mode, TerminalColorMode::FullColor);
    }

    #[test]
    fn no_color_forces_compatible_color_mode() {
        let mode =
            TerminalColorMode::from_env(env(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")]));

        assert_eq!(mode, TerminalColorMode::Compatible);
    }

    #[test]
    fn compatible_theme_preserves_name_and_avoids_rgb_palette() {
        let theme =
            Theme::from_name_with_color_mode(ThemeName::Green, TerminalColorMode::Compatible);

        assert_eq!(theme.name, ThemeName::Green);
        assert!(theme
            .colors
            .iter()
            .all(|color| !matches!(color, Color::Rgb(..))));
        assert!(!matches!(theme.background, Color::Rgb(..)));
        assert_ne!(theme.background, Color::Reset);
        assert!(!matches!(theme.foreground, Color::Rgb(..)));
        assert!(!matches!(theme.selection, Color::Rgb(..)));
    }
}
