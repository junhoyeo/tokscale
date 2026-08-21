use tokscale_core::ClientId;

pub struct ClientUi {
    pub hotkey: char,
}

pub const CLIENT_UI: [ClientUi; ClientId::COUNT] = [
    ClientUi { hotkey: '1' },
    ClientUi { hotkey: '2' },
    ClientUi { hotkey: '3' },
    ClientUi { hotkey: '4' },
    ClientUi { hotkey: '5' },
    ClientUi { hotkey: '6' },
    ClientUi { hotkey: '7' },
    ClientUi { hotkey: '8' },
    ClientUi { hotkey: '9' },
    ClientUi { hotkey: '0' },
    ClientUi { hotkey: 'w' },
    ClientUi { hotkey: 'r' },
    ClientUi { hotkey: 'k' },
    ClientUi { hotkey: 'x' },
    ClientUi { hotkey: 'l' },
    ClientUi { hotkey: 'h' },
    ClientUi { hotkey: 'e' },
    ClientUi { hotkey: 'c' },
    ClientUi { hotkey: 'o' },
    ClientUi { hotkey: 'b' },
    ClientUi { hotkey: 'a' },
    ClientUi { hotkey: 'z' },
    ClientUi { hotkey: 'i' },
    ClientUi { hotkey: 'y' },
    ClientUi { hotkey: 'v' },
    ClientUi { hotkey: 'n' },
    ClientUi { hotkey: 'g' },
    ClientUi { hotkey: 'u' },
    ClientUi { hotkey: 'j' },
    ClientUi { hotkey: 'd' },
    ClientUi { hotkey: 'm' },
    ClientUi { hotkey: 'f' },
    ClientUi { hotkey: 'p' },
    ClientUi { hotkey: 'q' },
    ClientUi { hotkey: 'O' },
    ClientUi { hotkey: 'C' },
    ClientUi { hotkey: 'B' },
    ClientUi { hotkey: 'D' },
    ClientUi { hotkey: 'E' },
    ClientUi { hotkey: 'S' },
    ClientUi { hotkey: 'A' },
    ClientUi { hotkey: 'K' },
    ClientUi { hotkey: 'R' },
    ClientUi { hotkey: 'P' },
    ClientUi { hotkey: 'F' },
    ClientUi { hotkey: 'G' },
    ClientUi { hotkey: 't' },
    ClientUi { hotkey: 'M' },
    // Fx: `s` is the global "sources" picker binding; `X` mirrors the fX
    // mnemonic (lowercase `x` belongs to Mux).
    ClientUi { hotkey: 'X' },
];

pub fn display_name(client: ClientId) -> &'static str {
    client.display_name()
}

/// Compact label for constrained TUI columns. Product-facing surfaces should
/// use [`display_name`] so the canonical registry label is preserved.
pub fn compact_display_name(client: ClientId) -> &'static str {
    match client {
        ClientId::Senpi => "Senpi",
        // "DeepSeek Harness" (16 cells) overflows the 15-cell Client column.
        ClientId::Dsh => "DeepSeek",
        _ => display_name(client),
    }
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
