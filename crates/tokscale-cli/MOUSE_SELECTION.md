# Mouse Selection Guide

## How to Select and Copy Text in the TUI

Tokscale's TUI supports **both mouse interactions** (clicking tabs, buttons) **and text selection** simultaneously.

### Text Selection Method

To select and copy text from the TUI, use your terminal's **native text selection** with a modifier key:

| Terminal | Method | Notes |
|----------|--------|-------|
| **Ghostty** | **Shift + Drag** | ⚠️ Known quirk: Second Shift+click extends selection. Add `mouse-shift-capture = never` to config for better behavior |
| **iTerm2** | **Shift + Drag** or **Option/Alt + Drag** | Both work |
| **WezTerm** | **Shift + Drag** | |
| **Alacritty** | **Shift + Drag** | |
| **Kitty** | **Shift + Drag** | |
| **Windows Terminal** | **Shift + Drag** | |
| **GNOME Terminal** | **Shift + Drag** | |
| **Konsole** | **Shift + Drag** | |

### How It Works

1. **Normal mouse operations** (click/drag without modifier) go to the TUI:
   - Click tabs to switch views
   - Click sources to toggle filters
   - Click graph cells to see details

2. **Shift + drag** bypasses mouse capture and enables native terminal text selection:
   - Select any text in the TUI
   - Copy with your terminal's copy shortcut (Cmd+C / Ctrl+Shift+C)

This is the same behavior as OpenCode, Claude Code, and other TUI applications that use mouse capture.

### Ghostty-Specific Configuration

If you're using Ghostty and experiencing issues with selection extending incorrectly, add this to `~/.config/ghostty/config`:

```ini
# Prevent Shift from being sent to applications
mouse-shift-capture = never
```

This ensures Shift+drag always works for text selection without quirks.

### Technical Details

The TUI enables standard mouse capture modes (1000, 1002, 1003, 1006 via crossterm). Most modern terminals support modifier-key bypass for text selection even when mouse reporting is enabled.

**Terminal escape sequences enabled:**
- `\x1b[?1000h` - Basic mouse tracking
- `\x1b[?1002h` - Button event tracking
- `\x1b[?1003h` - Any event tracking
- `\x1b[?1006h` - SGR mouse encoding

These are the same sequences used by OpenTUI and other modern TUI frameworks.
