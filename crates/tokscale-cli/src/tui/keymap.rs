//! Layout-independent hotkey normalization.
//!
//! A terminal never tells an app which *physical* key was pressed — it only
//! delivers the *character* that the active OS keyboard layout produced. So
//! pressing the physical `Q` key under a Russian layout arrives as
//! `KeyCode::Char('й')`, and a `match` on `KeyCode::Char('q')` silently never
//! fires. The result: every single-letter hotkey breaks the moment the user
//! switches to a non-Latin layout (Russian, Ukrainian, Kazakh, Tatar, Greek,
//! …).
//!
//! [`normalize_hotkey`] fixes this by mapping a produced character back to the
//! Latin letter that sits on the **same physical key** of a US-QWERTY layout.
//! `'й' -> 'q'`, `'с' -> 'c'`, `'χ' -> 'x'`, and so on. ASCII passes straight
//! through, so the English path is untouched.
//!
//! ## Where this is (and isn't) applied
//!
//! Only on the *command* path — global hotkeys and yes/no confirmations. Text
//! input (e.g. the client-picker filter) keeps the raw character, otherwise a
//! Russian user could never type Cyrillic into a search box. Navigation keys
//! (Tab/Enter/Esc/arrows/F-keys) are already layout-independent and pass
//! through unchanged.
//!
//! ## Adding a language
//!
//! Each script is a `&[(char, char)]` table of `(produced_char, us_qwerty_char)`
//! pairs, listed in lowercase; uppercase and case-preservation are handled
//! automatically. To support a new alphabetic layout, add a table and append
//! it to [`LAYOUTS`]. Different scripts live in disjoint Unicode blocks, so
//! tables never collide.
//!
//! ## Known limits
//!
//! - CJK input via an IME (Chinese pinyin, Japanese romaji) is composed by the
//!   OS and the terminal delivers nothing per-keystroke until composition ends
//!   — it cannot be intercepted here. Such users press hotkeys in the IME's
//!   direct/half-width mode, which already yields Latin characters.
//! - A few positions have no letter in some scripts (e.g. the `Q` key carries
//!   no Greek letter), so those specific hotkeys stay layout-bound; `Esc` and
//!   `Ctrl+C` always work regardless.

use crossterm::event::KeyCode;

/// Cyrillic ЙЦУКЕН base layout. Shared by Russian, Ukrainian, Belarusian,
/// Kazakh, Tatar, Bashkir, Kyrgyz and other languages that extend it — their
/// extra glyphs sit on the number row, not on the letter keys, so the letter
/// positions below cover them all. The lone non-Russian entry is Ukrainian
/// `і`, which replaces `ы` on the `S` key.
///
/// Bulgarian is intentionally excluded: its BDS and phonetic layouts place the
/// *same* Cyrillic letters on *different* keys, and a character-based map can
/// encode only one position per glyph (e.g. `я` is `Z` here but `S` on BDS).
/// Supporting it would require knowing the active layout, which the terminal
/// never reports — so it cannot share this table without misfiring hotkeys.
const CYRILLIC: &[(char, char)] = &[
    ('й', 'q'),
    ('ц', 'w'),
    ('у', 'e'),
    ('к', 'r'),
    ('е', 't'),
    ('н', 'y'),
    ('г', 'u'),
    ('ш', 'i'),
    ('щ', 'o'),
    ('з', 'p'),
    ('ф', 'a'),
    ('ы', 's'),
    ('і', 's'),
    ('в', 'd'),
    ('а', 'f'),
    ('п', 'g'),
    ('р', 'h'),
    ('о', 'j'),
    ('л', 'k'),
    ('д', 'l'),
    ('я', 'z'),
    ('ч', 'x'),
    ('с', 'c'),
    ('м', 'v'),
    ('и', 'b'),
    ('т', 'n'),
    ('ь', 'm'),
];

/// Standard Greek layout. Positional, mostly phonetic. The `Q` key carries no
/// Greek letter (it produces `;`), so there is intentionally no `-> 'q'` entry.
const GREEK: &[(char, char)] = &[
    ('ς', 'w'),
    ('ε', 'e'),
    ('ρ', 'r'),
    ('τ', 't'),
    ('υ', 'y'),
    ('θ', 'u'),
    ('ι', 'i'),
    ('ο', 'o'),
    ('π', 'p'),
    ('α', 'a'),
    ('σ', 's'),
    ('δ', 'd'),
    ('φ', 'f'),
    ('γ', 'g'),
    ('η', 'h'),
    ('ξ', 'j'),
    ('κ', 'k'),
    ('λ', 'l'),
    ('ζ', 'z'),
    ('χ', 'x'),
    ('ψ', 'c'),
    ('ω', 'v'),
    ('β', 'b'),
    ('ν', 'n'),
    ('μ', 'm'),
];

/// All known layouts, scanned in order. Append a new `&[(char, char)]` table
/// here to support another script.
const LAYOUTS: &[&[(char, char)]] = &[CYRILLIC, GREEK];

/// Map a produced character to the Latin letter on the same physical US-QWERTY
/// key, preserving case. ASCII and unknown characters are returned unchanged.
fn latin_at_position(c: char) -> char {
    // Fast path: the canonical hotkey alphabet is ASCII and needs no mapping.
    if c.is_ascii() {
        return c;
    }

    let lower = c.to_lowercase().next().unwrap_or(c);
    for table in LAYOUTS {
        if let Some(&(_, latin)) = table.iter().find(|(src, _)| *src == lower) {
            return if c.is_uppercase() {
                latin.to_ascii_uppercase()
            } else {
                latin
            };
        }
    }
    c
}

/// Normalize a key for *command* matching: a character key is remapped to its
/// US-QWERTY positional equivalent; every other key code is left as-is.
///
/// Do not call this on text-input paths — there the literal character is what
/// the user means to type.
pub fn normalize_hotkey(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char(c) => KeyCode::Char(latin_at_position(c)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_passes_through_unchanged() {
        for c in ['q', 'c', 't', 'R', '+', '-', '=', ' ', '1'] {
            assert_eq!(normalize_hotkey(KeyCode::Char(c)), KeyCode::Char(c));
        }
    }

    #[test]
    fn russian_letters_map_to_qwerty_positions() {
        // The hotkeys this app actually uses, typed on a Russian layout.
        let cases = [
            ('й', 'q'),
            ('с', 'c'),
            ('е', 't'),
            ('в', 'd'),
            ('о', 'j'),
            ('з', 'p'),
            ('к', 'r'),
            ('н', 'y'),
            ('у', 'e'),
            ('ы', 's'),
            ('р', 'h'),
            ('м', 'v'),
            ('п', 'g'),
            ('ф', 'a'),
            ('ь', 'm'),
            ('ч', 'x'),
        ];
        for (cyrillic, latin) in cases {
            assert_eq!(
                normalize_hotkey(KeyCode::Char(cyrillic)),
                KeyCode::Char(latin),
                "{cyrillic} should map to {latin}",
            );
        }
    }

    #[test]
    fn ukrainian_i_maps_to_s_position() {
        assert_eq!(normalize_hotkey(KeyCode::Char('і')), KeyCode::Char('s'));
    }

    #[test]
    fn greek_letters_map_to_qwerty_positions() {
        let cases = [
            ('ς', 'w'),
            ('ε', 'e'),
            ('τ', 't'),
            ('χ', 'x'),
            ('ψ', 'c'),
            ('δ', 'd'),
            ('α', 'a'),
            ('μ', 'm'),
        ];
        for (greek, latin) in cases {
            assert_eq!(
                normalize_hotkey(KeyCode::Char(greek)),
                KeyCode::Char(latin),
                "{greek} should map to {latin}",
            );
        }
    }

    #[test]
    fn case_is_preserved() {
        // Shift+R toggles auto-refresh; on a Russian layout that is Shift+К.
        assert_eq!(normalize_hotkey(KeyCode::Char('К')), KeyCode::Char('R'));
        assert_eq!(normalize_hotkey(KeyCode::Char('Й')), KeyCode::Char('Q'));
        // Greek uppercase rho -> R.
        assert_eq!(normalize_hotkey(KeyCode::Char('Ρ')), KeyCode::Char('R'));
    }

    #[test]
    fn unknown_and_non_char_keys_are_untouched() {
        // CJK glyph: no positional mapping exists, leave it alone.
        assert_eq!(normalize_hotkey(KeyCode::Char('日')), KeyCode::Char('日'));
        assert_eq!(normalize_hotkey(KeyCode::Tab), KeyCode::Tab);
        assert_eq!(normalize_hotkey(KeyCode::Esc), KeyCode::Esc);
        assert_eq!(normalize_hotkey(KeyCode::Enter), KeyCode::Enter);
    }
}
