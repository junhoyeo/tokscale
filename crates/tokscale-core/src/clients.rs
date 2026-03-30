#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRoot {
    Home,
    XdgData,
    EnvVar {
        var: &'static str,
        fallback_relative: &'static str,
    },
}

impl PathRoot {
    pub fn resolve(&self, home_dir: &str) -> String {
        match self {
            PathRoot::Home => home_dir.to_string(),
            PathRoot::XdgData => std::env::var("XDG_DATA_HOME")
                .unwrap_or_else(|_| format!("{}/.local/share", home_dir)),
            PathRoot::EnvVar {
                var,
                fallback_relative,
            } => {
                std::env::var(var).unwrap_or_else(|_| format!("{}/{}", home_dir, fallback_relative))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SourceDef {
    pub tag: &'static str,
    pub root: PathRoot,
    pub relative_path: &'static str,
    pub pattern: &'static str,
}

impl SourceDef {
    pub fn resolve_path(&self, home_dir: &str) -> String {
        format!("{}/{}", self.root.resolve(home_dir), self.relative_path)
    }
}

#[derive(Debug, Clone)]
pub struct ClientDef {
    pub id: &'static str,
    pub sources: &'static [SourceDef],
    pub headless: bool,
    pub parse_local: bool,
}

impl ClientDef {
    pub fn resolve_path(&self, home_dir: &str) -> String {
        self.sources
            .first()
            .map(|s| s.resolve_path(home_dir))
            .unwrap_or_default()
    }

    pub fn primary_source(&self) -> Option<&'static SourceDef> {
        self.sources.first()
    }

    pub fn source_by_tag(&self, tag: &str) -> Option<&'static SourceDef> {
        self.sources.iter().find(|s| s.tag == tag)
    }

    pub fn pattern(&self) -> &'static str {
        self.sources.first().map(|s| s.pattern).unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ClientId {
    OpenCode = 0,
    Claude = 1,
    Codex = 2,
    Cursor = 3,
    Gemini = 4,
    Amp = 5,
    Droid = 6,
    OpenClaw = 7,
    Pi = 8,
    Kimi = 9,
    Qwen = 10,
    RooCode = 11,
    Kilo = 12,
    Mux = 13,
}

impl ClientId {
    pub const COUNT: usize = 14;
    pub const ALL: [ClientId; Self::COUNT] = [
        ClientId::OpenCode,
        ClientId::Claude,
        ClientId::Codex,
        ClientId::Cursor,
        ClientId::Gemini,
        ClientId::Amp,
        ClientId::Droid,
        ClientId::OpenClaw,
        ClientId::Pi,
        ClientId::Kimi,
        ClientId::Qwen,
        ClientId::RooCode,
        ClientId::Kilo,
        ClientId::Mux,
    ];

    pub fn data(&self) -> &'static ClientDef {
        &CLIENTS[*self as usize]
    }

    pub fn as_str(&self) -> &'static str {
        self.data().id
    }

    pub fn file_pattern(&self) -> &'static str {
        self.data().pattern()
    }

    pub fn supports_headless(&self) -> bool {
        self.data().headless
    }

    pub fn parse_local(&self) -> bool {
        self.data().parse_local
    }

    pub fn iter() -> impl Iterator<Item = ClientId> {
        Self::ALL.iter().copied()
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<ClientId> {
        let normalized = if s == "kilocode" { "kilo" } else { s };
        Self::ALL.iter().copied().find(|c| c.as_str() == normalized)
    }
}

static SOURCES_OPENCODE: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::XdgData,
    relative_path: "opencode/storage/message",
    pattern: "*.json",
}];

static SOURCES_CLAUDE: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".claude/projects",
    pattern: "*.jsonl",
}];

static SOURCES_CODEX: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::EnvVar {
        var: "CODEX_HOME",
        fallback_relative: ".codex",
    },
    relative_path: "sessions",
    pattern: "*.jsonl",
}];

static SOURCES_CURSOR: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".config/tokscale/cursor-cache",
    pattern: "usage*.csv",
}];

static SOURCES_GEMINI: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".gemini/tmp",
    pattern: "*.json",
}];

static SOURCES_AMP: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::XdgData,
    relative_path: "amp/threads",
    pattern: "T-*.json",
}];

static SOURCES_DROID: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".factory/sessions",
    pattern: "*.settings.json",
}];

static SOURCES_OPENCLAW: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".openclaw/agents",
    pattern: "*.jsonl*",
}];

static SOURCES_PI: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".pi/agent/sessions",
    pattern: "*.jsonl",
}];

static SOURCES_KIMI: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".kimi/sessions",
    pattern: "wire.jsonl",
}];

static SOURCES_QWEN: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".qwen/projects",
    pattern: "*.jsonl",
}];

static SOURCES_ROOCODE: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks",
    pattern: "ui_messages.json",
}];

static SOURCES_KILO: [SourceDef; 2] = [
    SourceDef {
        tag: "vscode",
        root: PathRoot::Home,
        relative_path: ".config/Code/User/globalStorage/kilocode.kilo-code/tasks",
        pattern: "ui_messages.json",
    },
    SourceDef {
        tag: "cli",
        root: PathRoot::XdgData,
        relative_path: "kilo/kilo.db",
        pattern: "kilo.db",
    },
];

static SOURCES_MUX: [SourceDef; 1] = [SourceDef {
    tag: "default",
    root: PathRoot::Home,
    relative_path: ".mux/sessions",
    pattern: "session-usage.json",
}];

pub static CLIENTS: [ClientDef; ClientId::COUNT] = [
    ClientDef {
        id: "opencode",
        sources: &SOURCES_OPENCODE,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "claude",
        sources: &SOURCES_CLAUDE,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "codex",
        sources: &SOURCES_CODEX,
        headless: true,
        parse_local: true,
    },
    ClientDef {
        id: "cursor",
        sources: &SOURCES_CURSOR,
        headless: false,
        parse_local: false,
    },
    ClientDef {
        id: "gemini",
        sources: &SOURCES_GEMINI,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "amp",
        sources: &SOURCES_AMP,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "droid",
        sources: &SOURCES_DROID,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "openclaw",
        sources: &SOURCES_OPENCLAW,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "pi",
        sources: &SOURCES_PI,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "kimi",
        sources: &SOURCES_KIMI,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "qwen",
        sources: &SOURCES_QWEN,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "roocode",
        sources: &SOURCES_ROOCODE,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "kilo",
        sources: &SOURCES_KILO,
        headless: false,
        parse_local: true,
    },
    ClientDef {
        id: "mux",
        sources: &SOURCES_MUX,
        headless: false,
        parse_local: true,
    },
];

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
        assert_eq!(ClientId::COUNT, 14);
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
    fn test_source_def_resolve_path() {
        let source = SourceDef {
            tag: "default",
            root: PathRoot::Home,
            relative_path: ".test/sessions",
            pattern: "*.jsonl",
        };
        assert_eq!(source.resolve_path("/tmp/home"), "/tmp/home/.test/sessions");
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
        let def = ClientId::Codex.data();
        let source = def.primary_source().unwrap();
        assert_eq!(
            source.root,
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
    fn test_kilo_has_two_sources() {
        let def = ClientId::Kilo.data();
        assert_eq!(def.sources.len(), 2);
        assert_eq!(def.sources[0].tag, "vscode");
        assert_eq!(def.sources[1].tag, "cli");
    }

    #[test]
    fn test_kilo_source_by_tag() {
        let def = ClientId::Kilo.data();
        let vscode = def.source_by_tag("vscode").unwrap();
        assert!(vscode.relative_path.contains("kilocode.kilo-code"));

        let cli = def.source_by_tag("cli").unwrap();
        assert!(cli.relative_path.contains("kilo.db"));
    }

    #[test]
    fn test_kilocode_alias_maps_to_kilo() {
        assert_eq!(ClientId::from_str("kilocode"), Some(ClientId::Kilo));
        assert_eq!(ClientId::from_str("kilo"), Some(ClientId::Kilo));
    }
}
