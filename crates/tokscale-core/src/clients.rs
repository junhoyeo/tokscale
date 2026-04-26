#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRoot {
    Home,
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
                }

                // Match paths::get_config_dir() platform branches so the
                // scanner reads from the same root the writer (e.g.
                // get_antigravity_cache_dir) targets. Hardcoding
                // `{home}/.config/tokscale` everywhere would diverge from
                // dirs::config_dir() on Windows (where it resolves to
                // %APPDATA%\tokscale), causing synced data to land in
                // %APPDATA% while the scanner looks in %USERPROFILE%.
                #[cfg(target_os = "windows")]
                {
                    if let Some(dir) = dirs::config_dir() {
                        return dir.join("tokscale").to_string_lossy().into_owned();
                    }
                }

                format!("{home_dir}/.config/tokscale")
            }
            PathRoot::EnvVar {
                var,
                fallback_relative,
            } => {
                if use_env_roots {
                    std::env::var(var)
                        .unwrap_or_else(|_| format!("{}/{}", home_dir, fallback_relative))
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
        root: PathRoot::Home,
        relative: ".gemini/tmp",
        pattern: "*.json",
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
        submit_default: false
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
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn restore_env(var: &str, previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var(var, value) },
            None => unsafe { std::env::remove_var(var) },
        }
    }

    #[test]
    fn test_client_id_count() {
        assert_eq!(ClientId::COUNT, 21);
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
    fn test_path_root_home_resolves_to_home_dir() {
        let home = "/tmp/home";
        assert_eq!(PathRoot::Home.resolve(home), home);
    }

    #[test]
    fn test_path_root_xdg_data_uses_env_var_when_set() {
        let _guard = env_lock().lock().unwrap();
        let previous = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-data-home") };

        let resolved = PathRoot::XdgData.resolve("/tmp/home");
        assert_eq!(resolved, "/tmp/xdg-data-home");

        restore_env("XDG_DATA_HOME", previous);
    }

    #[test]
    fn test_path_root_xdg_data_falls_back_when_unset() {
        let _guard = env_lock().lock().unwrap();
        let previous = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        let resolved = PathRoot::XdgData.resolve("/tmp/home");
        assert_eq!(resolved, "/tmp/home/.local/share");

        restore_env("XDG_DATA_HOME", previous);
    }

    #[test]
    fn test_path_root_xdg_data_ignores_env_when_disabled() {
        let _guard = env_lock().lock().unwrap();
        let previous = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-data-home") };

        let resolved = PathRoot::XdgData.resolve_with_env_strategy("/tmp/home", false);
        assert_eq!(resolved, "/tmp/home/.local/share");

        restore_env("XDG_DATA_HOME", previous);
    }

    #[test]
    fn test_path_root_config_uses_override_when_set() {
        let _guard = env_lock().lock().unwrap();
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
    fn test_path_root_config_uses_xdg_config_home_when_override_unset() {
        let _guard = env_lock().lock().unwrap();
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
    fn test_path_root_config_uses_dirs_config_dir_on_windows() {
        // Windows must resolve PathRoot::Config to the same root that
        // paths::get_config_dir() and get_antigravity_cache_dir() use,
        // i.e. dirs::config_dir() (= %APPDATA%\tokscale). Hardcoding
        // {home}/.config/tokscale would diverge from the writer side
        // and silently hide synced Antigravity data from reports.
        let _guard = env_lock().lock().unwrap();
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
    fn test_path_root_config_ignores_env_when_disabled() {
        let _guard = env_lock().lock().unwrap();
        let previous_override = std::env::var("TOKSCALE_CONFIG_DIR").ok();
        let previous_xdg = std::env::var("XDG_CONFIG_HOME").ok();
        unsafe {
            std::env::set_var("TOKSCALE_CONFIG_DIR", "/tmp/custom-config-root");
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-config-home");
        }

        let resolved = PathRoot::Config.resolve_with_env_strategy("/tmp/home", false);
        assert_eq!(resolved, "/tmp/home/.config/tokscale");

        restore_env("TOKSCALE_CONFIG_DIR", previous_override);
        restore_env("XDG_CONFIG_HOME", previous_xdg);
    }

    #[test]
    fn test_path_root_env_var_uses_env_when_set() {
        let _guard = env_lock().lock().unwrap();
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
    fn test_path_root_env_var_falls_back_when_unset() {
        let _guard = env_lock().lock().unwrap();
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
    fn test_path_root_env_var_ignores_env_when_disabled() {
        let _guard = env_lock().lock().unwrap();
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
    fn test_antigravity_submit_default_is_false() {
        assert!(!ClientId::Antigravity.submit_default());
    }
}
