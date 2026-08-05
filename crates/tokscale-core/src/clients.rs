#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRoot {
    Home,
    ReasonixHome,
    XdgData,
    Config,
    EnvVar {
        var: &'static str,
        fallback_relative: &'static str,
    },
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
                    return std::path::Path::new(home_dir)
                        .join("AppData")
                        .join("Roaming")
                        .join("reasonix")
                        .to_string_lossy()
                        .into_owned();
                }
                #[cfg(not(target_os = "windows"))]
                {
                    format!("{home_dir}/.reasonix")
                }
            }
            PathRoot::XdgData => {
                if use_env_roots {
                    std::env::var("XDG_DATA_HOME")
                        .unwrap_or_else(|_| format!("{}/.local/share", home_dir))
                } else {
                    format!("{}/.local/share", home_dir)
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

                format!("{home_dir}/.config/tokscale")
            }
            PathRoot::EnvVar {
                var,
                fallback_relative,
            } => {
                if use_env_roots {
                    let val = std::env::var(var).unwrap_or_default();
                    if val.trim().is_empty() {
                        format!("{}/{}", home_dir, fallback_relative)
                    } else {
                        val
                    }
                } else {
                    format!("{}/{}", home_dir, fallback_relative)
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
        std::path::Path::new(home_dir).join(relative)
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
        format!(
            "{}/{}",
            self.root.resolve_with_env_strategy(home_dir, use_env_roots),
            self.relative_path
        )
    }

    pub fn resolve_path(&self, home_dir: &str) -> String {
        self.resolve_path_with_env_strategy(home_dir, true)
    }
}

macro_rules! define_clients {
    ( $( $variant:ident = $index:expr => { id: $id:expr, root: $root:expr, relative: $rel:expr, pattern: $pat:expr, headless: $hl:expr, parse_local: $pl:expr, submit_default: $sd:expr } ),+ $(,)? ) => {
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
        root: PathRoot::XdgData,
        relative: "opencode/storage/message",
        pattern: "*.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Claude = 1 => {
        id: "claude",
        root: PathRoot::Home,
        relative: ".claude/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Codex = 2 => {
        id: "codex",
        root: PathRoot::EnvVar {
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
        root: PathRoot::Home,
        relative: ".config/tokscale/cursor-cache",
        pattern: "usage*.csv",
        headless: false,
        parse_local: false,
        submit_default: true
    },
    Gemini = 4 => {
        id: "gemini",
        root: PathRoot::EnvVar {
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
        root: PathRoot::XdgData,
        relative: "amp/threads",
        pattern: "T-*.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Droid = 6 => {
        id: "droid",
        root: PathRoot::Home,
        relative: ".factory/sessions",
        pattern: "*.settings.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    OpenClaw = 7 => {
        id: "openclaw",
        root: PathRoot::Home,
        relative: ".openclaw/agents",
        pattern: "*.jsonl*",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Pi = 8 => {
        id: "pi",
        root: PathRoot::Home,
        relative: ".pi/agent/sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Kimi = 9 => {
        id: "kimi",
        root: PathRoot::Home,
        relative: ".kimi/sessions",
        pattern: "wire.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Qwen = 10 => {
        id: "qwen",
        root: PathRoot::Home,
        relative: ".qwen/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    RooCode = 11 => {
        id: "roocode",
        root: PathRoot::Home,
        relative: ".config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks",
        pattern: "ui_messages.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    KiloCode = 12 => {
        id: "kilocode",
        root: PathRoot::Home,
        relative: ".config/Code/User/globalStorage/kilocode.kilo-code/tasks",
        pattern: "ui_messages.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Mux = 13 => {
        id: "mux",
        root: PathRoot::Home,
        relative: ".mux/sessions",
        pattern: "session-usage.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Kilo = 14 => {
        id: "kilo",
        root: PathRoot::XdgData,
        relative: "kilo/kilo.db",
        pattern: "kilo.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Crush = 15 => {
        id: "crush",
        root: PathRoot::XdgData,
        relative: "crush/projects.json",
        pattern: "projects.json",
        headless: false,
        parse_local: true,
        submit_default: false
    },
    Hermes = 16 => {
        id: "hermes",
        root: PathRoot::EnvVar {
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
        root: PathRoot::Home,
        relative: ".copilot/otel",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Goose = 18 => {
        id: "goose",
        root: PathRoot::XdgData,
        relative: "goose/sessions/sessions.db",
        pattern: "sessions.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Codebuff = 19 => {
        id: "codebuff",
        root: PathRoot::EnvVar {
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
        root: PathRoot::Config,
        relative: "antigravity-cache/sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Zed = 21 => {
        id: "zed",
        root: PathRoot::XdgData,
        relative: "zed/threads/threads.db",
        pattern: "threads.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Kiro = 22 => {
        id: "kiro",
        root: PathRoot::Home,
        relative: ".kiro/sessions/cli",
        pattern: "*.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Trae = 23 => {
        id: "trae",
        root: PathRoot::Config,
        relative: "trae-cache/sessions",
        pattern: "*.json",
        headless: false,
        parse_local: true,
        submit_default: false
    },
    Warp = 24 => {
        id: "warp",
        root: PathRoot::Config,
        relative: "warp-cache",
        pattern: "usage*.json",
        headless: false,
        parse_local: true,
        submit_default: false
    },
    Cline = 25 => {
        id: "cline",
        root: PathRoot::Home,
        relative: ".config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks",
        pattern: "ui_messages.json",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Gjc = 26 => {
        id: "gjc",
        root: PathRoot::EnvVar {
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
        root: PathRoot::EnvVar {
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
        root: PathRoot::EnvVar {
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
        root: PathRoot::Home,
        relative: ".commandcode/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    MiMoCode = 30 => {
        id: "micode",
        root: PathRoot::XdgData,
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
        root: PathRoot::EnvVar {
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
        root: PathRoot::Home,
        relative: ".junie/sessions",
        pattern: "events.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    Zcode = 33 => {
        id: "zcode",
        root: PathRoot::Home,
        relative: ".zcode/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    OpenCodeReview = 34 => {
        id: "opencodereview",
        root: PathRoot::Home,
        relative: ".opencodereview/sessions",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    CodeBuddy = 35 => {
        id: "codebuddy",
        root: PathRoot::Home,
        relative: ".codebuddy/projects",
        pattern: "*.jsonl",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    WorkBuddy = 36 => {
        id: "workbuddy",
        root: PathRoot::Home,
        relative: ".workbuddy",
        pattern: "workbuddy.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    DevinCli = 37 => {
        id: "devin-cli",
        root: PathRoot::XdgData,
        relative: "devin/cli/sessions.db",
        pattern: "sessions.db",
        headless: false,
        parse_local: true,
        submit_default: true
    },
    DevinDesktop = 38 => {
        id: "devin-desktop",
        root: PathRoot::Home,
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
        root: PathRoot::EnvVar {
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
        root: PathRoot::Home,
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
        root: PathRoot::EnvVar {
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
        root: PathRoot::ReasonixHome,
        relative: "stats",
        pattern: "*.jsonl",
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
    fn test_client_id_count() {
        assert_eq!(ClientId::COUNT, 43);
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
    fn test_augment_client_registered_as_local_session_source() {
        let client = ClientId::from_str("augment").expect("augment client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            "/tmp/home/.augment/sessions"
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
    /// client's relative path: a forward slash, whatever the root's own
    /// separators are.
    fn reasonix_stats_under(root: impl AsRef<std::path::Path>) -> String {
        format!("{}/stats", root.as_ref().to_string_lossy())
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
            reasonix_stats_under(std::path::Path::new(reasonix_home()).join("reasonix-state"))
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
            reasonix_stats_under(
                std::path::Path::new(reasonix_home()).join("reasonix-state/nested")
            )
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
            "/tmp/home/.config/kimchi/harness/sessions"
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
            "/custom/kimchi-agent/sessions"
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
            "/tmp/home/.senpi/agent/sessions"
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
            "/custom/senpi-agent/sessions"
        );
    }

    #[test]
    fn test_codebuddy_client_registered_as_local_session_source() {
        let client =
            ClientId::from_str("codebuddy").expect("codebuddy client should be registered");
        assert_eq!(
            client.data().resolve_path("/tmp/home"),
            "/tmp/home/.codebuddy/projects"
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
            "/tmp/home/.workbuddy"
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
            "/tmp/home/.commandcode/projects"
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
            "/tmp/home/.junie/sessions"
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
        assert_eq!(resolved, "/tmp/home/.local/share");

        restore_env("XDG_DATA_HOME", previous);
    }

    #[test]
    #[serial]
    fn test_path_root_xdg_data_ignores_env_when_disabled() {
        let previous = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-data-home") };

        let resolved = PathRoot::XdgData.resolve_with_env_strategy("/tmp/home", false);
        assert_eq!(resolved, "/tmp/home/.local/share");

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
            "/tmp/home/.config/tokscale".to_string()
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
        assert_eq!(resolved, "/tmp/home/.fallback");

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
        assert_eq!(resolved, "/tmp/home/.fallback");

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

        assert_eq!(client.resolve_path("/tmp/home"), "/tmp/home/.test/sessions");
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
    #[serial]
    fn test_gjc_data_dir_path() {
        let var = "GJC_CODING_AGENT_DIR";
        let previous = std::env::var(var).ok();
        // Env unset (cleared): resolves under home/.gjc/agent/sessions.
        unsafe { std::env::remove_var(var) };
        assert_eq!(
            ClientId::Gjc.data().resolve_path("/tmp/home"),
            "/tmp/home/.gjc/agent/sessions"
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
            "/tmp/home/.gjc/agent/sessions"
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
            "/tmp/home/.local/share/zed/threads/threads.db"
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
            "/tmp/home/.kiro/sessions/cli"
        );
        assert_eq!(ClientId::Kiro.data().pattern, "*.json");
        assert!(ClientId::Kiro.parse_local());
        assert!(ClientId::Kiro.submit_default());
        assert!(!ClientId::Kiro.supports_headless());
    }
}
