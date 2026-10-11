use std::cell::Cell;

use chrono::{Local, NaiveDateTime, TimeZone};
use crossterm::event::{KeyCode, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::data::SessionUsage;
use crate::tui::i18n::{tr, MessageKey, TuiLanguage};
use crate::tui::themes::Theme;
use crate::tui::ui::widgets::{
    ambient_stable_scrollbar, format_cache_hit_rate, format_cost, format_cost_per_million,
    format_tokens_with_commas, get_client_display_name, viewport_scrollbar_state,
    AMBIENT_STABLE_BORDER_SET,
};

use super::{DialogContent, DialogResult};

/// Modal inspector presenting comprehensive metadata and metrics for a single session.
pub struct SessionDetailDialog {
    session: SessionUsage,
    lang: TuiLanguage,
    scroll_offset: Cell<usize>,
    total_lines: Cell<usize>,
    visible_height: Cell<usize>,
}

impl SessionDetailDialog {
    pub fn new(session: SessionUsage, lang: TuiLanguage) -> Self {
        Self {
            session,
            lang,
            scroll_offset: Cell::new(0),
            total_lines: Cell::new(0),
            visible_height: Cell::new(0),
        }
    }

    /// Current vertical scroll offset (for testing).
    #[cfg(test)]
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset.get()
    }

    fn build_lines(&self, theme: &Theme, inner_width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let avail_width = inner_width as usize;

        // Section 1: Overview
        lines.push(section_header(
            tr(self.lang, MessageKey::SessionDetailSectionOverview),
            theme,
            inner_width,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::SessionDetailId),
            Span::styled(
                self.session.session_id.clone(),
                Style::default().fg(theme.foreground),
            ),
            theme,
        ));

        if let Some(ref title) = self.session.title {
            lines.push(kv_line(
                tr(self.lang, MessageKey::SessionDetailTitle),
                Span::styled(title.clone(), Style::default().fg(theme.foreground)),
                theme,
            ));
        }

        let client_name = get_client_display_name(&self.session.client);
        lines.push(kv_line(
            tr(self.lang, MessageKey::ColClient),
            Span::styled(client_name, Style::default().fg(theme.foreground)),
            theme,
        ));

        if let Some(ref wk_label) = self.session.workspace_label {
            lines.push(kv_line(
                tr(self.lang, MessageKey::ColWorkspace),
                Span::styled(wk_label.clone(), Style::default().fg(theme.foreground)),
                theme,
            ));
        }

        if let Some(ref dir) = self.session.workspace_key {
            lines.extend(wrapped_kv_lines(
                tr(self.lang, MessageKey::SessionDetailDirectory),
                dir,
                Color::Cyan,
                theme,
                avail_width,
            ));
        } else {
            lines.push(kv_line(
                tr(self.lang, MessageKey::SessionDetailDirectory),
                Span::styled("\u{2014}", Style::default().fg(theme.muted)),
                theme,
            ));
        }

        if self.session.models.is_empty() {
            lines.push(kv_line(
                tr(self.lang, MessageKey::ColModels),
                Span::styled("\u{2014}", Style::default().fg(theme.muted)),
                theme,
            ));
        } else if self.session.models.len() == 1 {
            let m = &self.session.models[0];
            let label = if m.provider.is_empty() {
                m.display_name.clone()
            } else {
                format!("{} ({})", m.display_name, m.provider)
            };
            lines.push(kv_line(
                tr(self.lang, MessageKey::ColModels),
                Span::styled(label, Style::default().fg(theme.foreground)),
                theme,
            ));
        } else {
            let first_m = &self.session.models[0];
            let first_label = if first_m.provider.is_empty() {
                first_m.display_name.clone()
            } else {
                format!("{} ({})", first_m.display_name, first_m.provider)
            };
            lines.push(kv_line(
                tr(self.lang, MessageKey::ColModels),
                Span::styled(first_label, Style::default().fg(theme.foreground)),
                theme,
            ));
            for m in &self.session.models[1..] {
                let label = if m.provider.is_empty() {
                    m.display_name.clone()
                } else {
                    format!("{} ({})", m.display_name, m.provider)
                };
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(18)),
                    Span::styled(label, Style::default().fg(theme.foreground)),
                ]));
            }
        }

        if !self.session.agents.is_empty() {
            lines.push(kv_line(
                tr(self.lang, MessageKey::ColAgent),
                Span::styled(
                    self.session.agents.join(", "),
                    Style::default().fg(theme.foreground),
                ),
                theme,
            ));
        }

        if self.session.subagent_count > 0 {
            lines.push(kv_line(
                tr(self.lang, MessageKey::SessionDetailSubagents),
                Span::styled(self.session.subagent_count.to_string(), theme.count_style()),
                theme,
            ));
        }

        // Section 2: Activity
        lines.push(Line::raw(""));
        lines.push(section_header(
            tr(self.lang, MessageKey::SessionDetailSectionActivity),
            theme,
            inner_width,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColTurn),
            Span::styled(
                format_tokens_with_commas(self.session.turn_count as u64),
                Style::default().fg(theme.foreground),
            ),
            theme,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColMessages),
            Span::styled(
                format_tokens_with_commas(self.session.message_count as u64),
                Style::default().fg(theme.foreground),
            ),
            theme,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColDuration),
            Span::styled(
                format_duration(self.session.first_active_ms, self.session.last_active_ms),
                theme.hint_key_style(),
            ),
            theme,
        ));

        let first_active_str = ms_to_local_naive(self.session.first_active_ms)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "\u{2014}".to_string());
        lines.push(kv_line(
            tr(self.lang, MessageKey::SessionDetailFirstActive),
            Span::styled(first_active_str, Style::default().fg(theme.muted)),
            theme,
        ));

        let last_active_str = ms_to_local_naive(self.session.last_active_ms)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "\u{2014}".to_string());
        lines.push(kv_line(
            tr(self.lang, MessageKey::ColLastActive),
            Span::styled(last_active_str, Style::default().fg(theme.muted)),
            theme,
        ));

        // Section 3: Tokens & Cost
        lines.push(Line::raw(""));
        lines.push(section_header(
            tr(self.lang, MessageKey::SessionDetailSectionTokens),
            theme,
            inner_width,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColCost),
            Span::styled(
                format_cost(self.session.cost),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            theme,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColCostPer1M),
            Span::styled(
                format_cost_per_million(self.session.cost, self.session.tokens.total()),
                Style::default().fg(Color::Rgb(150, 200, 150)),
            ),
            theme,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColInput),
            Span::styled(
                format_tokens_with_commas(self.session.tokens.input),
                Style::default().fg(theme.foreground),
            ),
            theme,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColOutput),
            Span::styled(
                format_tokens_with_commas(self.session.tokens.output),
                Style::default().fg(theme.foreground),
            ),
            theme,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColCacheRead),
            Span::styled(
                format_tokens_with_commas(self.session.tokens.cache_read),
                Style::default().fg(theme.foreground),
            ),
            theme,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColCacheWrite),
            Span::styled(
                format_tokens_with_commas(self.session.tokens.cache_write),
                Style::default().fg(theme.foreground),
            ),
            theme,
        ));

        if self.session.tokens.reasoning > 0 {
            lines.push(kv_line(
                tr(self.lang, MessageKey::SessionDetailReasoning),
                Span::styled(
                    format_tokens_with_commas(self.session.tokens.reasoning),
                    Style::default().fg(theme.foreground),
                ),
                theme,
            ));
        }

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColTotal),
            Span::styled(
                format_tokens_with_commas(self.session.tokens.total()),
                theme.metric_total_style(),
            ),
            theme,
        ));

        lines.push(kv_line(
            tr(self.lang, MessageKey::ColCacheHit),
            Span::styled(
                format_cache_hit_rate(
                    self.session.tokens.cache_read,
                    self.session.tokens.input,
                    self.session.tokens.cache_write,
                ),
                theme.count_style(),
            ),
            theme,
        ));

        lines
    }
}

impl DialogContent for SessionDetailDialog {
    fn desired_size(&self, viewport: Rect) -> (u16, u16) {
        let width = 78u16.min(viewport.width.saturating_sub(4)).max(20);
        let height = 28u16.min(viewport.height.saturating_sub(2)).max(8);
        (width, height)
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .title(tr(self.lang, MessageKey::SessionDetailDialogTitle))
            .borders(Borders::ALL)
            .border_set(AMBIENT_STABLE_BORDER_SET)
            .border_style(Style::default().fg(theme.accent));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 2 || inner.width < 10 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let content_area = chunks[0];
        let footer_area = chunks[1];

        let lines = self.build_lines(theme, content_area.width);
        let total_lines = lines.len();
        let visible_height = content_area.height as usize;

        self.total_lines.set(total_lines);
        self.visible_height.set(visible_height);

        let max_scroll = total_lines.saturating_sub(visible_height);
        let effective_scroll = self.scroll_offset.get().min(max_scroll);
        self.scroll_offset.set(effective_scroll);

        let paragraph = Paragraph::new(lines).scroll((effective_scroll as u16, 0));
        frame.render_widget(paragraph, content_area);

        if total_lines > visible_height {
            let scrollbar = ambient_stable_scrollbar();
            let mut scrollbar_state =
                viewport_scrollbar_state(total_lines, effective_scroll, visible_height);
            frame.render_stateful_widget(scrollbar, content_area, &mut scrollbar_state);
        }

        let hint = tr(self.lang, MessageKey::SessionDetailDialogHint);
        let footer_para = Paragraph::new(Line::from(vec![Span::styled(
            hint,
            Style::default().fg(theme.muted),
        )]))
        .alignment(Alignment::Center);
        frame.render_widget(footer_para, footer_area);
    }

    fn handle_key(&mut self, key: KeyCode) -> DialogResult {
        match key {
            KeyCode::Esc
            | KeyCode::Enter
            | KeyCode::Char('q')
            | KeyCode::Char('Q')
            | KeyCode::Char('i')
            | KeyCode::Char('I')
            | KeyCode::Char(' ') => DialogResult::Close,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                let max_scroll = self
                    .total_lines
                    .get()
                    .saturating_sub(self.visible_height.get());
                self.scroll_offset
                    .set((self.scroll_offset.get() + 1).min(max_scroll));
                DialogResult::None
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                self.scroll_offset
                    .set(self.scroll_offset.get().saturating_sub(1));
                DialogResult::None
            }
            KeyCode::PageDown => {
                let step = self.visible_height.get().max(1);
                let max_scroll = self
                    .total_lines
                    .get()
                    .saturating_sub(self.visible_height.get());
                self.scroll_offset
                    .set((self.scroll_offset.get() + step).min(max_scroll));
                DialogResult::None
            }
            KeyCode::PageUp => {
                let step = self.visible_height.get().max(1);
                self.scroll_offset
                    .set(self.scroll_offset.get().saturating_sub(step));
                DialogResult::None
            }
            KeyCode::Home => {
                self.scroll_offset.set(0);
                DialogResult::None
            }
            KeyCode::End => {
                let max_scroll = self
                    .total_lines
                    .get()
                    .saturating_sub(self.visible_height.get());
                self.scroll_offset.set(max_scroll);
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> DialogResult {
        match event.kind {
            MouseEventKind::ScrollDown => {
                let max_scroll = self
                    .total_lines
                    .get()
                    .saturating_sub(self.visible_height.get());
                self.scroll_offset
                    .set((self.scroll_offset.get() + 1).min(max_scroll));
                DialogResult::None
            }
            MouseEventKind::ScrollUp => {
                self.scroll_offset
                    .set(self.scroll_offset.get().saturating_sub(1));
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }
}

fn section_header(title: &str, theme: &Theme, inner_width: u16) -> Line<'static> {
    let title_width = UnicodeWidthStr::width(title);
    let dashes = (inner_width as usize)
        .saturating_sub(title_width + 4)
        .max(3);
    Line::from(vec![
        Span::styled("── ", Style::default().fg(theme.border)),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default()),
        Span::styled("─".repeat(dashes), Style::default().fg(theme.border)),
    ])
}

fn kv_line(label: &str, value: Span<'static>, theme: &Theme) -> Line<'static> {
    let label_width = UnicodeWidthStr::width(label);
    let pad = 16usize.saturating_sub(label_width);
    Line::from(vec![
        Span::styled(label.to_string(), Style::default().fg(theme.muted)),
        Span::styled(
            format!("{}: ", " ".repeat(pad)),
            Style::default().fg(theme.muted),
        ),
        value,
    ])
}

fn wrapped_kv_lines(
    label: &str,
    text: &str,
    color: Color,
    theme: &Theme,
    available_width: usize,
) -> Vec<Line<'static>> {
    let label_width = UnicodeWidthStr::width(label);
    let pad = 16usize.saturating_sub(label_width);
    let prefix_colon = format!("{}: ", " ".repeat(pad));
    let prefix_width = label_width + prefix_colon.len();

    // If available width cannot comfortably hold the prefix plus at least 6 chars of path,
    // put the label on its own row so the path gets the full row width with a small 2-space indent.
    if available_width <= prefix_width.saturating_add(6) {
        let mut out = vec![Line::from(vec![
            Span::styled(label.to_string(), Style::default().fg(theme.muted)),
            Span::styled(":", Style::default().fg(theme.muted)),
        ])];
        let val_width = available_width.saturating_sub(2).max(1);
        let mut remaining = text;
        while !remaining.is_empty() {
            let (chunk, rest) = chunk_str_to_width(remaining, val_width);
            remaining = rest;
            out.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(chunk.to_string(), Style::default().fg(color)),
            ]));
        }
        return out;
    }

    let val_width = available_width.saturating_sub(prefix_width).max(1);
    if text.is_empty() || UnicodeWidthStr::width(text) <= val_width {
        return vec![Line::from(vec![
            Span::styled(label.to_string(), Style::default().fg(theme.muted)),
            Span::styled(prefix_colon, Style::default().fg(theme.muted)),
            Span::styled(text.to_string(), Style::default().fg(color)),
        ])];
    }

    let indent_spaces = " ".repeat(prefix_width);
    let mut out = Vec::new();
    let mut remaining = text;
    let mut first = true;

    while !remaining.is_empty() {
        let (chunk, rest) = chunk_str_to_width(remaining, val_width);
        remaining = rest;
        if first {
            out.push(Line::from(vec![
                Span::styled(label.to_string(), Style::default().fg(theme.muted)),
                Span::styled(prefix_colon.clone(), Style::default().fg(theme.muted)),
                Span::styled(chunk.to_string(), Style::default().fg(color)),
            ]));
            first = false;
        } else {
            out.push(Line::from(vec![
                Span::raw(indent_spaces.clone()),
                Span::styled(chunk.to_string(), Style::default().fg(color)),
            ]));
        }
    }

    out
}

fn chunk_str_to_width(s: &str, max_width: usize) -> (&str, &str) {
    let mut current_width = 0;
    let mut split_byte = s.len();

    for (byte_idx, ch) in s.char_indices() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1);
        if current_width + ch_width > max_width {
            split_byte = byte_idx;
            break;
        }
        current_width += ch_width;
    }

    if split_byte == 0 {
        if let Some((next_byte, _)) = s.char_indices().nth(1) {
            split_byte = next_byte;
        } else {
            split_byte = s.len();
        }
    }

    s.split_at(split_byte)
}

fn ms_to_local_naive(ms: i64) -> Option<NaiveDateTime> {
    if ms <= 0 {
        return None;
    }
    let secs = ms / 1000;
    match Local.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => Some(dt.naive_local()),
        _ => None,
    }
}

fn format_duration(first_ms: i64, last_ms: i64) -> String {
    if first_ms <= 0 || last_ms <= 0 || last_ms < first_ms {
        return "\u{2014}".to_string();
    }
    let secs = (last_ms - first_ms) / 1000;
    if secs <= 0 {
        return "0s".to_string();
    }
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else if mins > 0 {
        if s > 0 {
            format!("{}m {}s", mins, s)
        } else {
            format!("{}m", mins)
        }
    } else {
        format!("{}s", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{SessionModel, TokenBreakdown};
    use crate::tui::themes::ThemeName;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn test_theme() -> Theme {
        Theme::from_name_for_current_terminal(ThemeName::Green)
    }

    fn test_session() -> SessionUsage {
        SessionUsage {
            session_id: "sess-abc-123".to_string(),
            client: "opencode".to_string(),
            title: Some("Implement hotkey inspector".to_string()),
            workspace_key: Some("/Users/user/workspace/tokscale".to_string()),
            workspace_label: Some("tokscale".to_string()),
            agents: vec!["primary".to_string(), "coder".to_string()],
            models: vec![
                SessionModel {
                    display_name: "claude-3-5-sonnet".to_string(),
                    provider: "anthropic".to_string(),
                    color_key: "claude-3-5-sonnet".to_string(),
                },
                SessionModel {
                    display_name: "gpt-4o".to_string(),
                    provider: "openai".to_string(),
                    color_key: "gpt-4o".to_string(),
                },
            ],
            tokens: TokenBreakdown {
                input: 120_000,
                output: 45_000,
                cache_read: 80_000,
                cache_write: 10_000,
                reasoning: 5_000,
            },
            cost: 1.2345,
            message_count: 52,
            turn_count: 14,
            first_active_ms: 1_700_000_000_000,
            last_active_ms: 1_700_003_600_000,
            subagent_count: 2,
        }
    }

    #[test]
    fn test_session_detail_dialog_desired_size() {
        let dialog = SessionDetailDialog::new(test_session(), TuiLanguage::En);
        let (w, h) = dialog.desired_size(Rect::new(0, 0, 100, 40));
        assert_eq!(w, 78);
        assert_eq!(h, 28);

        let (w_small, h_small) = dialog.desired_size(Rect::new(0, 0, 50, 15));
        assert_eq!(w_small, 46);
        assert_eq!(h_small, 13);
    }

    #[test]
    fn test_session_detail_dialog_dismiss_keys() {
        let mut dialog = SessionDetailDialog::new(test_session(), TuiLanguage::En);

        assert!(matches!(
            dialog.handle_key(KeyCode::Esc),
            DialogResult::Close
        ));
        assert!(matches!(
            dialog.handle_key(KeyCode::Enter),
            DialogResult::Close
        ));
        assert!(matches!(
            dialog.handle_key(KeyCode::Char('q')),
            DialogResult::Close
        ));
        assert!(matches!(
            dialog.handle_key(KeyCode::Char('Q')),
            DialogResult::Close
        ));
        assert!(matches!(
            dialog.handle_key(KeyCode::Char('i')),
            DialogResult::Close
        ));
        assert!(matches!(
            dialog.handle_key(KeyCode::Char('I')),
            DialogResult::Close
        ));
        assert!(matches!(
            dialog.handle_key(KeyCode::Char(' ')),
            DialogResult::Close
        ));
    }

    #[test]
    fn test_session_detail_dialog_scrolling() {
        let mut dialog = SessionDetailDialog::new(test_session(), TuiLanguage::En);
        dialog.total_lines.set(30);
        dialog.visible_height.set(10);

        assert_eq!(dialog.scroll_offset(), 0);
        assert!(matches!(
            dialog.handle_key(KeyCode::Down),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 1);

        assert!(matches!(
            dialog.handle_key(KeyCode::Char('j')),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 2);

        assert!(matches!(
            dialog.handle_key(KeyCode::PageDown),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 12);

        assert!(matches!(
            dialog.handle_key(KeyCode::End),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 20);

        assert!(matches!(dialog.handle_key(KeyCode::Up), DialogResult::None));
        assert_eq!(dialog.scroll_offset(), 19);

        assert!(matches!(
            dialog.handle_key(KeyCode::Char('k')),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 18);

        assert!(matches!(
            dialog.handle_key(KeyCode::PageUp),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 8);

        assert!(matches!(
            dialog.handle_key(KeyCode::Home),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 0);
    }

    #[test]
    fn test_session_detail_dialog_mouse_scroll() {
        let mut dialog = SessionDetailDialog::new(test_session(), TuiLanguage::En);
        dialog.total_lines.set(25);
        dialog.visible_height.set(10);

        let area = Rect::new(0, 0, 78, 28);
        assert!(matches!(
            dialog.handle_mouse(
                MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: 10,
                    row: 10,
                    modifiers: crossterm::event::KeyModifiers::empty(),
                },
                area
            ),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 1);

        assert!(matches!(
            dialog.handle_mouse(
                MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    column: 10,
                    row: 10,
                    modifiers: crossterm::event::KeyModifiers::empty(),
                },
                area
            ),
            DialogResult::None
        ));
        assert_eq!(dialog.scroll_offset(), 0);
    }

    #[test]
    fn test_session_detail_dialog_renders_all_fields() {
        let session = test_session();
        let dialog = SessionDetailDialog::new(session, TuiLanguage::En);
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = test_theme();

        terminal
            .draw(|f| {
                dialog.render(f, Rect::new(0, 0, 80, 30), &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let rendered_text: String = buffer.content().iter().map(|c| c.symbol()).collect();

        assert!(rendered_text.contains("Session Details"));
        assert!(rendered_text.contains("sess-abc-123"));
        assert!(rendered_text.contains("/Users/user/workspace/tokscale"));
        assert!(rendered_text.contains("Implement hotkey inspector"));
        assert!(rendered_text.contains("claude-3-5-sonnet"));
        assert!(rendered_text.contains("gpt-4o"));
        assert!(rendered_text.contains("primary"));
        assert!(rendered_text.contains("coder"));
        assert!(rendered_text.contains("Reasoning"));
        assert!(rendered_text.contains("Esc/q/Enter/i close"));
    }

    #[test]
    fn test_session_detail_dialog_scroll_clamped_on_render() {
        let session = test_session();
        let dialog = SessionDetailDialog::new(session, TuiLanguage::En);
        dialog.scroll_offset.set(100);

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = test_theme();

        terminal
            .draw(|f| {
                dialog.render(f, Rect::new(0, 0, 80, 20), &theme);
            })
            .unwrap();

        let max_scroll = dialog
            .total_lines
            .get()
            .saturating_sub(dialog.visible_height.get());
        assert_eq!(dialog.scroll_offset(), max_scroll);
        assert!(dialog.scroll_offset() < 100);
    }

    #[test]
    fn test_session_detail_dialog_narrow_terminal_directory_wrapping() {
        let mut session = test_session();
        session.workspace_key = Some("/very/long/nested/path/to/project/workspace".to_string());
        let dialog = SessionDetailDialog::new(session, TuiLanguage::En);

        let backend = TestBackend::new(25, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = test_theme();

        terminal
            .draw(|f| {
                dialog.render(f, Rect::new(0, 0, 25, 20), &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let rendered_text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(rendered_text.contains("Directory"));
        assert!(rendered_text.contains("/very/long"));
    }

    #[test]
    fn test_session_detail_dialog_languages() {
        for lang in TuiLanguage::ALL {
            let session = test_session();
            let dialog = SessionDetailDialog::new(session, lang);
            let backend = TestBackend::new(80, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            let theme = test_theme();

            terminal
                .draw(|f| {
                    dialog.render(f, Rect::new(0, 0, 80, 30), &theme);
                })
                .unwrap();
        }
    }
}
