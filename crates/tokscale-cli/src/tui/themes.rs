use ratatui::style::{Color, Modifier, Style};

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
    Graphite,
    Lagoon,
    Dusk,
    TokyoNight,
    Catppuccin,
    Solarized,
    Gruvbox,
    GruvboxMaterial,
    OneDark,
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
            ThemeName::Graphite,
            ThemeName::Lagoon,
            ThemeName::Dusk,
            ThemeName::TokyoNight,
            ThemeName::Catppuccin,
            ThemeName::Solarized,
            ThemeName::Gruvbox,
            ThemeName::GruvboxMaterial,
            ThemeName::OneDark,
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
            ThemeName::Graphite => "graphite",
            ThemeName::Lagoon => "lagoon",
            ThemeName::Dusk => "dusk",
            ThemeName::TokyoNight => "tokyo-night",
            ThemeName::Catppuccin => "catppuccin",
            ThemeName::Solarized => "solarized",
            ThemeName::Gruvbox => "gruvbox",
            ThemeName::GruvboxMaterial => "gruvbox-material",
            ThemeName::OneDark => "one-dark",
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
            "graphite" => Ok(ThemeName::Graphite),
            "lagoon" => Ok(ThemeName::Lagoon),
            "dusk" => Ok(ThemeName::Dusk),
            "tokyo-night" => Ok(ThemeName::TokyoNight),
            "catppuccin" => Ok(ThemeName::Catppuccin),
            "solarized" => Ok(ThemeName::Solarized),
            "gruvbox" => Ok(ThemeName::Gruvbox),
            "gruvbox-material" => Ok(ThemeName::GruvboxMaterial),
            "one-dark" => Ok(ThemeName::OneDark),
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
    striped_row: Color,
    current_row: Color,
    color_mode: TerminalColorMode,
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
            ThemeName::Graphite => [
                Color::Rgb(24, 27, 34),    // grade0: empty
                Color::Rgb(148, 163, 184), // grade1: #94a3b8
                Color::Rgb(125, 211, 252), // grade2: #7dd3fc
                Color::Rgb(56, 189, 248),  // grade3: #38bdf8
                Color::Rgb(14, 116, 144),  // grade4: #0e7490
            ],
            ThemeName::Lagoon => [
                Color::Rgb(6, 32, 36),     // grade0: empty
                Color::Rgb(153, 246, 228), // grade1: #99f6e4
                Color::Rgb(94, 234, 212),  // grade2: #5eead4
                Color::Rgb(45, 212, 191),  // grade3: #2dd4bf
                Color::Rgb(15, 118, 110),  // grade4: #0f766e
            ],
            ThemeName::Dusk => [
                Color::Rgb(27, 24, 38),    // grade0: empty
                Color::Rgb(196, 181, 253), // grade1: #c4b5fd
                Color::Rgb(167, 139, 250), // grade2: #a78bfa
                Color::Rgb(139, 92, 246),  // grade3: #8b5cf6
                Color::Rgb(109, 40, 217),  // grade4: #6d28d9
            ],
            ThemeName::TokyoNight => [
                Color::Rgb(36, 40, 59),    // grade0: empty
                Color::Rgb(125, 207, 255), // grade1: #7dcfff
                Color::Rgb(122, 162, 247), // grade2: #7aa2f7
                Color::Rgb(187, 154, 247), // grade3: #bb9af7
                Color::Rgb(247, 118, 142), // grade4: #f7768e
            ],
            ThemeName::Catppuccin => [
                Color::Rgb(49, 50, 68),    // grade0: empty
                Color::Rgb(166, 227, 161), // grade1: #a6e3a1
                Color::Rgb(137, 180, 250), // grade2: #89b4fa
                Color::Rgb(203, 166, 247), // grade3: #cba6f7
                Color::Rgb(243, 139, 168), // grade4: #f38ba8
            ],
            ThemeName::Solarized => [
                Color::Rgb(7, 54, 66),    // grade0: empty
                Color::Rgb(42, 161, 152), // grade1: #2aa198
                Color::Rgb(38, 139, 210), // grade2: #268bd2
                Color::Rgb(181, 137, 0),  // grade3: #b58900
                Color::Rgb(220, 50, 47),  // grade4: #dc322f
            ],
            ThemeName::Gruvbox => [
                Color::Rgb(50, 48, 47),    // grade0: empty
                Color::Rgb(142, 192, 124), // grade1: #8ec07c
                Color::Rgb(131, 165, 152), // grade2: #83a598
                Color::Rgb(250, 189, 47),  // grade3: #fabd2f
                Color::Rgb(251, 73, 52),   // grade4: #fb4934
            ],
            ThemeName::GruvboxMaterial => [
                Color::Rgb(40, 40, 40),   // grade0: empty
                Color::Rgb(184, 187, 38), // grade1: #b8bb26
                Color::Rgb(250, 189, 47), // grade2: #fabd2f
                Color::Rgb(215, 153, 33), // grade3: #d79921
                Color::Rgb(251, 73, 52),  // grade4: #fb4934
            ],
            ThemeName::OneDark => [
                Color::Rgb(40, 44, 52),    // grade0: empty
                Color::Rgb(152, 195, 121), // grade1: #98c379
                Color::Rgb(97, 175, 239),  // grade2: #61afef
                Color::Rgb(198, 120, 221), // grade3: #c678dd
                Color::Rgb(224, 108, 117), // grade4: #e06c75
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
            striped_row: Color::Rgb(20, 24, 30),
            current_row: Color::Rgb(28, 42, 34),
            color_mode,
        };

        match name {
            ThemeName::Graphite => {
                theme.background = Color::Rgb(10, 12, 16);
                theme.foreground = Color::Rgb(226, 232, 240);
                theme.border = Color::Rgb(55, 65, 81);
                theme.muted = Color::Rgb(148, 163, 184);
                theme.accent = Color::Rgb(125, 211, 252);
                theme.selection = Color::Rgb(31, 41, 55);
                theme.striped_row = Color::Rgb(15, 18, 24);
                theme.current_row = Color::Rgb(24, 39, 38);
            }
            ThemeName::Lagoon => {
                theme.background = Color::Rgb(5, 20, 23);
                theme.foreground = Color::Rgb(216, 241, 238);
                theme.border = Color::Rgb(31, 83, 88);
                theme.muted = Color::Rgb(133, 177, 175);
                theme.accent = Color::Rgb(94, 234, 212);
                theme.selection = Color::Rgb(15, 54, 58);
                theme.striped_row = Color::Rgb(7, 26, 30);
                theme.current_row = Color::Rgb(18, 54, 42);
            }
            ThemeName::Dusk => {
                theme.background = Color::Rgb(17, 16, 26);
                theme.foreground = Color::Rgb(232, 226, 238);
                theme.border = Color::Rgb(63, 57, 82);
                theme.muted = Color::Rgb(166, 154, 184);
                theme.accent = Color::Rgb(196, 181, 253);
                theme.selection = Color::Rgb(43, 37, 58);
                theme.striped_row = Color::Rgb(22, 20, 32);
                theme.current_row = Color::Rgb(40, 45, 36);
            }
            ThemeName::TokyoNight => {
                theme.background = Color::Rgb(26, 27, 38); // #1a1b26
                theme.foreground = Color::Rgb(192, 202, 245); // #c0caf5
                theme.border = Color::Rgb(65, 72, 104); // #414868
                theme.muted = Color::Rgb(86, 95, 137); // #565f89
                theme.accent = Color::Rgb(187, 154, 247); // #bb9af7
                theme.selection = Color::Rgb(41, 46, 66); // #292e42
                theme.striped_row = Color::Rgb(31, 35, 53);
                theme.current_row = Color::Rgb(45, 74, 102);
            }
            ThemeName::Catppuccin => {
                theme.background = Color::Rgb(30, 30, 46); // #1e1e2e
                theme.foreground = Color::Rgb(205, 214, 244); // #cdd6f4
                theme.border = Color::Rgb(88, 91, 112); // #585b70
                theme.muted = Color::Rgb(166, 173, 200); // #a6adc8
                theme.accent = Color::Rgb(203, 166, 247); // #cba6f7
                theme.selection = Color::Rgb(69, 71, 90); // #45475a
                theme.striped_row = Color::Rgb(49, 50, 68);
                theme.current_row = Color::Rgb(69, 71, 90);
            }
            ThemeName::Solarized => {
                theme.background = Color::Rgb(0, 43, 54); // #002b36
                theme.foreground = Color::Rgb(147, 161, 161); // #93a1a1
                theme.border = Color::Rgb(88, 110, 117); // #586e75
                theme.muted = Color::Rgb(101, 123, 131); // #657b83
                theme.accent = Color::Rgb(181, 137, 0); // #b58900
                theme.selection = Color::Rgb(7, 54, 66); // #073642
                theme.striped_row = Color::Rgb(0, 52, 65);
                theme.current_row = Color::Rgb(7, 54, 66);
            }
            ThemeName::Gruvbox => {
                theme.background = Color::Rgb(40, 40, 40); // #282828
                theme.foreground = Color::Rgb(235, 219, 178); // #ebdbb2
                theme.border = Color::Rgb(102, 92, 84); // #665c54
                theme.muted = Color::Rgb(168, 153, 132); // #a89984
                theme.accent = Color::Rgb(250, 189, 47); // #fabd2f
                theme.selection = Color::Rgb(60, 56, 54); // #3c3836
                theme.striped_row = Color::Rgb(50, 48, 47);
                theme.current_row = Color::Rgb(69, 133, 136);
            }
            ThemeName::GruvboxMaterial => {
                theme.background = Color::Rgb(50, 48, 47); // #32302f
                theme.foreground = Color::Rgb(235, 219, 178); // #ebdbb2
                theme.border = Color::Rgb(102, 92, 84); // #665c54
                theme.muted = Color::Rgb(168, 153, 132); // #a89984
                theme.accent = Color::Rgb(215, 153, 33); // #d79921
                theme.selection = Color::Rgb(80, 73, 69); // #504945
                theme.striped_row = Color::Rgb(60, 56, 54);
                theme.current_row = Color::Rgb(69, 133, 136);
            }
            ThemeName::OneDark => {
                theme.background = Color::Rgb(40, 44, 52); // #282c34
                theme.foreground = Color::Rgb(171, 178, 191); // #abb2bf
                theme.border = Color::Rgb(92, 99, 112); // #5c6370
                theme.muted = Color::Rgb(130, 137, 151); // #828997
                theme.accent = Color::Rgb(97, 175, 239); // #61afef
                theme.selection = Color::Rgb(62, 68, 81); // #3e4451
                theme.striped_row = Color::Rgb(44, 49, 58);
                theme.current_row = Color::Rgb(58, 73, 94);
            }
            _ => {}
        }

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
            theme.striped_row = Color::Black;
            theme.current_row = Color::DarkGray;
        }

        theme
    }

    pub(crate) fn color(&self, color: Color) -> Color {
        match (self.color_mode, color) {
            (TerminalColorMode::Compatible, Color::Rgb(r, g, b)) => compatible_rgb(r, g, b),
            _ => color,
        }
    }

    pub(crate) fn metric_input_style(&self) -> Style {
        Style::default().fg(self.color(Color::Rgb(100, 200, 100)))
    }

    pub(crate) fn metric_output_style(&self) -> Style {
        Style::default().fg(self.color(Color::Rgb(200, 100, 100)))
    }

    pub(crate) fn metric_cache_read_style(&self) -> Style {
        Style::default().fg(self.color(Color::Rgb(100, 150, 200)))
    }

    pub(crate) fn metric_cache_write_style(&self) -> Style {
        Style::default().fg(self.color(Color::Rgb(200, 150, 100)))
    }

    pub(crate) fn metric_total_style(&self) -> Style {
        Style::default()
            .fg(self.foreground)
            .add_modifier(Modifier::BOLD)
    }

    pub(crate) fn secondary_text_style(&self) -> Style {
        Style::default().fg(self.color(Color::Rgb(170, 170, 170)))
    }

    pub(crate) fn subtle_text_style(&self) -> Style {
        Style::default().fg(self.color(Color::Rgb(102, 102, 102)))
    }

    pub(crate) fn striped_row_style(&self) -> Style {
        if self.color_mode == TerminalColorMode::Compatible {
            Style::default()
        } else {
            Style::default().bg(self.striped_row)
        }
    }

    pub(crate) fn current_row_style(&self) -> Style {
        if self.color_mode == TerminalColorMode::Compatible {
            Style::default().bg(self.selection)
        } else {
            Style::default().bg(self.current_row)
        }
    }
}

fn compatible_rgb(r: u8, g: u8, b: u8) -> Color {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);

    if max < 64 {
        return Color::Black;
    }

    if max.saturating_sub(min) < 40 {
        return if max < 160 {
            Color::DarkGray
        } else {
            Color::Gray
        };
    }

    if r >= g && r >= b {
        if g >= 150 {
            Color::Yellow
        } else if b >= 150 {
            Color::Magenta
        } else {
            Color::Red
        }
    } else if g >= r && g >= b {
        if b >= 150 {
            Color::Cyan
        } else {
            Color::Green
        }
    } else if r >= 150 {
        Color::Magenta
    } else if g >= 150 {
        Color::Cyan
    } else {
        Color::Blue
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
    fn theme_names_round_trip_through_settings_value() {
        for theme in ThemeName::all() {
            assert_eq!(theme.as_str().parse::<ThemeName>(), Ok(*theme));
        }
    }

    #[test]
    fn theme_pack_names_parse_from_settings_strings() {
        assert_eq!("tokyo-night".parse(), Ok(ThemeName::TokyoNight));
        assert_eq!("catppuccin".parse(), Ok(ThemeName::Catppuccin));
        assert_eq!("solarized".parse(), Ok(ThemeName::Solarized));
        assert_eq!("gruvbox".parse(), Ok(ThemeName::Gruvbox));
        assert_eq!("gruvbox-material".parse(), Ok(ThemeName::GruvboxMaterial));
        assert_eq!("one-dark".parse(), Ok(ThemeName::OneDark));
    }

    #[test]
    fn theme_pack_themes_are_listed_for_cycling() {
        let themes = ThemeName::all();

        assert!(themes.contains(&ThemeName::TokyoNight));
        assert!(themes.contains(&ThemeName::Catppuccin));
        assert!(themes.contains(&ThemeName::Solarized));
        assert!(themes.contains(&ThemeName::Gruvbox));
        assert!(themes.contains(&ThemeName::GruvboxMaterial));
        assert!(themes.contains(&ThemeName::OneDark));
    }

    #[test]
    fn gruvbox_material_differs_from_gruvbox() {
        let gruvbox =
            Theme::from_name_with_color_mode(ThemeName::Gruvbox, TerminalColorMode::FullColor);
        let material = Theme::from_name_with_color_mode(
            ThemeName::GruvboxMaterial,
            TerminalColorMode::FullColor,
        );

        assert_ne!(gruvbox.background, material.background);
        assert_ne!(gruvbox.accent, material.accent);
        assert_ne!(gruvbox.selection, material.selection);
        assert_ne!(gruvbox.colors, material.colors);
    }

    #[test]
    fn surface_themes_customize_background_and_row_colors() {
        let cases = [
            (
                ThemeName::Graphite,
                Color::Rgb(10, 12, 16),
                Color::Rgb(226, 232, 240),
                Color::Rgb(31, 41, 55),
                Color::Rgb(15, 18, 24),
                Color::Rgb(24, 39, 38),
            ),
            (
                ThemeName::Lagoon,
                Color::Rgb(5, 20, 23),
                Color::Rgb(216, 241, 238),
                Color::Rgb(15, 54, 58),
                Color::Rgb(7, 26, 30),
                Color::Rgb(18, 54, 42),
            ),
            (
                ThemeName::Dusk,
                Color::Rgb(17, 16, 26),
                Color::Rgb(232, 226, 238),
                Color::Rgb(43, 37, 58),
                Color::Rgb(22, 20, 32),
                Color::Rgb(40, 45, 36),
            ),
            (
                ThemeName::TokyoNight,
                Color::Rgb(26, 27, 38),
                Color::Rgb(192, 202, 245),
                Color::Rgb(41, 46, 66),
                Color::Rgb(31, 35, 53),
                Color::Rgb(45, 74, 102),
            ),
            (
                ThemeName::Catppuccin,
                Color::Rgb(30, 30, 46),
                Color::Rgb(205, 214, 244),
                Color::Rgb(69, 71, 90),
                Color::Rgb(49, 50, 68),
                Color::Rgb(69, 71, 90),
            ),
            (
                ThemeName::Solarized,
                Color::Rgb(0, 43, 54),
                Color::Rgb(147, 161, 161),
                Color::Rgb(7, 54, 66),
                Color::Rgb(0, 52, 65),
                Color::Rgb(7, 54, 66),
            ),
            (
                ThemeName::Gruvbox,
                Color::Rgb(40, 40, 40),
                Color::Rgb(235, 219, 178),
                Color::Rgb(60, 56, 54),
                Color::Rgb(50, 48, 47),
                Color::Rgb(69, 133, 136),
            ),
            (
                ThemeName::GruvboxMaterial,
                Color::Rgb(50, 48, 47),
                Color::Rgb(235, 219, 178),
                Color::Rgb(80, 73, 69),
                Color::Rgb(60, 56, 54),
                Color::Rgb(69, 133, 136),
            ),
            (
                ThemeName::OneDark,
                Color::Rgb(40, 44, 52),
                Color::Rgb(171, 178, 191),
                Color::Rgb(62, 68, 81),
                Color::Rgb(44, 49, 58),
                Color::Rgb(58, 73, 94),
            ),
        ];

        for (name, background, foreground, selection, striped, current) in cases {
            let theme = Theme::from_name_with_color_mode(name, TerminalColorMode::FullColor);

            assert_eq!(theme.background, background);
            assert_eq!(theme.foreground, foreground);
            assert_eq!(theme.selection, selection);
            assert_eq!(theme.striped_row_style().bg, Some(striped));
            assert_eq!(theme.current_row_style().bg, Some(current));
        }
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

    #[test]
    fn full_color_theme_preserves_rgb_accent_styles() {
        let theme = Theme::from_name_with_color_mode(ThemeName::Blue, TerminalColorMode::FullColor);

        assert_eq!(
            theme.metric_input_style().fg,
            Some(Color::Rgb(100, 200, 100))
        );
        assert_eq!(theme.striped_row_style().bg, Some(Color::Rgb(20, 24, 30)));
    }

    #[test]
    fn compatible_theme_downgrades_rgb_accent_styles() {
        let theme =
            Theme::from_name_with_color_mode(ThemeName::Blue, TerminalColorMode::Compatible);

        let styles = [
            theme.metric_input_style(),
            theme.metric_output_style(),
            theme.metric_cache_read_style(),
            theme.metric_cache_write_style(),
            theme.metric_total_style(),
            theme.secondary_text_style(),
            theme.subtle_text_style(),
            theme.striped_row_style(),
            theme.current_row_style(),
        ];

        for style in styles {
            assert!(
                !matches!(style.fg, Some(Color::Rgb(..))),
                "compatible foreground should not use RGB: {:?}",
                style.fg
            );
            assert!(
                !matches!(style.bg, Some(Color::Rgb(..))),
                "compatible background should not use RGB: {:?}",
                style.bg
            );
        }
    }
}
