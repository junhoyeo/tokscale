use tokscale_core::ClientId;

pub struct ClientUi {
    pub display_name: &'static str,
    pub hotkey: char,
}

pub const CLIENT_UI: [ClientUi; ClientId::COUNT] = [
    ClientUi {
        display_name: "OpenCode",
        hotkey: '1',
    },
    ClientUi {
        display_name: "Claude",
        hotkey: '2',
    },
    ClientUi {
        display_name: "Codex",
        hotkey: '3',
    },
    ClientUi {
        display_name: "Cursor",
        hotkey: '4',
    },
    ClientUi {
        display_name: "Gemini",
        hotkey: '5',
    },
    ClientUi {
        display_name: "Amp",
        hotkey: '6',
    },
    ClientUi {
        display_name: "Droid",
        hotkey: '7',
    },
    ClientUi {
        display_name: "OpenClaw",
        hotkey: '8',
    },
    ClientUi {
        display_name: "Pi",
        hotkey: '9',
    },
    ClientUi {
        display_name: "Kimi",
        hotkey: '0',
    },
    ClientUi {
        display_name: "Qwen",
        hotkey: 'w',
    },
    ClientUi {
        display_name: "Roo Code",
        hotkey: 'r',
    },
    ClientUi {
        display_name: "KiloCode",
        hotkey: 'k',
    },
    ClientUi {
        display_name: "Mux",
        hotkey: 'x',
    },
    ClientUi {
        display_name: "Kilo CLI",
        hotkey: 'l',
    },
    ClientUi {
        display_name: "Crush",
        hotkey: 'h',
    },
    ClientUi {
        display_name: "Hermes Agent",
        hotkey: 'e',
    },
];

pub fn display_name(client: ClientId) -> &'static str {
    CLIENT_UI[client as usize].display_name
}

pub fn hotkey(client: ClientId) -> char {
    CLIENT_UI[client as usize].hotkey
}

pub fn from_hotkey(key: char) -> Option<ClientId> {
    CLIENT_UI.iter().enumerate().find_map(|(i, ui)| {
        if ui.hotkey == key {
            ClientId::ALL.get(i).copied()
        } else {
            None
        }
    })
}
