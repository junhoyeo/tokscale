#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRoot {
    Home,
    ReasonixHome,
    XdgData,
    Config,
    /// The per-user application data directory, resolved via the `dirs` crate:
    /// `%APPDATA%` on Windows, `~/Library/Application Support` on macOS, and
    /// the XDG config home on Linux. When an explicit home is supplied — or,
    /// on Windows, when the resolved home is not the Win32 profile — this root
    /// is derived from that home using the matching platform convention; see
    /// [`app_data_follows_home`].
    AppData,
    EnvVar {
        var: &'static str,
        fallback_relative: &'static str,
    },
}

/// Join `home_dir` and a `/`-joined relative literal with native separators
/// throughout. `Path::join` only normalizes the junction; the relative half's
/// own `/` separators would survive untouched on Windows (#1048).
fn join_home(home_dir: &str, relative: &str) -> String {
    let mut path = std::path::PathBuf::from(home_dir);
    for component in std::path::Path::new(relative).components() {
        path.push(component.as_os_str());
    }
    path.to_string_lossy().into_owned()
}

/// Whether an [`PathRoot::AppData`] scan must be derived from `home_dir`
/// rather than from the platform's own app-data lookup, even under env roots.
///
/// Only Windows can disagree with `home_dir`. `dirs::config_dir()` is
/// `SHGetKnownFolderPath(FOLDERID_RoamingAppData)` there — a Win32 known
/// folder that no environment variable can redirect, not even `%APPDATA%`.
/// macOS and Linux resolve it from `$HOME` / `$XDG_CONFIG_HOME`, so they
/// already follow whatever home `paths::home_dir()` handed this call.
///
/// That asymmetry is the one `paths::home_dir` was written to close for
/// `dirs::home_dir()` in #997: every home-rooted scan target obeys a
/// redirected `HOME`, so an AppData-rooted client must not be the single
/// target that keeps reading the machine's real profile. Cherry Studio is
/// currently that client, and on Windows its transcripts were discovered
/// under the live profile no matter where the caller pointed the home.
///
/// The known-folder answer still wins when `home_dir` *is* the Win32 profile,
/// because folder redirection and roaming profiles can legitimately place
/// `%APPDATA%` outside the profile directory; only a home that actually names
/// somewhere else overrides it. A non-absolute `home_dir` (a POSIX-shaped
/// `HOME` from Git Bash, a drive-relative `C:temp`) is never treated as a
/// redirect, matching `paths::home_dir`, which rejects those same shapes
/// because `Path` resolves them against ambient state.
fn app_data_follows_home(home_dir: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        let home = std::path::Path::new(home_dir);
        if !home.is_absolute() {
            return false;
        }
        match dirs::home_dir() {
            Some(profile) => !same_windows_dir(home, &profile),
            None => true,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = home_dir;
        false
    }
}

/// Whether two absolute Windows paths name the same directory.
///
/// A lexical comparison is not enough here, and getting it wrong is not
/// symmetric: a home that merely *spells* the profile differently would be
/// misread as a redirect, and on a machine whose `FOLDERID_RoamingAppData` is
/// itself redirected that would move the scan to `<profile>\AppData\Roaming`
/// and lose the user's transcripts. Windows offers at least three such
/// spellings — different casing (`c:\users\me`), the 8.3 alias
/// (`C:\Users\RUNNER~1`), and a junction or symlink pointing at the profile —
/// and `Path`'s component comparison treats all three as different paths.
///
/// `canonicalize` resolves every one of them to the same `\\?\`-verbatim
/// path. It touches the filesystem and fails on a path that does not exist, so
/// fall back to the lexical comparison when either side cannot be
/// canonicalized: a home that is not on disk cannot be the live profile under
/// another spelling, and the fallback then correctly reports a redirect.
#[cfg(target_os = "windows")]
fn same_windows_dir(home: &std::path::Path, profile: &std::path::Path) -> bool {
    match (std::fs::canonicalize(home), std::fs::canonicalize(profile)) {
        (Ok(home_real), Ok(profile_real)) => home_real == profile_real,
        _ => home == profile,
    }
}

impl PathRoot {
    pub fn resolve_with_env_strategy(&self, home_dir: &str, use_env_roots: bool) -> String {
        match self {
            PathRoot::Home => home_dir.to_string(),
            PathRoot::ReasonixHome => {
                if use_env_roots {
                    if let Some(state_home) =
                        clean_reasonix_env_dir("REASONIX_STATE_HOME", home_dir)
                    {
                        return state_home;
                    }
                    if let Some(home) = clean_reasonix_env_dir("REASONIX_HOME", home_dir) {
                        return home;
                    }
                }
                #[cfg(target_os = "windows")]
                {
                    if use_env_roots {
                        if let Some(config_dir) = dirs::config_dir() {
                            return config_dir.join("reasonix").to_string_lossy().into_owned();
                        }
                    }
                    std::path::Path::new(home_dir)
                        .join("AppData")
                        .join("Roaming")
                        .join("reasonix")
                        .to_string_lossy()
                        .into_owned()
                }
                #[cfg(not(target_os = "windows"))]
                {
                    join_home(home_dir, ".reasonix")
                }
            }
            PathRoot::XdgData => {
                if use_env_roots {
                    std::env::var("XDG_DATA_HOME")
                        .unwrap_or_else(|_| join_home(home_dir, ".local/share"))
                } else {
                    join_home(home_dir, ".local/share")
                }
            }
            PathRoot::Config => {
                if use_env_roots {
                    if let Some(custom) = std::env::var_os("TOKSCALE_CONFIG_DIR") {
                        if !custom.is_empty() {
                            return custom.to_string_lossy().into_owned();
                        }
                    }

                    #[cfg(target_os = "linux")]
                    if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
                        return format!("{xdg_config_home}/tokscale");
                    }

                    // Match paths::get_config_dir() so default Windows scans
                    // read the same %APPDATA% root used by cache writers.
                    #[cfg(target_os = "windows")]
                    if let Some(dir) = dirs::config_dir() {
                        return dir.join("tokscale").to_string_lossy().into_owned();
                    }
                }

                #[cfg(target_os = "windows")]
                if !use_env_roots {
                    return std::path::Path::new(home_dir)
                        .join("AppData/Roaming/tokscale")
                        .to_string_lossy()
                        .into_owned();
                }

                join_home(home_dir, ".config/tokscale")
            }
            PathRoot::AppData => {
                if use_env_roots && !app_data_follows_home(home_dir) {
                    if let Some(dir) = dirs::config_dir() {
                        return dir.to_string_lossy().into_owned();
                    }
                }
                // Without env roots (tests, explicit `--home`) the other roots
                // resolve under the given home; follow the same convention so
                // an AppData-rooted client cannot leak the machine's real
                // per-user data into a hermetic scan.
                #[cfg(target_os = "windows")]
                {
                    join_home(home_dir, "AppData/Roaming")
                }
                #[cfg(target_os = "macos")]
                {
                    join_home(home_dir, "Library/Application Support")
                }
                #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
                {
                    join_home(home_dir, ".config")
                }
            }
            PathRoot::EnvVar {
                var,
                fallback_relative,
            } => {
                if use_env_roots {
                    let val = std::env::var(var).unwrap_or_default();
                    if val.trim().is_empty() {
                        join_home(home_dir, fallback_relative)
                    } else {
                        val
                    }
                } else {
                    join_home(home_dir, fallback_relative)
                }
            }
        }
    }

    pub fn resolve(&self, home_dir: &str) -> String {
        self.resolve_with_env_strategy(home_dir, true)
    }
}

fn clean_reasonix_env_dir(name: &str, home_dir: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let value = expand_reasonix_env_vars(value.trim());
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let path = if value == "~" {
        std::path::PathBuf::from(home_dir)
    } else if let Some(relative) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        std::path::PathBuf::from(join_home(home_dir, relative))
    } else {
        std::path::PathBuf::from(value)
    };

    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    Some(path.to_string_lossy().into_owned())
}

// Match Reasonix's config expansion for ${VAR} and ${VAR:-default}. This must
// happen before tilde and relative-path handling because either expansion may
// produce one of those forms.
fn expand_reasonix_env_vars(value: &str) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut remainder = value;

    while let Some(start) = remainder.find("${") {
        expanded.push_str(&remainder[..start]);
        let reference = &remainder[start + 2..];
        let Some(end) = reference.find('}') else {
            expanded.push_str(&remainder[start..]);
            return expanded;
        };

        let expression = &reference[..end];
        let (name, default) = expression
            .split_once(":-")
            .map_or((expression, None), |(name, default)| (name, Some(default)));
        let is_valid_name = name.chars().enumerate().all(|(index, character)| {
            (character == '_' || character.is_ascii_alphabetic())
                || (index > 0 && character.is_ascii_digit())
        });

        if is_valid_name && !name.is_empty() {
            match std::env::var(name) {
                Ok(env_value) if !env_value.is_empty() => expanded.push_str(&env_value),
                _ => expanded.push_str(default.unwrap_or_default()),
            }
        } else {
            expanded.push_str("${");
            expanded.push_str(expression);
            expanded.push('}');
        }
        remainder = &reference[end + 1..];
    }

    expanded.push_str(remainder);
    expanded
}

#[derive(Debug, Clone)]
pub struct ClientDef {
    pub id: &'static str,
    pub root: PathRoot,
    pub relative_path: &'static str,
    pub pattern: &'static str,
    pub headless: bool,
    pub parse_local: bool,
    pub submit_default: bool,
}

impl ClientDef {
    pub fn resolve_path_with_env_strategy(&self, home_dir: &str, use_env_roots: bool) -> String {
        let root = self.root.resolve_with_env_strategy(home_dir, use_env_roots);
        if self.relative_path.is_empty() {
            return root;
        }
        // Join component-by-component instead of hand-concatenating
        // "{root}/{relative}": a hardcoded `/` — and even `Path::join`, which
        // only normalizes the junction — leaves the relative half's own `/`
        // separators untouched on Windows, producing mixed-separator paths
        // (`C:\Users\me/.codex/sessions`) that reached user-facing
        // `clients --json` output (#1048). Pushing each component yields
        // native separators throughout on every platform.
        let mut path = std::path::PathBuf::from(&root);
        for component in std::path::Path::new(self.relative_path).components() {
            path.push(component.as_os_str());
        }
        path.to_string_lossy().into_owned()
    }

    pub fn resolve_path(&self, home_dir: &str) -> String {
        self.resolve_path_with_env_strategy(home_dir, true)
    }
}

macro_rules! define_clients {
    ( $( $variant:ident = $index:expr => { id: $id:expr, display: $display:expr, logo: $logo:expr, root: $root:expr, relative: $rel:expr, pattern: $pat:expr, headless: $hl:expr, parse_local: $pl:expr, submit_default: $sd:expr } ),+ $(,)? ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(usize)]
        pub enum ClientId {
            $( $variant = $index ),+
        }

        impl ClientId {
            pub const COUNT: usize = [ $( $index ),+ ].len();
            pub const ALL: [ClientId; Self::COUNT] = [ $( ClientId::$variant ),+ ];

            pub fn data(&self) -> &'static ClientDef {
                &CLIENTS[*self as usize]
            }

            pub fn as_str(&self) -> &'static str {
                self.data().id
            }

            pub fn display_name(&self) -> &'static str {
                CLIENT_DISPLAY_NAMES[*self as usize]
            }

            pub fn logo_url(&self) -> Option<&'static str> {
                CLIENT_LOGO_URLS[*self as usize]
            }

            pub fn file_pattern(&self) -> &'static str {
                self.data().pattern
            }

            pub fn supports_headless(&self) -> bool {
                self.data().headless
            }

            pub fn parse_local(&self) -> bool {
                self.data().parse_local
            }

            pub fn submit_default(&self) -> bool {
                self.data().submit_default
            }

            pub fn iter() -> impl Iterator<Item = ClientId> {
                Self::ALL.iter().copied()
            }

            #[allow(clippy::should_implement_trait)]
            pub fn from_str(s: &str) -> Option<ClientId> {
                Self::ALL.iter().copied().find(|c| c.as_str() == s)
            }
        }

        pub const CLIENTS: [ClientDef; ClientId::COUNT] = [
            $( ClientDef {
                id: $id,
                root: $root,
                relative_path: $rel,
                pattern: $pat,
                headless: $hl,
                parse_local: $pl,
                submit_default: $sd,
            } ),+
        ];

        // Display metadata is generated from the same exhaustive registry but
        // kept out of public ClientDef so downstream struct literals remain
        // source-compatible.
        const CLIENT_DISPLAY_NAMES: [&str; ClientId::COUNT] = [ $( $display ),+ ];
        const CLIENT_LOGO_URLS: [Option<&str>; ClientId::COUNT] = [ $( $logo ),+ ];

        const _: () = {
            let mut i = 0;
            $(
                assert!($index == i, "ClientId indices must be sequential");
                i += 1;
                let _ = i;
            )+
        };
    };
}

define_clients!(
    OpenCode = 0 => {
        id: "opencode",
        display: "OpenCode",
        logo: Some("https://tokscale.ai/assets/logos/opencode.png"),root: PathRoot::XdgData,
        relative: "opencode/storage/message",
        pattern: "*.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Claude = 1 => {
        id: "claude",
        display: "Claude Code",
        logo: Some("https://tokscale.ai/assets/logos/claude.jpg"),root: PathRoot::EnvVar {
            var: "CLAUDE_CONFIG_DIR",
            fallback_relative: ".claude",
        },
        relative: "projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Codex = 2 => {
        id: "codex",
        display: "Codex CLI",
        logo: Some("https://tokscale.ai/assets/logos/openai.jpg"),root: PathRoot::EnvVar {
            var: "CODEX_HOME",
            fallback_relative: ".codex",
        },
        relative: "sessions",
        pattern: "*.jsonl",
        headless: true,
        parse_local: true,
        submit_default: true
    },
    Cursor = 3 => {
        id: "cursor",
        display: "Cursor IDE",
        logo: Some("https://tokscale.ai/assets/logos/cursor.jpg"),root: PathRoot::Home,
        relative: ".config/tokscale/cursor-cache",
        pattern: "usage*.csv",
        headless: false,
        parse_local: false,
        submit_default: true
    },
    Gemini = 4 => {
        id: "gemini",
        display: "Gemini CLI",
        logo: Some("https://tokscale.ai/assets/logos/gemini.png"),root: PathRoot::EnvVar {
            var: "GEMINI_CLI_HOME",
            fallback_relative: ".gemini",
        },
        relative: "tmp",
        pattern: "*.json|*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Amp = 5 => {
        id: "amp",
        display: "Amp",
        logo: Some("https://tokscale.ai/assets/logos/amp.png"),root: PathRoot::XdgData,
        relative: "amp/threads",
        pattern: "T-*.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Droid = 6 => {
        id: "droid",
        display: "Droid",
        logo: Some("https://tokscale.ai/assets/logos/droid.png"),root: PathRoot::Home,
        relative: ".factory/sessions",
        pattern: "*.settings.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    OpenClaw = 7 => {
        id: "openclaw",
        display: "OpenClaw",
        logo: Some("https://github.com/openclaw.png"),root: PathRoot::Home,
        relative: ".openclaw/agents",
        pattern: "*.jsonl*",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Pi = 8 => {
        id: "pi",
        display: "Pi",
        logo: Some("https://tokscale.ai/assets/logos/pi.png"),root: PathRoot::Home,
        relative: ".pi/agent/sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Kimi = 9 => {
        id: "kimi",
        display: "Kimi CLI",
        logo: Some("https://tokscale.ai/assets/logos/kimi.png"),root: PathRoot::Home,
        relative: ".kimi/sessions",
        pattern: "wire.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Qwen = 10 => {
        id: "qwen",
        display: "Qwen CLI",
        logo: Some("https://tokscale.ai/assets/logos/qwen.png"),root: PathRoot::Home,
        relative: ".qwen/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    RooCode = 11 => {
        id: "roocode",
        display: "Roo Code",
        logo: Some("https://tokscale.ai/assets/logos/roocode.png"),root: PathRoot::Home,
        relative: ".config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks",
        pattern: "ui_messages.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    KiloCode = 12 => {
        id: "kilocode",
        display: "Kilo Code",
        logo: Some("https://tokscale.ai/assets/logos/kilocode.png"),root: PathRoot::Home,
        relative: ".config/Code/User/globalStorage/kilocode.kilo-code/tasks",
        pattern: "ui_messages.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Mux = 13 => {
        id: "mux",
        display: "Mux",
        logo: Some("https://tokscale.ai/assets/logos/mux.png"),root: PathRoot::Home,
        relative: ".mux/sessions",
        pattern: "session-usage.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Kilo = 14 => {
        id: "kilo",
        display: "Kilo CLI",
        logo: Some("https://tokscale.ai/assets/logos/kilocode.png"),root: PathRoot::XdgData,
        relative: "kilo/kilo.db",
        pattern: "kilo.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Crush = 15 => {
        id: "crush",
        display: "Crush",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/6b483d0f2de3717266dec8faed13acd067f90ff3/.github/assets/client-crush.png"),root: PathRoot::XdgData,
        relative: "crush/projects.json",
        pattern: "projects.json",
        headless: false,
        parse_local: true,
        submit_default: false
    },
    Hermes = 16 => {
        id: "hermes",
        display: "Hermes Agent",
        logo: Some("https://tokscale.ai/assets/logos/hermes.png"),root: PathRoot::EnvVar {
            var: "HERMES_HOME",
            fallback_relative: ".hermes",
        },
        relative: "state.db",
        pattern: "state.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Copilot = 17 => {
        id: "copilot",
        display: "Copilot CLI",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-copilot.jpg"),root: PathRoot::Home,
        relative: ".copilot/otel",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Goose = 18 => {
        id: "goose",
        display: "Goose",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-goose.png"),root: PathRoot::XdgData,
        relative: "goose/sessions/sessions.db",
        pattern: "sessions.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Codebuff = 19 => {
        id: "codebuff",
        display: "Codebuff",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-codebuff.png"),root: PathRoot::EnvVar {
            var: "CODEBUFF_DATA_DIR",
            fallback_relative: ".config/manicode",
        },
        relative: "projects",
        pattern: "chat-messages.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Antigravity = 20 => {
        id: "antigravity",
        display: "Antigravity",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-antigravity.png"),root: PathRoot::Config,
        relative: "antigravity-cache/sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Zed = 21 => {
        id: "zed",
        display: "Zed Agent",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-zed.webp"),root: PathRoot::XdgData,
        relative: "zed/threads/threads.db",
        pattern: "threads.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Kiro = 22 => {
        id: "kiro",
        display: "Kiro",
        logo: None,root: PathRoot::Home,
        relative: ".kiro/sessions/cli",
        pattern: "*.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Trae = 23 => {
        id: "trae",
        display: "Trae",
        logo: None,root: PathRoot::Config,
        relative: "trae-cache/sessions",
        pattern: "*.json",
        headless: false,
        parse_local: true,
        submit_default: false
    },
    Warp = 24 => {
        id: "warp",
        display: "Warp",
        logo: None,root: PathRoot::Config,
        relative: "warp-cache",
        pattern: "usage*.json",
        headless: false,
        parse_local: true,
        submit_default: false
    },
    Cline = 25 => {
        id: "cline",
        display: "Cline",
        logo: None,root: PathRoot::Home,
        relative: ".config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks",
        pattern: "ui_messages.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Gjc = 26 => {
        id: "gjc",
        display: "Gajae-Code",
        logo: None,root: PathRoot::EnvVar {
            var: "GJC_CODING_AGENT_DIR",
            fallback_relative: ".gjc/agent",
        },
        relative: "sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Grok = 27 => {
        id: "grok",
        display: "Grok Build",
        logo: None,root: PathRoot::EnvVar {
            var: "GROK_HOME",
            fallback_relative: ".grok",
        },
        relative: "sessions",
        pattern: "updates.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Jcode = 28 => {
        id: "jcode",
        display: "Jcode",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-jcode.png"),root: PathRoot::EnvVar {
            var: "JCODE_HOME",
            fallback_relative: ".jcode",
        },
        relative: "sessions",
        pattern: "session_*.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    CommandCode = 29 => {
        id: "commandcode",
        display: "Command Code",
        logo: None,root: PathRoot::Home,
        relative: ".commandcode/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    MiMoCode = 30 => {
        id: "micode",
        display: "MiMo Code",
        logo: None,root: PathRoot::XdgData,
        relative: "mimocode",
        pattern: "*.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // Antigravity CLI stores each conversation as a SQLite `.db` under
    // `~/.gemini/antigravity-cli/conversations/`. Unlike the IDE-backed
    // `Antigravity` client (which pulls usage from a running language server
    // over RPC and caches JSONL under the config dir), the CLI usage sits on
    // disk and is read directly — no RPC, no `antigravity sync` needed. Honors
    // `GEMINI_CLI_HOME` so a relocated Gemini home is picked up.
    AntigravityCli = 31 => {
        id: "antigravity-cli",
        display: "Antigravity CLI",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-antigravity.png"),root: PathRoot::EnvVar {
            var: "GEMINI_CLI_HOME",
            fallback_relative: ".gemini",
        },
        relative: "antigravity-cli/conversations",
        pattern: "*.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Junie = 32 => {
        id: "junie",
        display: "Junie",
        logo: Some("https://github.com/JetBrains.png"),root: PathRoot::Home,
        relative: ".junie/sessions",
        pattern: "events.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Zcode = 33 => {
        id: "zcode",
        display: "ZCode",
        logo: None,root: PathRoot::Home,
        relative: ".zcode/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    OpenCodeReview = 34 => {
        id: "opencodereview",
        display: "OpenCodeReview",
        logo: None,root: PathRoot::Home,
        relative: ".opencodereview/sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    CodeBuddy = 35 => {
        id: "codebuddy",
        display: "CodeBuddy",
        logo: None,root: PathRoot::Home,
        relative: ".codebuddy/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    WorkBuddy = 36 => {
        id: "workbuddy",
        display: "WorkBuddy",
        logo: None,root: PathRoot::Home,
        relative: ".workbuddy",
        pattern: "workbuddy.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    DevinCli = 37 => {
        id: "devin-cli",
        display: "Devin CLI",
        logo: None,root: PathRoot::XdgData,
        relative: "devin/cli/sessions.db",
        pattern: "sessions.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    DevinDesktop = 38 => {
        id: "devin-desktop",
        display: "Devin Desktop",
        logo: None,root: PathRoot::Home,
        relative: "Library/Application Support/Devin/User/acp-events",
        pattern: "*.ndjson",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // Senpi (OmO Native) is a pi-mono descendant and writes the same session
    // JSONL under `<agent dir>/sessions/<encoded-cwd>/*.jsonl`. The agent dir
    // honors `SENPI_CODING_AGENT_DIR` and otherwise defaults to `~/.senpi/agent`,
    // mirroring the `gjc` layout.
    Senpi = 39 => {
        id: "senpi",
        display: "Senpi (OmO Native)",
        logo: None,root: PathRoot::EnvVar {
            var: "SENPI_CODING_AGENT_DIR",
            fallback_relative: ".senpi/agent",
        },
        relative: "sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // Augment Code / Auggie CLI stores per-session JSON snapshots under
    // `~/.augment/sessions/<sessionId>.json` with per-turn token_usage on
    // exchange.response_nodes.
    Augment = 40 => {
        id: "augment",
        display: "Augment Code",
        logo: Some("https://github.com/augmentcode.png"),root: PathRoot::Home,
        relative: ".augment/sessions",
        pattern: "*.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // Kimchi Coding uses the Pi session format under its own agent directory.
    // The launcher exposes KIMCHI_CODING_AGENT_DIR for relocated installs.
    Kimchi = 41 => {
        id: "kimchi",
        display: "Kimchi",
        logo: Some("https://github.com/getkimchi.png"),root: PathRoot::EnvVar {
            var: "KIMCHI_CODING_AGENT_DIR",
            fallback_relative: ".config/kimchi/harness",
        },
        relative: "sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // Reasonix stores authoritative provider usage as daily append-only JSONL
    // records under `<state root>/stats/`. Transcript JSONL is intentionally
    // excluded: it lacks exact token counters and overlaps these records.
    Reasonix = 42 => {
        id: "reasonix",
        display: "Reasonix",
        logo: None,root: PathRoot::ReasonixHome,
        relative: "stats",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // Prime Agent uses the Pi append-only JSONL session format. Root sessions
    // live in `<agent dir>/sessions`; RLM child sessions are discovered from
    // the sibling `session-artifacts` tree by the scanner.
    PrimeAgent = 43 => {
        id: "prime-agent",
        display: "Prime Agent",
        logo: Some("https://github.com/PrimeIntellect-ai.png"),root: PathRoot::EnvVar {
            var: "PRIME_AGENT_CODING_AGENT_DIR",
            fallback_relative: ".prime/agent",
        },
        relative: "sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // Freebuff is a compile-time build variant of the Codebuff CLI, so it
    // writes to the same `~/.config/manicode*` tree and the same
    // `projects/<project>/chats/<chatId>/chat-messages.json` layout. The two
    // products are told apart per chat by the persisted root agent id, not by
    // location (see `sessions::freebuff`).
    Freebuff = 44 => {
        id: "freebuff",
        display: "Freebuff",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-freebuff.png"),root: PathRoot::EnvVar {
            var: "FREEBUFF_DATA_DIR",
            fallback_relative: ".config/manicode",
        },
        relative: "projects",
        pattern: "chat-messages.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // Cherry Studio (Electron desktop client) writes standard Claude Code
    // transcripts under its per-user app-data directory. V2 uses
    // `%APPDATA%\CherryStudio\Data\Agents\.claude\projects` on Windows;
    // V1 uses the root below. The transcript format is identical to Claude
    // Code's, but parsing uses the dedicated `sessions::cherrystudio` parser,
    // which dedupes replayed records by stable request/message IDs.
    CherryStudio = 45 => {
        id: "cherrystudio",
        display: "Cherry Studio",
        logo: None,
        root: PathRoot::AppData,
        relative: "CherryStudio/.claude/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // DeepSeek Harness (DSH) writes one JSONL transcript per session under
    // `<DSH_HOME>/sessions/<encoded-cwd>/<session-id>/session.jsonl.zstd`
    // (`DSH_HOME` defaults to `~/.dsh`); a backend configured with
    // `compression: none` writes the same rows to `session.jsonl`, so the scan
    // pattern accepts both spellings. Each `assistant/message` event carries
    // authoritative per-call usage (`inputTokens`/`outputTokens`/`cacheReadTokens`)
    // plus the model/provider it was served by; the `session` event supplies the
    // workspace (`cwd`) and session id. See `sessions::dsh`.
    Dsh = 46 => {
        id: "dsh",
        display: "DeepSeek Harness",
        logo: None,
        root: PathRoot::EnvVar {
            var: "DSH_HOME",
            fallback_relative: ".dsh",
        },
        relative: "sessions",
        pattern: "dsh-session-log",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    // MiniMax Code exposes authoritative per-call usage through
    // `mcode exec --output-format stream-json`. Tokscale captures those
    // streams under its own headless root instead of scanning MiniMax Code's
    // shared Desktop/Runtime session store, where the originating surface is
    // not distinguishable.
    Mcode = 47 => {
        id: "mcode",
        display: "MiniMax Code",
        logo: Some("https://github.com/MiniMax-AI.png"),
        root: PathRoot::Config,
        relative: "headless/mcode",
        pattern: "*.jsonl",
        headless: true,
        parse_local: true,
        submit_default: true
    },
    // fx (vercel-labs/fx) is a Unix-like coding agent written in Zig. fx
    // aggregates per-request token usage into one snapshot per session under
    // `~/.fx/sessions/<sessionId>/usage-v2.json`; the sibling `session.json`
    // (`workspace_root`) and the shared `sessions/index.json` (session title)
    // give the parser real workspace/session attribution.
    Fx = 48 => {
        id: "fx",
        display: "Fx",
        logo: Some("https://raw.githubusercontent.com/junhoyeo/tokscale/main/.github/assets/client-fx.png"),
        root: PathRoot::Home,
        relative: ".fx/sessions",
        pattern: "usage-v2.json",
        headless: false,
        parse_local: true,
        submit_default: true
    }
);

pub struct ClientCounts {
    counts: [i32; ClientId::COUNT],
}

impl ClientCounts {
    pub fn new() -> Self {
        Self {
            counts: [0; ClientId::COUNT],
        }
    }

    pub fn get(&self, client: ClientId) -> i32 {
        self.counts[client as usize]
    }

    pub fn set(&mut self, client: ClientId, value: i32) {
        self.counts[client as usize] = value;
    }

    pub fn add(&mut self, client: ClientId, value: i32) {
        self.counts[client as usize] += value;
    }
}

impl Default for ClientCounts {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // These tests mutate process-global environment variables, so they take
    // `#[serial]` rather than the private `Mutex` they used to share. The
    // mutex made them exclusive with each other but not with the rest of the
    // crate, which serializes on `serial_test` — two disjoint domains over one
    // set of variables. `test_path_root_config_*` restores its snapshot of
    // `TOKSCALE_CONFIG_DIR` on the way out, which is a *clear* when the
    // developer has none set, and that clear landed in the middle of the
    // pricing cache tests that redirect the same variable: they went on
    // asserting against a temp path the code was no longer writing to while
    // the real `~/.config/tokscale/cache` took the write. One domain is the
    // only arrangement that holds, so nothing here may reintroduce a second.
    // The tests this module serializes restore through `EnvGuard` rather than a
    // trailing restore call: a failing assertion panics before such a call
    // runs, and `#[serial]` prevents overlap, not inheritance — so a redirect
    // would leak into every later test in the process. `paths::tests` documents
    // the same trap.
    use crate::paths::test_env::EnvGuard;

    /// Join `relative` onto `root` with native separators throughout — the
    /// behavior `ClientDef::resolve_path_with_env_strategy` must produce on
    /// every platform (#1048). `Path::join` alone is not enough: it only
    /// normalizes the junction, leaving the relative half's own `/`
    /// separators untouched on Windows. Pushing each component is the only
    /// spelling that yields `C:\Users\me\.codex\sessions` there.
    fn native_join(root: &std::path::Path, relative: &str) -> String {
        let mut path = root.to_path_buf();
        for component in std::path::Path::new(relative).components() {
            path.push(component.as_os_str());
        }
        path.to_string_lossy().into_owned()
    }

    // Retained for the env tests below that predate this module's move to one
    // serialization domain. New tests should capture an `EnvGuard` instead;
    // this pairing is only panic-safe when nothing between it and the restore
    // can fail.
    fn restore_env(var: &str, previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var(var, value) },
            None => unsafe { std::env::remove_var(var) },
        }
    }

    #[test]
    fn every_registered_client_has_human_readable_display_metadata() {
        for client in ClientId::iter() {
            let display_name = client.display_name();
            assert!(
                !display_name.trim().is_empty(),
                "{} has no display name",
                client.as_str()
            );
            assert_ne!(
                display_name,
                client.as_str(),
                "{} falls back to its raw lowercase id",
                client.as_str()
            );
        }
    }

    #[test]
    fn canonical_client_brand_labels_and_logos_are_registered() {
        assert_eq!(ClientId::Claude.display_name(), "Claude Code");
        assert_eq!(ClientId::Codex.display_name(), "Codex CLI");
        assert_eq!(ClientId::Cursor.display_name(), "Cursor IDE");
        assert_eq!(ClientId::KiloCode.display_name(), "Kilo Code");
        assert_eq!(ClientId::Kilo.display_name(), "Kilo CLI");
        assert_eq!(ClientId::Senpi.display_name(), "Senpi (OmO Native)");
        assert_eq!(
            ClientId::OpenCode.logo_url(),
            Some("https://tokscale.ai/assets/logos/opencode.png")
        );
    }

    #[test]
    fn test_client_id_count() {
        assert_eq!(ClientId::COUNT, 49);
    }

    #[test]
    fn test_mcode_registered_as_headless_local_source() {
        let client = ClientId::from_str("mcode").expect("mcode client should be registered");
        assert_eq!(client.data().relative_path, "headless/mcode");
        assert_eq!(client.data().pattern, "*.jsonl");
        assert!(client.data().headless);
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
    }

    #[test]
    fn test_senpi_client_registered_as_local_session_source() {
        let client = ClientId::from_str("senpi").expect("senpi client should be registered");
        assert_eq!(client.data().relative_path, "sessions");
        assert_eq!(client.data().pattern, "*.jsonl");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_prime_agent_client_registered_as_local_session_source() {
        let client =
            ClientId::from_str("prime-agent").expect("prime-agent client should be registered");
        assert_eq!(
            client
                .data()
                .resolve_path_with_env_strategy("/tmp/home", false),
            native_join(std::path::Path::new("/tmp/home"), ".prime/agent/sessions")
        );
        assert_eq!(client.data().pattern, "*.jsonl");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_resolve_path_joins_with_native_separators_not_hardcoded_slash() {
        // #1048: on Windows a `C:\Users\me` root joined with a `/`-separated
        // relative path used to produce `C:\Users\me/.codex/sessions` (mixed
        // separators) that reached user-facing `clients --json` output. The
        // joined result must use native separators throughout — component
        // pushes, not a hand-concatenated "{root}/{relative}" string and not
        // a single `Path::join` (which only normalizes the junction).
        let client = ClientDef {
            id: "codex",
            root: PathRoot::Home,
            relative_path: ".codex/sessions",
            pattern: "*.jsonl",
            headless: false,
            parse_local: true,
            submit_default: true,
        };
        let windows_style_home = r"C:\Users\me";
        let joined = client.resolve_path_with_env_strategy(windows_style_home, false);
        let expected = native_join(
            std::path::Path::new(windows_style_home),
            client.relative_path,
        );
        assert_eq!(joined, expected);
        // On Windows the resolved path must use native separators throughout:
        // no forward slash may remain from the relative half or the joiner.
        #[cfg(windows)]
        assert!(
            !joined.contains('/'),
            "mixed separators in resolved path: {joined:?}"
        );
    }

    #[test]
    fn test_explicit_home_app_data_root_uses_platform_layout() {
        let home = absolute_test_path("explicit-home");
        let expected = {
            #[cfg(target_os = "windows")]
            {
                home.join("AppData").join("Roaming")
            }
            #[cfg(target_os = "macos")]
            {
                home.join("Library").join("Application Support")
            }
            #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
            {
                home.join(".config")
            }
        };

        assert_eq!(
            PathRoot::AppData.resolve_with_env_strategy(home.to_str().unwrap(), false),
            expected.to_string_lossy()
        );
    }

    /// Under env roots the AppData root must still land under the home it was
    /// handed once that home is not the machine profile.
    ///
    /// `dirs::config_dir()` on Windows is the `FOLDERID_RoamingAppData` known
    /// folder, which ignores every environment variable, so this root was the
    /// one scan target that kept reading the live profile after
    /// `paths::home_dir()` had been redirected. Cherry Studio — the only
    /// AppData-rooted client — was therefore never discovered under a
    /// redirected home on Windows, while macOS and Linux resolved it correctly
    /// because their `dirs::config_dir()` is `$HOME`/`$XDG_CONFIG_HOME`-derived
    /// and so already follows the redirect.
    ///
    /// The assertions read the known-folder API but no environment variable, so
    /// this test does not need to serialize against the `EnvGuard` tests above.
    #[test]
    fn test_env_roots_app_data_follows_a_redirected_home() {
        let home = absolute_test_path("redirected-home");

        assert_eq!(
            app_data_follows_home(home.to_str().unwrap()),
            cfg!(target_os = "windows"),
            "only Windows has an app-data lookup that a redirected home cannot reach"
        );

        #[cfg(target_os = "windows")]
        assert_eq!(
            PathRoot::AppData.resolve_with_env_strategy(home.to_str().unwrap(), true),
            native_join(&home, "AppData/Roaming"),
            "a redirected Windows home must win over the roaming-app-data known folder"
        );
    }

    /// The known-folder answer stays authoritative when the home *is* the real
    /// profile: folder redirection and roaming profiles can legitimately place
    /// `%APPDATA%` outside the profile directory, and deriving it from the home
    /// would silently relocate those users' scans.
    #[test]
    fn test_env_roots_app_data_keeps_the_platform_lookup_for_the_real_profile() {
        let Some(profile) = dirs::home_dir() else {
            return;
        };

        assert!(
            !app_data_follows_home(&profile.to_string_lossy()),
            "the machine profile is not a redirect and must not override the platform lookup"
        );
    }

    /// A home that only *spells* the profile differently is not a redirect.
    ///
    /// Windows reaches the same directory through different casing, through the
    /// 8.3 alias (`C:\Users\RUNNER~1`), and through junctions, and `Path`
    /// compares all of those as distinct. Reading one as a redirect would pull
    /// the app-data root off the known folder, and on a machine whose
    /// `FOLDERID_RoamingAppData` is itself redirected that loses the user's
    /// transcripts. Both assertions are skipped rather than inverted if
    /// `canonicalize` cannot resolve the spelling, since a case-sensitive
    /// volume would legitimately make them different directories.
    #[cfg(target_os = "windows")]
    #[test]
    fn test_env_roots_app_data_sees_through_windows_spellings_of_the_profile() {
        let Some(profile) = dirs::home_dir() else {
            return;
        };
        let Ok(canonical_profile) = std::fs::canonicalize(&profile) else {
            return;
        };

        assert!(
            !app_data_follows_home(&canonical_profile.to_string_lossy()),
            "the verbatim spelling of the profile is the profile, not a redirect"
        );

        let shouted = profile.to_string_lossy().to_uppercase();
        if std::fs::canonicalize(&shouted).is_ok_and(|resolved| resolved == canonical_profile) {
            assert!(
                !app_data_follows_home(&shouted),
                "a case variant of the profile must not read as a redirect"
            );
        }
    }

    /// A POSIX-shaped `HOME` (Git Bash, MSYS2, Cygwin) is not a redirect.
    /// `paths::home_dir` rejects those because `Path` reads the leading `/` as
    /// "root of the current drive"; the AppData root must agree rather than
    /// relocating every Unix-shell user's scan to `C:\home\user\AppData`. The
    /// same holds for a drive-relative `C:temp`, which Windows resolves against
    /// the per-drive current directory.
    #[test]
    fn test_env_roots_app_data_ignores_non_absolute_windows_homes() {
        for home in ["/home/user", "C:temp", ""] {
            assert!(
                !app_data_follows_home(home),
                "{home:?} is not a usable native home and must not override the platform lookup"
            );
        }
    }

    /// The end-to-end claim the Windows CLI regression turns on: Cherry Studio
    /// is the only AppData-rooted client, and under env roots its transcript
    /// root must sit under a redirected home.
    #[cfg(target_os = "windows")]
    #[test]
    fn test_cherrystudio_transcript_root_follows_a_redirected_home_under_env_roots() {
        let home = absolute_test_path("cherry-home");

        assert_eq!(
            ClientId::CherryStudio
                .data()
                .resolve_path_with_env_strategy(home.to_str().unwrap(), true),
            native_join(&home, "AppData/Roaming/CherryStudio/.claude/projects")
        );
    }

    #[test]
    fn test_augment_client_registered_as_local_session_source() {
        let client = ClientId::from_str("augment").expect("augment client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".augment/sessions")
        );
        assert_eq!(client.data().relative_path, ".augment/sessions");
        assert_eq!(client.data().pattern, "*.json");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_kimchi_client_registered_as_local_session_source() {
        let client = ClientId::from_str("kimchi").expect("kimchi client should be registered");
        assert_eq!(client.data().relative_path, "sessions");
        assert_eq!(client.data().pattern, "*.jsonl");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    /// A home directory these tests can hand to the reasonix resolver.
    ///
    /// Reasonix is the one client whose root runs through `Path` — tilde
    /// expansion, an `is_absolute` check, and joins — rather than string
    /// concatenation. `Path` on Windows reads a POSIX-shaped `/tmp/home` as
    /// "the root of the current drive": not absolute, because it carries no
    /// drive prefix, so the resolver's relative-path arm fires and prepends the
    /// process's working directory. The other clients in this module keep
    /// `/tmp/home` because they never look at the value.
    fn reasonix_home() -> &'static str {
        if cfg!(windows) {
            "C:\\tmp\\home"
        } else {
            "/tmp/home"
        }
    }

    /// An absolute path on this platform, from `/`-separated components.
    fn absolute_test_path(relative: &str) -> std::path::PathBuf {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let mut path = std::path::PathBuf::from(root);
        for component in relative.split('/') {
            path.push(component);
        }
        path
    }

    /// `<root>/stats`, spelled the way [`ClientDef::resolve_path`] appends a
    /// client's relative path: native separators throughout, on every
    /// platform (#1048).
    fn reasonix_stats_under(root: impl AsRef<std::path::Path>) -> String {
        native_join(root.as_ref(), "stats")
    }

    /// The reasonix root with no environment override.
    ///
    /// Spelled out per platform rather than resolved, because the layout is the
    /// claim: `~/.reasonix` on Unix, and `%HOME%\AppData\Roaming\reasonix` on
    /// Windows, matching where the application actually keeps per-user config
    /// there.
    fn reasonix_default_root() -> std::path::PathBuf {
        let home = std::path::Path::new(reasonix_home());
        if cfg!(windows) {
            home.join("AppData").join("Roaming").join("reasonix")
        } else {
            home.join(".reasonix")
        }
    }

    #[test]
    fn test_reasonix_client_registered_as_local_session_source() {
        let client = ClientId::from_str("reasonix").expect("reasonix client should be registered");
        assert_eq!(
            client
                .data()
                .resolve_path_with_env_strategy(reasonix_home(), false),
            reasonix_stats_under(reasonix_default_root())
        );
        assert_eq!(client.data().pattern, "*.jsonl");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    #[serial]
    fn test_reasonix_stats_prefers_state_home_then_reasonix_home() {
        let mut env = EnvGuard::capture(&["REASONIX_STATE_HOME", "REASONIX_HOME"]);
        let custom_home = absolute_test_path("custom/reasonix-home");
        let custom_state = absolute_test_path("custom/reasonix-state");
        env.set("REASONIX_HOME", &custom_home);
        env.set("REASONIX_STATE_HOME", &custom_state);
        let client = ClientId::Reasonix;
        assert_eq!(
            client.data().resolve_path(reasonix_home()),
            reasonix_stats_under(&custom_state)
        );
        env.remove("REASONIX_STATE_HOME");
        assert_eq!(
            client.data().resolve_path(reasonix_home()),
            reasonix_stats_under(&custom_home)
        );
    }

    #[test]
    #[serial]
    fn test_reasonix_stats_normalizes_env_roots_and_ignores_blank_values() {
        let mut env = EnvGuard::capture(&["REASONIX_STATE_HOME", "REASONIX_HOME"]);
        let client = ClientId::Reasonix;

        env.set("REASONIX_STATE_HOME", "  ~/reasonix-state  ");
        env.set("REASONIX_HOME", absolute_test_path("unused/reasonix-home"));
        assert_eq!(
            client.data().resolve_path(reasonix_home()),
            reasonix_stats_under(native_join(
                std::path::Path::new(reasonix_home()),
                "reasonix-state"
            ))
        );

        env.set("REASONIX_STATE_HOME", " \t ");
        env.set("REASONIX_HOME", " relative-reasonix ");
        let expected = reasonix_stats_under(
            std::env::current_dir()
                .expect("test process has a current directory")
                .join("relative-reasonix"),
        );
        assert_eq!(client.data().resolve_path(reasonix_home()), expected);
    }

    #[test]
    #[serial]
    fn test_reasonix_stats_expands_environment_references_before_normalizing_paths() {
        let mut env = EnvGuard::capture(&[
            "REASONIX_STATE_HOME",
            "TOKSCALE_REASONIX_TEST_ROOT",
            "TOKSCALE_REASONIX_TEST_UNSET",
        ]);
        let client = ClientId::Reasonix;

        env.set("TOKSCALE_REASONIX_TEST_ROOT", "~/reasonix-state");
        env.remove("TOKSCALE_REASONIX_TEST_UNSET");
        env.set(
            "REASONIX_STATE_HOME",
            "${TOKSCALE_REASONIX_TEST_ROOT}/nested",
        );
        assert_eq!(
            client.data().resolve_path(reasonix_home()),
            reasonix_stats_under(native_join(
                std::path::Path::new(reasonix_home()),
                "reasonix-state/nested"
            ))
        );

        env.set(
            "REASONIX_STATE_HOME",
            "${TOKSCALE_REASONIX_TEST_UNSET:-relative-reasonix}",
        );
        let expected = reasonix_stats_under(
            std::env::current_dir()
                .expect("test process has a current directory")
                .join("relative-reasonix"),
        );
        // The home argument cannot reach this expectation — the default the
        // reference falls back to is relative, so the resolver prepends the
        // working directory and never consults the home. It is still
        // `reasonix_home()` like every other call in this module: a literal
        // `/tmp/home` here would read as a claim that this arm is special,
        // when the only thing special about it is that any home would do.
        assert_eq!(client.data().resolve_path(reasonix_home()), expected);
    }

    #[test]
    #[serial]
    fn test_reasonix_stats_ignores_env_roots_when_requested() {
        let mut env = EnvGuard::capture(&["REASONIX_STATE_HOME", "REASONIX_HOME"]);
        env.set(
            "REASONIX_STATE_HOME",
            absolute_test_path("custom/reasonix-state"),
        );
        env.set("REASONIX_HOME", absolute_test_path("custom/reasonix-home"));

        assert_eq!(
            ClientId::Reasonix
                .data()
                .resolve_path_with_env_strategy(reasonix_home(), false),
            reasonix_stats_under(reasonix_default_root())
        );
    }

    #[test]
    #[serial]
    fn test_kimchi_defaults_to_home_agent_dir_without_env_override() {
        let mut env = EnvGuard::capture(&["KIMCHI_CODING_AGENT_DIR"]);
        env.remove("KIMCHI_CODING_AGENT_DIR");

        let client = ClientId::from_str("kimchi").expect("kimchi client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(
                std::path::Path::new("/tmp/home"),
                ".config/kimchi/harness/sessions"
            )
        );
    }

    #[test]
    #[serial]
    fn test_kimchi_honors_agent_dir_env_override() {
        let mut env = EnvGuard::capture(&["KIMCHI_CODING_AGENT_DIR"]);
        env.set("KIMCHI_CODING_AGENT_DIR", "/custom/kimchi-agent");

        let client = ClientId::from_str("kimchi").expect("kimchi client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/custom/kimchi-agent"), "sessions")
        );
    }

    #[test]
    #[serial]
    fn test_dsh_defaults_to_home_sessions_without_env_override() {
        let mut env = EnvGuard::capture(&["DSH_HOME"]);
        env.remove("DSH_HOME");

        let client = ClientId::from_str("dsh").expect("dsh client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".dsh/sessions")
        );
        assert_eq!(client.data().pattern, "dsh-session-log");
    }

    #[test]
    #[serial]
    fn test_dsh_honors_dsh_home_env_override() {
        // DSH resolves its single root as configured path, then `$DSH_HOME`,
        // then `~/.dsh` (`util/home-paths/src/index.ts`, `resolveDshHome`), and
        // the shipped base pins the session store to `<home>/sessions`.
        let mut env = EnvGuard::capture(&["DSH_HOME"]);
        env.set("DSH_HOME", "/custom/dsh-home");

        let client = ClientId::from_str("dsh").expect("dsh client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/custom/dsh-home"), "sessions")
        );

        // Env roots disabled: fall back to the home-relative default.
        assert_eq!(
            client
                .data()
                .resolve_path_with_env_strategy("/tmp/home", false),
            native_join(std::path::Path::new("/tmp/home"), ".dsh/sessions")
        );
    }

    #[test]
    #[serial]
    fn test_senpi_defaults_to_home_agent_dir_without_env_override() {
        let mut env = EnvGuard::capture(&["SENPI_CODING_AGENT_DIR"]);
        env.remove("SENPI_CODING_AGENT_DIR");

        let client = ClientId::from_str("senpi").expect("senpi client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".senpi/agent/sessions")
        );
    }

    #[test]
    #[serial]
    fn test_senpi_honors_agent_dir_env_override() {
        let mut env = EnvGuard::capture(&["SENPI_CODING_AGENT_DIR"]);
        env.set("SENPI_CODING_AGENT_DIR", "/custom/senpi-agent");

        let client = ClientId::from_str("senpi").expect("senpi client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/custom/senpi-agent"), "sessions")
        );
    }

    #[test]
    fn test_codebuddy_client_registered_as_local_session_source() {
        let client =
            ClientId::from_str("codebuddy").expect("codebuddy client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".codebuddy/projects")
        );
        assert_eq!(client.data().pattern, "*.jsonl");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_workbuddy_client_registered_as_local_sqlite_source() {
        let client =
            ClientId::from_str("workbuddy").expect("workbuddy client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".workbuddy")
        );
        assert_eq!(client.data().pattern, "workbuddy.db");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_devincli_client_registered_as_local_session_source() {
        let client =
            ClientId::from_str("devin-cli").expect("devin-cli client should be registered");
        assert_eq!(client.data().relative_path, "devin/cli/sessions.db");
        assert_eq!(client.data().pattern, "sessions.db");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_devindesktop_client_registered_as_local_session_source() {
        let client =
            ClientId::from_str("devin-desktop").expect("devin-desktop client should be registered");
        assert_eq!(
            client.data().relative_path,
            "Library/Application Support/Devin/User/acp-events"
        );
        assert_eq!(client.data().pattern, "*.ndjson");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_commandcode_client_registered_as_local_session_source() {
        let client =
            ClientId::from_str("commandcode").expect("commandcode client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".commandcode/projects")
        );
        assert_eq!(client.data().pattern, "*.jsonl");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_junie_client_registered_as_local_session_source() {
        let client = ClientId::from_str("junie").expect("junie client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".junie/sessions")
        );
        assert_eq!(client.data().pattern, "events.jsonl");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
        assert!(!client.data().headless);
    }

    #[test]
    fn test_client_id_all_len_matches_count() {
        assert_eq!(ClientId::ALL.len(), ClientId::COUNT);
    }

    #[test]
    fn test_client_id_string_round_trip() {
        for client in ClientId::iter() {
            let id = client.as_str();
            assert_eq!(ClientId::from_str(id), Some(client));
        }
    }

    #[test]
    fn test_warp_client_registered_as_aggregate_cache_source() {
        let client = ClientId::from_str("warp").expect("warp client should be registered");
        assert_eq!(client.data().relative_path, "warp-cache");
        assert_eq!(client.data().pattern, "usage*.json");
        assert!(client.data().parse_local);
        assert!(!client.data().submit_default);
    }

    #[test]
    fn test_grok_client_registered_as_local_session_source() {
        let client = ClientId::from_str("grok").expect("grok client should be registered");
        assert_eq!(client.data().relative_path, "sessions");
        assert_eq!(client.data().pattern, "updates.jsonl");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
    }

    #[test]
    fn test_jcode_client_registered_as_local_session_source() {
        let client = ClientId::from_str("jcode").expect("jcode client should be registered");
        assert_eq!(client.data().relative_path, "sessions");
        assert_eq!(client.data().pattern, "session_*.json");
        assert!(client.data().parse_local);
        assert!(client.data().submit_default);
    }

    #[test]
    fn test_path_root_home_resolves_to_home_dir() {
        let home = "/tmp/home";
        assert_eq!(PathRoot::Home.resolve(home), home);
    }

    #[test]
    #[serial]
    fn test_path_root_xdg_data_uses_env_var_when_set() {
        let previous = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-data-home") };

        let resolved = PathRoot::XdgData.resolve("/tmp/home");
        assert_eq!(resolved, "/tmp/xdg-data-home");

        restore_env("XDG_DATA_HOME", previous);
    }

    #[test]
    #[serial]
    fn test_path_root_xdg_data_falls_back_when_unset() {
        let previous = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        let resolved = PathRoot::XdgData.resolve("/tmp/home");
        assert_eq!(
            resolved,
            native_join(std::path::Path::new("/tmp/home"), ".local/share")
        );

        restore_env("XDG_DATA_HOME", previous);
    }

    #[test]
    #[serial]
    fn test_path_root_xdg_data_ignores_env_when_disabled() {
        let previous = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-data-home") };

        let resolved = PathRoot::XdgData.resolve_with_env_strategy("/tmp/home", false);
        assert_eq!(
            resolved,
            native_join(std::path::Path::new("/tmp/home"), ".local/share")
        );

        restore_env("XDG_DATA_HOME", previous);
    }

    #[test]
    #[serial]
    fn test_path_root_config_uses_override_when_set() {
        let previous_override = std::env::var("TOKSCALE_CONFIG_DIR").ok();
        let previous_xdg = std::env::var("XDG_CONFIG_HOME").ok();
        unsafe {
            std::env::set_var("TOKSCALE_CONFIG_DIR", "/tmp/custom-config-root");
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-config-home");
        }

        let resolved = PathRoot::Config.resolve("/tmp/home");
        assert_eq!(resolved, "/tmp/custom-config-root");

        restore_env("TOKSCALE_CONFIG_DIR", previous_override);
        restore_env("XDG_CONFIG_HOME", previous_xdg);
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[serial]
    fn test_path_root_config_uses_xdg_config_home_when_override_unset() {
        let previous_override = std::env::var("TOKSCALE_CONFIG_DIR").ok();
        let previous_xdg = std::env::var("XDG_CONFIG_HOME").ok();
        unsafe {
            std::env::remove_var("TOKSCALE_CONFIG_DIR");
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-config-home");
        }

        let resolved = PathRoot::Config.resolve("/tmp/home");
        assert_eq!(resolved, "/tmp/xdg-config-home/tokscale");

        restore_env("TOKSCALE_CONFIG_DIR", previous_override);
        restore_env("XDG_CONFIG_HOME", previous_xdg);
    }

    #[test]
    #[cfg(target_os = "windows")]
    #[serial]
    fn test_path_root_config_uses_dirs_config_dir_on_windows() {
        // Windows must resolve PathRoot::Config to the same root that
        // paths::get_config_dir() and get_antigravity_cache_dir() use,
        // i.e. dirs::config_dir() (= %APPDATA%\tokscale). Hardcoding
        // {home}/.config/tokscale would diverge from the writer side
        // and silently hide synced Antigravity data from reports.
        let previous_override = std::env::var("TOKSCALE_CONFIG_DIR").ok();
        unsafe {
            std::env::remove_var("TOKSCALE_CONFIG_DIR");
        }

        let resolved = PathRoot::Config.resolve("C:\\fake-home");
        let expected = dirs::config_dir()
            .expect("Windows always exposes dirs::config_dir")
            .join("tokscale")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            resolved, expected,
            "PathRoot::Config on Windows must match dirs::config_dir().join('tokscale') so the scanner agrees with the writer"
        );

        restore_env("TOKSCALE_CONFIG_DIR", previous_override);
    }

    #[test]
    #[serial]
    fn test_path_root_config_ignores_env_when_disabled() {
        let previous_override = std::env::var("TOKSCALE_CONFIG_DIR").ok();
        let previous_xdg = std::env::var("XDG_CONFIG_HOME").ok();
        unsafe {
            std::env::set_var("TOKSCALE_CONFIG_DIR", "/tmp/custom-config-root");
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-config-home");
        }

        let resolved = PathRoot::Config.resolve_with_env_strategy("/tmp/home", false);
        let expected = if cfg!(target_os = "windows") {
            std::path::Path::new("/tmp/home")
                .join("AppData/Roaming/tokscale")
                .to_string_lossy()
                .into_owned()
        } else {
            native_join(std::path::Path::new("/tmp/home"), ".config/tokscale")
        };
        assert_eq!(resolved, expected);

        restore_env("TOKSCALE_CONFIG_DIR", previous_override);
        restore_env("XDG_CONFIG_HOME", previous_xdg);
    }

    #[test]
    #[serial]
    fn test_path_root_env_var_uses_env_when_set() {
        let var = "TOKSCALE_TEST_PATH_ROOT";
        let previous = std::env::var(var).ok();
        unsafe { std::env::set_var(var, "/tmp/custom-root") };

        let root = PathRoot::EnvVar {
            var,
            fallback_relative: ".fallback",
        };
        let resolved = root.resolve("/tmp/home");
        assert_eq!(resolved, "/tmp/custom-root");

        restore_env(var, previous);
    }

    #[test]
    #[serial]
    fn test_path_root_env_var_falls_back_when_unset() {
        let var = "TOKSCALE_TEST_PATH_ROOT";
        let previous = std::env::var(var).ok();
        unsafe { std::env::remove_var(var) };

        let root = PathRoot::EnvVar {
            var,
            fallback_relative: ".fallback",
        };
        let resolved = root.resolve("/tmp/home");
        assert_eq!(
            resolved,
            native_join(std::path::Path::new("/tmp/home"), ".fallback")
        );

        restore_env(var, previous);
    }

    #[test]
    #[serial]
    fn test_path_root_env_var_ignores_env_when_disabled() {
        let var = "TOKSCALE_TEST_PATH_ROOT";
        let previous = std::env::var(var).ok();
        unsafe { std::env::set_var(var, "/tmp/custom-root") };

        let root = PathRoot::EnvVar {
            var,
            fallback_relative: ".fallback",
        };
        let resolved = root.resolve_with_env_strategy("/tmp/home", false);
        assert_eq!(
            resolved,
            native_join(std::path::Path::new("/tmp/home"), ".fallback")
        );

        restore_env(var, previous);
    }

    #[test]
    fn test_client_def_resolve_path_combines_root_and_relative() {
        let client = ClientDef {
            id: "test",
            root: PathRoot::Home,
            relative_path: ".test/sessions",
            pattern: "*.jsonl",
            headless: false,
            parse_local: true,
            submit_default: true,
        };

        assert_eq!(
            client.resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".test/sessions")
        );
    }

    #[test]
    fn test_client_id_iter_yields_all_in_order() {
        let all: Vec<ClientId> = ClientId::iter().collect();
        assert_eq!(all, ClientId::ALL);
    }

    #[test]
    fn test_client_counts_get_set_add_work() {
        let mut counts = ClientCounts::new();

        assert_eq!(counts.get(ClientId::Claude), 0);
        counts.set(ClientId::Claude, 3);
        assert_eq!(counts.get(ClientId::Claude), 3);
        counts.add(ClientId::Claude, 2);
        assert_eq!(counts.get(ClientId::Claude), 5);
    }

    #[test]
    fn test_codex_root_uses_codex_home_env_var() {
        assert_eq!(
            ClientId::Codex.data().root,
            PathRoot::EnvVar {
                var: "CODEX_HOME",
                fallback_relative: ".codex",
            }
        );
    }

    #[test]
    fn test_claude_root_uses_claude_config_dir_env_var() {
        assert_eq!(
            ClientId::Claude.data().root,
            PathRoot::EnvVar {
                var: "CLAUDE_CONFIG_DIR",
                fallback_relative: ".claude",
            }
        );
    }

    #[test]
    #[serial]
    fn test_claude_defaults_to_home_dot_claude_without_env_override() {
        let mut env = EnvGuard::capture(&["CLAUDE_CONFIG_DIR"]);
        env.remove("CLAUDE_CONFIG_DIR");

        assert_eq!(
            ClientId::Claude.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".claude/projects")
        );
    }

    #[test]
    #[serial]
    fn test_claude_honors_claude_config_dir_env_override() {
        let mut env = EnvGuard::capture(&["CLAUDE_CONFIG_DIR"]);
        env.set("CLAUDE_CONFIG_DIR", "/custom/claude");

        assert_eq!(
            ClientId::Claude.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/custom/claude"), "projects")
        );
    }

    #[test]
    #[serial]
    fn test_gjc_data_dir_path() {
        let var = "GJC_CODING_AGENT_DIR";
        let previous = std::env::var(var).ok();
        // Env unset (cleared): resolves under home/.gjc/agent/sessions.
        unsafe { std::env::remove_var(var) };
        assert_eq!(
            ClientId::Gjc.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".gjc/agent/sessions")
        );
        assert_eq!(ClientId::Gjc.data().pattern, "*.jsonl");
        assert!(ClientId::Gjc.data().parse_local);
        assert!(ClientId::Gjc.data().submit_default);
        assert_eq!(ClientId::from_str("gjc"), Some(ClientId::Gjc));

        // Env set but env roots disabled: falls back to home, ignoring env.
        unsafe { std::env::set_var(var, "/tmp/custom-gjc") };
        assert_eq!(
            ClientId::Gjc
                .data()
                .resolve_path_with_env_strategy("/tmp/home", false),
            native_join(std::path::Path::new("/tmp/home"), ".gjc/agent/sessions")
        );

        restore_env(var, previous);
    }

    #[test]
    fn test_cursor_parse_local_is_false() {
        assert!(!ClientId::Cursor.data().parse_local);
    }

    #[test]
    fn test_crush_submit_default_is_false() {
        assert!(!ClientId::Crush.submit_default());
    }

    #[test]
    fn test_hermes_root_uses_hermes_home_env_var() {
        assert_eq!(
            ClientId::Hermes.data().root,
            PathRoot::EnvVar {
                var: "HERMES_HOME",
                fallback_relative: ".hermes",
            }
        );
        assert_eq!(ClientId::Hermes.data().relative_path, "state.db");
    }

    #[test]
    fn test_codebuff_root_uses_codebuff_data_dir_env_var() {
        assert_eq!(
            ClientId::Codebuff.data().root,
            PathRoot::EnvVar {
                var: "CODEBUFF_DATA_DIR",
                fallback_relative: ".config/manicode",
            }
        );
        assert_eq!(ClientId::Codebuff.data().pattern, "chat-messages.json");
    }

    #[test]
    fn test_freebuff_root_uses_freebuff_data_dir_env_var() {
        // Freebuff shares Codebuff's ~/.config/manicode layout (built on the
        // same runtime), keyed via its own FREEBUFF_DATA_DIR override.
        assert_eq!(
            ClientId::Freebuff.data().root,
            PathRoot::EnvVar {
                var: "FREEBUFF_DATA_DIR",
                fallback_relative: ".config/manicode",
            }
        );
        assert_eq!(ClientId::Freebuff.data().pattern, "chat-messages.json");
    }

    #[test]
    fn test_antigravity_parse_local_is_true() {
        assert!(ClientId::Antigravity.data().parse_local);
    }

    #[test]
    fn test_antigravity_submit_default_is_true() {
        assert!(ClientId::Antigravity.submit_default());
    }

    #[test]
    #[serial]
    fn test_zed_data_dir_path() {
        let previous = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        assert_eq!(
            ClientId::Zed.data().resolve_path("/tmp/home"),
            native_join(
                std::path::Path::new("/tmp/home"),
                ".local/share/zed/threads/threads.db"
            )
        );

        restore_env("XDG_DATA_HOME", previous);
    }

    #[test]
    fn test_zed_submit_default_is_true() {
        assert!(ClientId::Zed.submit_default());
    }

    #[test]
    fn test_kiro_data_dir_path() {
        assert_eq!(
            ClientId::Kiro.data().resolve_path("/tmp/home"),
            native_join(std::path::Path::new("/tmp/home"), ".kiro/sessions/cli")
        );
        assert_eq!(ClientId::Kiro.data().pattern, "*.json");
        assert!(ClientId::Kiro.parse_local());
        assert!(ClientId::Kiro.submit_default());
        assert!(!ClientId::Kiro.supports_headless());
    }

    /// Every `tokscale.ai/assets/logos/<file>` logo must be backed by a file
    /// that is actually committed under the frontend's public assets, because
    /// that directory is what gets deployed to the domain.
    ///
    /// This catches the failure this test was added for: the OpenClaw entry
    /// pointed at `openclaw.png` while the committed asset is `openclaw.jpg`,
    /// so the URL 404ed and `wrapped` silently rendered no OpenClaw logo. The
    /// fetch is deliberately fault-tolerant, and the unit test that covered the
    /// URL asserted the broken string verbatim, so nothing failed.
    #[test]
    fn test_hosted_logo_urls_have_a_committed_asset() {
        const HOSTED_PREFIX: &str = "https://tokscale.ai/assets/logos/";

        let logos_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/frontend/public/assets/logos");
        // Published crates and pruned checkouts do not carry the frontend.
        if !logos_dir.is_dir() {
            return;
        }

        let missing: Vec<String> = ClientId::iter()
            .filter_map(|client| Some((client, client.logo_url()?)))
            .filter_map(|(client, url)| Some((client, url.strip_prefix(HOSTED_PREFIX)?)))
            .filter(|(_, file)| !logos_dir.join(file).exists())
            .map(|(client, file)| format!("{} -> {}", client.as_str(), file))
            .collect();

        assert!(
            missing.is_empty(),
            "logo URLs with no committed asset in {}: {:?}",
            logos_dir.display(),
            missing
        );
    }
}
