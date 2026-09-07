use ratatui::prelude::*;
use ratatui::widgets::{Cell, ScrollbarState};
use tokscale_core::sessions::WORKTREE_SEPARATOR;
use tokscale_core::ClientId;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::tui::client_ui;
use crate::tui::config::TokscaleConfig;
use crate::tui::themes::Theme;

pub fn format_tokens_compact(tokens: u64) -> String {
    if tokens >= 1_000_000_000 {
        format!("{:.1}B", tokens as f64 / 1_000_000_000.0)
    } else if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}K", tokens / 1_000)
    } else {
        format_tokens_with_commas(tokens)
    }
}

pub fn format_tokens(tokens: u64) -> String {
    format_tokens_compact(tokens)
}

pub(crate) fn total_tokens_cell(total_tokens: u64, theme: &Theme) -> Cell<'static> {
    Cell::from(format_tokens(total_tokens)).style(theme.metric_total_style())
}

pub fn format_tokens_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.insert(0, ',');
        }
        result.insert(0, c);
    }
    result
}

pub fn format_cost(cost: f64) -> String {
    if !cost.is_finite() || cost < 0.0 {
        return "$0.00".to_string();
    }
    if cost >= 1000.0 {
        format!("${:.1}K", cost / 1000.0)
    } else {
        format!("${:.2}", cost)
    }
}

/// Cost per million tokens: useful for comparing model efficiency across sessions.
/// Returns "—" when there are no tokens to avoid division by zero.
pub fn format_cost_per_million(cost: f64, total_tokens: u64) -> String {
    if total_tokens == 0 || !cost.is_finite() || cost < 0.0 {
        return "\u{2014}".to_string(); // —
    }
    let per_m = cost / (total_tokens as f64) * 1_000_000.0;
    format!("${:.2}", per_m)
}

/// Cache reuse multiplier: cached reads per full-price input token.
/// `cache_read / (input + cache_write)` — how many low-cost reads you
/// got for every token you paid full price (fresh input or cache write).
pub fn format_cache_hit_rate(cache_read: u64, input: u64, cache_write: u64) -> String {
    let paid = input.saturating_add(cache_write);
    if paid == 0 {
        return if cache_read > 0 {
            "∞".to_string()
        } else {
            "—".to_string()
        };
    }
    let ratio = cache_read as f64 / paid as f64;
    format!("{:.1}x", ratio)
}

pub fn format_ms_per_1k(ms_per_1k_tokens: Option<f64>) -> String {
    let Some(value) = ms_per_1k_tokens else {
        return "—".to_string();
    };
    if !value.is_finite() || value <= 0.0 {
        "—".to_string()
    } else if value >= 1000.0 {
        format!("{:.1}s", value / 1000.0)
    } else {
        format!("{:.0}ms", value)
    }
}

pub fn viewport_scrollbar_state(
    content_len: usize,
    scroll_offset: usize,
    viewport_len: usize,
) -> ScrollbarState {
    let viewport_len = viewport_len.max(1);
    ScrollbarState::new(content_len)
        .position(scrollbar_position(scroll_offset, content_len, viewport_len))
        .viewport_content_length(viewport_len)
}

/// Truncate a string to `max_chars` Unicode code points, appending "..." when
/// truncation occurs. Returns the original string when it fits, and an empty
/// string when `max_chars` is 0. Shared across all table-style tabs (Daily,
/// Monthly, Sessions, Models, Agents) so ellipsis behavior stays consistent.
///
/// Counts code points, not terminal cells, so a CJK or emoji string it calls
/// short is twice as wide as the column that asked for it and gets clipped by
/// the solver with no marker. Every caller passes a column width, so cells are
/// the unit they all actually want: prefer [`truncate_to_width`] for anything
/// that can hold a non-ASCII name. Kept as-is only because the Daily, Models
/// and Agents tables are pinned by golden renders that this commit does not
/// touch.
pub fn truncate_text(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else if max_chars <= 3 {
        s.chars().take(max_chars).collect()
    } else {
        let head: String = s.chars().take(max_chars - 3).collect();
        format!("{}...", head)
    }
}

/// Terminal cells `s` occupies, which is the unit ratatui lays tables out in.
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Longest prefix of `s` that fits in `max_cells` terminal cells. Never splits
/// a grapheme, so the result can come in one cell short of the budget rather
/// than one over it.
pub fn prefix_to_width(s: &str, max_cells: usize) -> &str {
    let mut used = 0usize;
    // Grapheme clusters, not chars. A flag is two regional indicators and a
    // family emoji is a ZWJ sequence, so cutting on a char boundary can leave
    // half a flag or a dangling ZWJ -- and the width of the pieces does not
    // add up to the width of the whole, so the cut would not even respect the
    // budget it was measuring against.
    for (offset, cluster) in s.grapheme_indices(true) {
        let w = UnicodeWidthStr::width(cluster);
        if used + w > max_cells {
            return &s[..offset];
        }
        used += w;
    }
    s
}

/// [`truncate_text`] measured in terminal cells instead of code points: the
/// result never occupies more than `max_cells`, and carries a "..." whenever
/// anything was dropped. Use this wherever the text can hold a full-width
/// grapheme — a session title, a client display name — because those are the
/// cases where a code-point count and the ratatui solver disagree.
pub fn truncate_to_width(s: &str, max_cells: usize) -> String {
    if max_cells == 0 {
        return String::new();
    }
    if display_width(s) <= max_cells {
        return s.to_string();
    }
    if max_cells <= 3 {
        return prefix_to_width(s, max_cells).to_string();
    }
    format!("{}...", prefix_to_width(s, max_cells - 3))
}

/// Longest suffix of `s` that fits in `max_cells` terminal cells. The mirror of
/// [`prefix_to_width`], and grapheme-safe for the same reasons.
pub fn suffix_to_width(s: &str, max_cells: usize) -> &str {
    let mut used = 0usize;
    let mut start = s.len();
    for (offset, cluster) in s.grapheme_indices(true).rev() {
        let w = UnicodeWidthStr::width(cluster);
        if used + w > max_cells {
            break;
        }
        used += w;
        start = offset;
    }
    &s[start..]
}

/// One-cell ellipsis. A three-dot "..." costs three of the cells the elision is
/// trying to conserve, which matters at an 18-cell column.
///
/// U+22EF, not the usual U+2026 `…`. U+2026 is East-Asian-Ambiguous: terminals
/// running a CJK locale render it two cells wide, and a fitted label can carry
/// three markers, so the row that measured 18 cells here occupied 21 there and
/// pushed the rest of the table sideways. U+22EF is Neutral, so it is one cell
/// in both ambients and the budget this module computes is the width the
/// terminal actually uses.
pub(crate) const MIDDLE_ELLIPSIS: &str = "⋯";

/// Fit `s` into `max_cells` by dropping its MIDDLE instead of its tail.
///
/// Head-first truncation assumes the leading text identifies the row. Workspace
/// labels break that assumption: `repo ⑃ worktree` rows share the repo, and
/// disambiguated labels share the parent segments too, so the only part that
/// tells two rows apart is the part a tail-cut discards. At an 18-cell column
/// that turned six distinct worktrees into six identical `tokscale ⑃ wo...`
/// rows. Keeping both ends keeps the discriminating segment on screen whichever
/// end of the label it sits at.
pub fn elide_middle_to_width(s: &str, max_cells: usize) -> String {
    if max_cells == 0 {
        return String::new();
    }
    if display_width(s) <= max_cells {
        return s.to_string();
    }
    // Below this there is no room for two halves and a marker, so an ordinary
    // head cut is all that is left.
    if max_cells <= 2 {
        return truncate_to_width(s, max_cells);
    }

    let budget = max_cells - 1;
    let head_cells = budget / 2;
    // The odd cell goes to the tail: the discriminating segment is at the end
    // far more often than at the start.
    let head = prefix_to_width(s, head_cells);
    let tail = suffix_to_width(s, budget - head_cells);
    // Zero-width clusters can let the two halves meet even though the string is
    // too wide; splicing then would duplicate text rather than elide it.
    if head.len() + tail.len() >= s.len() {
        return truncate_to_width(s, max_cells);
    }
    format!("{head}{MIDDLE_ELLIPSIS}{tail}")
}

/// Cells a path segment must be granted before showing it beats dropping it:
/// two characters of content on either side of the elision marker.
const MIN_SEGMENT_CELLS: usize = 4;

/// Fit a workspace label into `max_cells` while keeping every part of it
/// identifiable.
///
/// A workspace label is a path-shaped string — `parent/repo ⑃ worktree`,
/// `qualifier/name`, `name (key)` — whose segments are each identified by their
/// own ends. Cutting the string once, from either end, throws away whole
/// segments: at the 18 cells this column gets on an ordinary terminal a head cut
/// rendered 915 of 1181 real workspace rows as some other row's twin. This
/// budgets the column across the segments instead, so each one contributes as
/// much of itself as the row can afford, and elides each segment's middle rather
/// than its tail.
///
/// Measured over 1181 real workspace keys, counting rows that render to a string
/// some other row also renders to: 915 keys in 52 groups at 18 cells before,
/// 30 keys in 15 groups after; 757/31 to 0 at 44 cells; 749/28 to 0 at 60.
///
/// That is a large net reduction in rendered collisions, not a strict
/// improvement per key. Spending the column on every segment's ends can merge a
/// pair that a single head cut happened to keep apart, because the head cut kept
/// one segment whole. On an independent 2364-label corpus at 18 cells this
/// removed 29457 colliding pairs and introduced 42 (13 plain paths, 29
/// qualified) against a head cut, and introduced 17 against the previous
/// fitter — `agent-runtime ⑃ desktop-skill-session` and
/// `agent-runtime ⑃ desktop-window-drag-permission` render alike here and did
/// not before. The column is a summary; the full key stays in `--json` and in
/// the wider layouts this same function produces.
pub fn fit_workspace_label_to_width(label: &str, max_cells: usize) -> String {
    if display_width(label) <= max_cells {
        return label.to_string();
    }
    if max_cells <= 2 {
        return truncate_to_width(label, max_cells);
    }

    match split_qualifying_key(label) {
        Some((name, key)) => fit_qualified_label(name, key, max_cells),
        None => fit_path_label(label, max_cells),
    }
}

/// Fixed cells a ` (key)` qualifier costs regardless of the key: the separating
/// space and the two parentheses. Held out of the budget rather than elided, so
/// the qualifier always reads as one.
const KEY_QUALIFIER_CELLS: usize = 3;

/// Split `name (key)` into its two halves.
///
/// `disambiguate_workspace_labels` appends the grouping key verbatim to labels
/// the filesystem could not separate, so the parenthesised part is a whole path
/// and the only unique thing on the row. Treating it as ordinary path segments
/// is what made it lose to them.
fn split_qualifying_key(label: &str) -> Option<(&str, &str)> {
    let inner = label.strip_suffix(')')?;
    let open = inner.rfind(" (")?;
    Some((&label[..open], &inner[open + 2..]))
}

/// Fit `name (key)` by giving the name what it needs and the key what is left.
///
/// The key is a path, but it is not part of the name's path, and the two are not
/// peers: the name is what the row is read by, and the key is a tiebreaker that
/// only has to contribute its own ends. Water-filling them together spent the
/// column on whichever happened to be longer — `/Users/junhoyeo/tokscale-2` and
/// `/Users/junhoyeo/tokscale-giwa-2` both rendered `tok⋯-2 (/⋯/tok⋯-2)` at 18
/// cells, worse than the unqualified labels they started from, because the key
/// took half the row and then collapsed its own interior. The name is capped
/// only by the floor every segment gets, so what is left for the key is enough
/// to show where it starts and where it ends and nothing more.
fn fit_qualified_label(name: &str, key: &str, max_cells: usize) -> String {
    let budget = max_cells.saturating_sub(KEY_QUALIFIER_CELLS);
    if budget < MIN_SEGMENT_CELLS * 2 {
        // No room to say anything about both halves; the qualifier is dead
        // weight, so spend every cell on the name.
        return elide_middle_to_width(&format!("{name} ({key})"), max_cells);
    }

    let name_cells = display_width(name).min(budget - MIN_SEGMENT_CELLS);
    format!(
        "{} ({})",
        fit_path_label(name, name_cells),
        elide_middle_to_width(key, budget - name_cells)
    )
}

/// [`fit_workspace_label_to_width`] for a label that is only a path.
fn fit_path_label(label: &str, max_cells: usize) -> String {
    if display_width(label) <= max_cells {
        return label.to_string();
    }
    if max_cells <= 2 {
        return truncate_to_width(label, max_cells);
    }

    let (mut segments, mut separators) = split_label_parts(label);
    if segments.len() < 2 {
        return elide_middle_to_width(label, max_cells);
    }

    // The separators are not negotiable: they are what makes the result read as
    // a path rather than as one mangled word.
    let separator_cells =
        |separators: &[&str]| separators.iter().map(|s| display_width(s)).sum::<usize>();
    let mut budget = max_cells.saturating_sub(separator_cells(&separators));
    let mut collapsed_interior = false;
    if budget < MIN_SEGMENT_CELLS * segments.len() && segments.len() > 2 {
        // Not enough cells to say anything about every segment. Keep the two
        // that carry the identity — the qualifier and the name — and say so with
        // a marker rather than quietly rendering a path that does not exist.
        separators = vec![separators[0], separators[separators.len() - 1]];
        segments = vec![segments[0], segments[segments.len() - 1]];
        collapsed_interior = true;
        // The outer separators, kept so the result still reads as what it is,
        // and the one-cell marker between them.
        budget = max_cells.saturating_sub(separator_cells(&separators) + 1);
    }
    if budget < MIN_SEGMENT_CELLS * segments.len() {
        return elide_middle_to_width(label, max_cells);
    }

    let widths: Vec<usize> = segments.iter().map(|s| display_width(s)).collect();
    let allocation = allocate_segment_cells(&widths, budget);
    let parts: Vec<String> = segments
        .iter()
        .zip(&allocation)
        .map(|(segment, cells)| elide_middle_to_width(segment, *cells))
        .collect();

    if collapsed_interior {
        return format!(
            "{}{}{MIDDLE_ELLIPSIS}{}{}",
            parts[0], separators[0], separators[1], parts[1]
        );
    }
    let mut fitted = parts[0].clone();
    for (separator, part) in separators.iter().zip(&parts[1..]) {
        fitted.push_str(separator);
        fitted.push_str(part);
    }
    fitted
}

/// Split a label into its parts and the separators between them.
///
/// Both `/` and [`WORKTREE_SEPARATOR`] are structural: `repo ⑃ worktree` is two
/// names the same way `parent/repo` is. Splitting on `/` alone made the repo
/// half — which every worktree of one repo shares — and the worktree name one
/// inseparable segment, so a middle elision cut through the only part that
/// differed and six worktrees of `wrks-sisyphus` rendered as one row again.
fn split_label_parts(label: &str) -> (Vec<&str>, Vec<&str>) {
    let mut parts: Vec<&str> = Vec::new();
    let mut separators: Vec<&str> = Vec::new();
    let mut rest = label;
    while !rest.is_empty() {
        let next = rest
            .find('/')
            .map(|index| (index, "/"))
            .into_iter()
            .chain(
                rest.find(WORKTREE_SEPARATOR)
                    .map(|index| (index, WORKTREE_SEPARATOR)),
            )
            .min_by_key(|(index, _)| *index);
        let Some((index, separator)) = next else {
            break;
        };
        parts.push(&rest[..index]);
        separators.push(separator);
        rest = &rest[index + separator.len()..];
    }
    parts.push(rest);
    (parts, separators)
}

/// Split `budget` cells across segments of the given widths.
///
/// Water-filling, not a proportional share: every segment is capped at the same
/// width, raised as high as the budget allows, and a segment shorter than the
/// cap keeps all of itself and leaves its surplus to the long ones. That is what
/// makes a label only a few cells too wide lose those cells from its longest
/// segment alone, instead of sprinkling an ellipsis through every segment.
fn allocate_segment_cells(widths: &[usize], budget: usize) -> Vec<usize> {
    let mut cap = MIN_SEGMENT_CELLS;
    let (mut low, mut high) = (MIN_SEGMENT_CELLS, widths.iter().copied().max().unwrap_or(0));
    while low <= high {
        let candidate = low + (high - low) / 2;
        let used: usize = widths.iter().map(|w| (*w).min(candidate)).sum();
        if used <= budget {
            cap = candidate;
            low = candidate + 1;
        } else {
            high = candidate - 1;
        }
    }

    let mut allocation: Vec<usize> = widths.iter().map(|w| (*w).min(cap)).collect();
    // Whatever the uniform cap left over goes to the segments still clipped,
    // longest first: they are the ones carrying the text that got cut.
    let mut remaining = budget.saturating_sub(allocation.iter().sum::<usize>());
    let mut order: Vec<usize> = (0..widths.len()).collect();
    order.sort_by_key(|index| std::cmp::Reverse(widths[*index]));
    while remaining > 0 {
        let mut progressed = false;
        for &index in &order {
            if remaining == 0 {
                break;
            }
            if allocation[index] < widths[index] {
                allocation[index] += 1;
                remaining -= 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    allocation
}

fn scrollbar_position(scroll_offset: usize, content_len: usize, viewport_len: usize) -> usize {
    let max_scroll = content_len.saturating_sub(viewport_len);
    if max_scroll == 0 {
        0
    } else {
        ((scroll_offset.min(max_scroll) as u128) * (content_len.saturating_sub(1) as u128)
            / (max_scroll as u128)) as usize
    }
}

pub(crate) fn light_ratio_bar_spans(
    ratio: f64,
    width: usize,
    fill_style: Style,
    empty_style: Style,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let ratio = ratio.clamp(0.0, 1.0);
    let scaled = ratio * width as f64;
    let trace = ratio > 0.0 && ratio < 0.01 && scaled < 1.0;
    let filled = if ratio > 0.0 && !trace {
        (scaled.round() as usize).clamp(1, width)
    } else {
        0
    };
    let empty = width.saturating_sub(filled + usize::from(trace));

    let mut spans = Vec::with_capacity(3);
    if filled > 0 {
        spans.push(Span::styled("█".repeat(filled), fill_style));
    }
    if trace {
        spans.push(Span::styled("▏", fill_style));
    }
    if empty > 0 {
        spans.push(Span::styled("·".repeat(empty), empty_style));
    }
    spans
}

pub(crate) fn truncate_ellipsis(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else if max_chars == 1 {
        "…".to_string()
    } else {
        let head: String = s.chars().take(max_chars - 1).collect();
        format!("{head}…")
    }
}

pub fn get_model_color(model: &str) -> Color {
    get_provider_shade(get_provider_from_model(model), 0)
}

/// Maps a lowercased provider id to its built-in vendor shade ramp, or `None`
/// when the provider has no known vendor palette. Kept as the single source of
/// truth so [`get_provider_shade`] and [`provider_has_palette`] never drift.
fn known_provider_palette(provider_lower: &str) -> Option<&'static [(u8, u8, u8)]> {
    let palette: &[(u8, u8, u8)] = match provider_lower {
        s if s.contains("anthropic") => &ANTHROPIC_SHADES,
        s if s.contains("openai") => &OPENAI_SHADES,
        s if s.contains("google") || s.contains("gemini") => &GOOGLE_SHADES,
        s if s.contains("deepseek") => &DEEPSEEK_SHADES,
        s if s.contains("xai") || s.contains("grok") => &XAI_SHADES,
        s if s.contains("zai") || s.contains("z_ai") || s.contains("zhipu") => &ZAI_SHADES,
        s if s.contains("moonshot") || s.contains("kimi") => &MOONSHOT_SHADES,
        s if s.contains("meta") || s.contains("llama") => &META_SHADES,
        s if s.contains("cursor") => &CURSOR_SHADES,
        s if s.contains("sakana") || s.contains("fugu") => &SAKANA_SHADES,
        _ => return None,
    };
    Some(palette)
}

/// Whether `provider` resolves to a known vendor color — either a built-in
/// shade ramp or a user `[colors.providers]` override. Gateway providers that
/// resell other vendors' models (e.g. `github-copilot`, `openrouter`) have no
/// vendor palette of their own and return `false`, so callers can color by the
/// model's own vendor instead of the neutral "unknown" gray.
pub fn provider_has_palette(provider: &str) -> bool {
    TokscaleConfig::load()
        .get_provider_color(provider)
        .is_some()
        || known_provider_palette(&provider.to_lowercase()).is_some()
}

/// Returns the shade for a given `(provider, rank)` pair.
/// Honors `[colors.providers]` config overrides at every rank by deriving
/// a 7-step lighten-to-white palette from the override base color.
pub fn get_provider_shade(provider: &str, rank: usize) -> Color {
    if let Some(base) = TokscaleConfig::load().get_provider_color(provider) {
        return shade_from_base(base, rank);
    }

    let p = provider.to_lowercase();
    let palette = known_provider_palette(&p).unwrap_or(&UNKNOWN_SHADES);

    let idx = rank.min(palette.len() - 1);
    let (r, g, b) = palette[idx];
    Color::Rgb(r, g, b)
}

/// Generates a 7-step monochromatic palette from `base` by interpolating
/// toward white. Factors roughly match the end-of-ramp lightness of the
/// hardcoded palettes so overrides feel visually consistent.
fn shade_from_base(base: Color, rank: usize) -> Color {
    const FACTORS: [f32; 7] = [0.00, 0.11, 0.22, 0.33, 0.44, 0.56, 0.67];
    let Color::Rgb(r, g, b) = base else {
        return base;
    };
    let idx = rank.min(FACTORS.len() - 1);
    let f = FACTORS[idx];
    let lerp = |c: u8| -> u8 {
        let c = c as f32;
        (c + (255.0 - c) * f).round().clamp(0.0, 255.0) as u8
    };
    Color::Rgb(lerp(r), lerp(g), lerp(b))
}

const ANTHROPIC_SHADES: [(u8, u8, u8); 7] = [
    (218, 119, 86),  // #DA7756
    (223, 136, 107), // #DF886B
    (227, 153, 128), // #E39980
    (232, 170, 149), // #E8AA95
    (236, 184, 166), // #ECB8A6
    (239, 197, 183), // #EFC5B7
    (243, 210, 199), // #F3D2C7
];

const OPENAI_SHADES: [(u8, u8, u8); 7] = [
    (16, 185, 129),  // #10B981
    (18, 208, 145),  // #12D091
    (20, 232, 162),  // #14E8A2
    (41, 236, 172),  // #29ECAC
    (61, 238, 179),  // #3DEEB3
    (97, 241, 193),  // #61F1C1
    (133, 244, 208), // #85F4D0
];

const GOOGLE_SHADES: [(u8, u8, u8); 7] = [
    (59, 130, 246),  // #3B82F6
    (83, 146, 247),  // #5392F7
    (108, 161, 248), // #6CA1F8
    (132, 177, 249), // #84B1F9
    (153, 190, 250), // #99BEFA
    (172, 202, 251), // #ACCAFB
    (190, 214, 252), // #BED6FC
];

const DEEPSEEK_SHADES: [(u8, u8, u8); 7] = [
    (6, 182, 212),   // #06B6D4
    (7, 203, 237),   // #07CBED
    (21, 215, 248),  // #15D7F8
    (45, 219, 249),  // #2DDBF9
    (66, 223, 250),  // #42DFFA
    (85, 226, 250),  // #55E2FA
    (105, 229, 251), // #69E5FB
];

const XAI_SHADES: [(u8, u8, u8); 7] = [
    (234, 179, 8),   // #EAB308
    (247, 192, 21),  // #F7C015
    (248, 199, 45),  // #F8C72D
    (249, 205, 70),  // #F9CD46
    (249, 211, 91),  // #F9D35B
    (250, 216, 110), // #FAD86E
    (251, 221, 129), // #FBDD81
];

const ZAI_SHADES: [(u8, u8, u8); 7] = [
    (168, 85, 247),  // #A855F7
    (181, 110, 249), // #B56EF9
    (193, 132, 250), // #C184FA
    (204, 153, 251), // #CC99FB
    (214, 172, 252), // #D6ACFC
    (224, 192, 253), // #E0C0FD
    (235, 213, 254), // #EBD5FE
];

const MOONSHOT_SHADES: [(u8, u8, u8); 7] = [
    (20, 184, 166),  // #14B8A6
    (35, 197, 178),  // #23C5B2
    (58, 207, 190),  // #3ACFBE
    (85, 216, 202),  // #55D8CA
    (112, 224, 212), // #70E0D4
    (143, 232, 222), // #8FE8DE
    (174, 240, 232), // #AEF0E8
];

const META_SHADES: [(u8, u8, u8); 7] = [
    (99, 102, 241),  // #6366F1
    (122, 125, 243), // #7A7DF3
    (146, 148, 245), // #9294F5
    (169, 171, 247), // #A9ABF7
    (189, 190, 249), // #BDBEF9
    (207, 208, 251), // #CFD0FB
    (225, 226, 252), // #E1E2FC
];

const CURSOR_SHADES: [(u8, u8, u8); 7] = [
    (139, 92, 246),  // #8B5CF6
    (154, 114, 247), // #9A72F7
    (169, 135, 248), // #A987F8
    (184, 156, 250), // #B89CFA
    (199, 177, 251), // #C7B1FB
    (215, 199, 252), // #D7C7FC
    (230, 220, 253), // #E6DCFD
];

/// Sakana (Fugu) red — the brand's "one red fish leading the school" accent.
const SAKANA_SHADES: [(u8, u8, u8); 7] = [
    (219, 43, 31),   // #DB2B1F
    (223, 66, 56),   // #DF4238
    (227, 90, 80),   // #E35A50
    (231, 113, 105), // #E77169
    (235, 136, 130), // #EB8882
    (239, 162, 156), // #EFA29C
    (243, 185, 181), // #F3B9B5
];

/// Neutral gray ramp for providers that don't match any known palette.
/// Still produces distinct shades per rank instead of collapsing to white.
const UNKNOWN_SHADES: [(u8, u8, u8); 7] = [
    (136, 136, 136), // #888888
    (156, 156, 156), // #9C9C9C
    (176, 176, 176), // #B0B0B0
    (196, 196, 196), // #C4C4C4
    (212, 212, 212), // #D4D4D4
    (228, 228, 228), // #E4E4E4
    (244, 244, 244), // #F4F4F4
];

fn is_delimited_model_family(model_lower: &str, family: &str) -> bool {
    model_lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| {
            token == family
                || token
                    .strip_prefix(family)
                    .and_then(|suffix| suffix.chars().next())
                    .is_some_and(|c| c.is_ascii_digit())
        })
}

pub fn get_provider_from_model(model: &str) -> &'static str {
    let model_lower = model.to_lowercase();

    if model_lower.contains("claude")
        || model_lower.contains("sonnet")
        || model_lower.contains("opus")
        || model_lower.contains("haiku")
        // Match "fable" only as a delimited token (mirrors core's
        // provider_identity::contains_delimited) so unrelated names like
        // "unfabled-x" don't get misattributed to Anthropic.
        || model_lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| token == "fable")
    {
        "anthropic"
    } else if model_lower.contains("gpt")
        || model_lower.starts_with("o1")
        || model_lower.starts_with("o3")
        || model_lower.contains("codex")
        || model_lower.contains("text-embedding")
        || model_lower.contains("dall-e")
        || model_lower.contains("whisper")
        || model_lower.contains("tts")
    {
        "openai"
    } else if model_lower.contains("gemini") {
        "google"
    } else if model_lower.contains("deepseek") {
        "deepseek"
    } else if model_lower.contains("grok") {
        "xai"
    } else if is_delimited_model_family(&model_lower, "glm") {
        "zai"
    } else if is_delimited_model_family(&model_lower, "kimi") {
        "moonshotai"
    } else if model_lower.contains("llama") {
        "meta"
    } else if model_lower.contains("mixtral") {
        "mistral"
    } else if model_lower == "auto" || model_lower.contains("cursor") {
        "cursor"
    } else {
        "unknown"
    }
}

pub fn get_client_color(client: &str) -> Color {
    let config = TokscaleConfig::load();
    if let Some(color) = config.get_client_color(client) {
        return color;
    }
    match client.to_lowercase().as_str() {
        "opencode" => Color::Rgb(34, 197, 94),     // #22c55e
        "claude" => Color::Rgb(218, 119, 86),      // #DA7756 Claude brand coral
        "codex" => Color::Rgb(59, 130, 246),       // #3b82f6
        "cursor" => Color::Rgb(168, 85, 247),      // #a855f7
        "gemini" => Color::Rgb(6, 182, 212),       // #06b6d4
        "amp" => Color::Rgb(236, 72, 153),         // #EC4899
        "droid" => Color::Rgb(16, 185, 129),       // #10b981
        "openclaw" => Color::Rgb(239, 68, 68),     // #ef4444
        "hermes" => Color::Rgb(255, 215, 0),       // #ffd700
        "goose" => Color::Rgb(100, 180, 220),      // #64b4dc
        "codebuff" => Color::Rgb(124, 58, 237),    // #7C3AED Codebuff brand purple
        "freebuff" => Color::Rgb(56, 189, 248),    // #38BDF8 Freebuff sky blue
        "antigravity" => Color::Rgb(99, 102, 241), // #6366F1 Antigravity indigo
        "zed" => Color::Rgb(8, 76, 207),           // #084CCF Zed blue
        "warp" => Color::Rgb(1, 155, 150),         // #019B96 Warp teal
        "gjc" => Color::Rgb(220, 38, 38),          // #DC2626 gajae-code red-claw
        "jcode" => Color::Rgb(245, 158, 11),       // #F59E0B Jcode amber
        "junie" => Color::Rgb(123, 97, 255),       // #7B61FF Junie violet
        "prime-agent" => Color::Rgb(108, 99, 255), // #6C63FF Prime violet
        "unsloth" => Color::Rgb(88, 204, 2),       // #58CC02 Unsloth green
        _ => Color::Rgb(136, 136, 136),            // #888888
    }
}

fn registered_client_display_name(client: &str) -> Option<&'static str> {
    ClientId::from_str(&client.to_lowercase()).map(|client_id| client_id.display_name())
}

fn registered_compact_client_display_name(client: &str) -> Option<&'static str> {
    ClientId::from_str(&client.to_lowercase()).map(client_ui::compact_display_name)
}

pub fn get_client_display_name(client: &str) -> String {
    let config = TokscaleConfig::load();
    if let Some(name) = config.get_client_display_name(client) {
        return name.to_string();
    }
    registered_client_display_name(client)
        .unwrap_or(client)
        .to_string()
}

pub fn get_compact_client_display_name(client: &str) -> String {
    let config = TokscaleConfig::load();
    if let Some(name) = config.get_client_display_name(client) {
        return name.to_string();
    }
    registered_compact_client_display_name(client)
        .unwrap_or(client)
        .to_string()
}

pub fn get_provider_display_name(provider: &str) -> String {
    let config = TokscaleConfig::load();
    if let Some(name) = config.get_provider_display_name(provider) {
        return name.to_string();
    }

    // Merged Models rows store multiple providers as a ", "-joined string
    // (aggregate_model_usage_entries sorts + dedups + joins). Map EACH segment
    // independently and rejoin, otherwise a prefix/brand branch below would
    // match the whole string and silently drop the rest — e.g.
    // "openai, openrouter" must render "OpenAI, OpenRouter", not just "OpenAI".
    if provider.contains(", ") {
        return provider
            .split(", ")
            .map(|segment| map_single_provider(segment, config))
            .collect::<Vec<_>>()
            .join(", ");
    }

    map_single_provider(provider, config)
}

/// Display name for a SINGLE provider id (no comma-joined lists — the public
/// `get_provider_display_name` splits those first).
fn map_single_provider(provider: &str, config: &TokscaleConfig) -> String {
    if let Some(name) = config.get_provider_display_name(provider) {
        return name.to_string();
    }
    let lower = provider.to_lowercase();
    match lower.as_str() {
        "anthropic" => return "Anthropic".to_string(),
        "google" => return "Google".to_string(),
        "cursor" => return "Cursor".to_string(),
        "deepseek" => return "DeepSeek".to_string(),
        "xai" => return "xAI".to_string(),
        "meta" => return "Meta".to_string(),
        "mistral" => return "Mistral".to_string(),
        "cohere" => return "Cohere".to_string(),
        "opencode" => return "OpenCode".to_string(),
        "openrouter" => return "OpenRouter".to_string(),
        "orcarouter" => return "OrcaRouter".to_string(),
        // `canonical_provider` rewrites `google-vertex` → `google_vertex`, so
        // accept both spellings here.
        "google-vertex" | "google_vertex" => return "Google Vertex".to_string(),
        _ => {}
    }

    // Brand families: any provider id that starts with these stems collapses to
    // the brand name. Covers `openai`, `openai-codex`, `kimi`, `kimi-code`,
    // `kimi-for-coding`, etc. without enumerating every variant.
    if lower.starts_with("openai") {
        return "OpenAI".to_string();
    }
    if lower.starts_with("kimi") {
        return "Kimi".to_string();
    }
    if lower.starts_with("github-cop") || lower.contains("copilot") {
        return "GitHub Copilot".to_string();
    }

    // Smart fallback: split on `-`, `_`, and whitespace, title-case each word,
    // and map known acronyms/brands per word. So unknown multi-word providers
    // like `google-vertex` → "Google Vertex" and `some-new-provider` →
    // "Some New Provider".
    smart_titlecase(provider)
}

/// Title-cases a provider/brand identifier word-by-word, splitting on `-`, `_`,
/// and whitespace. Per-word acronym/brand overrides (e.g. `ai` → "AI",
/// `gpt` → "GPT") win over plain capitalization. Empty input yields an empty
/// string; runs of separators are collapsed.
fn smart_titlecase(s: &str) -> String {
    s.split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .map(titlecase_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn titlecase_word(word: &str) -> String {
    match word.to_lowercase().as_str() {
        "ai" => "AI".to_string(),
        "gpt" => "GPT".to_string(),
        "openai" => "OpenAI".to_string(),
        "xai" => "xAI".to_string(),
        "vertex" => "Vertex".to_string(),
        "llm" => "LLM".to_string(),
        "api" => "API".to_string(),
        _ => capitalize_first(word),
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_client_display_names_cover_the_core_registry() {
        for client in ClientId::iter() {
            assert_eq!(
                registered_client_display_name(client.as_str()),
                Some(client.display_name())
            );
            assert_eq!(
                registered_compact_client_display_name(client.as_str()),
                Some(client_ui::compact_display_name(client))
            );
        }
        assert_eq!(ClientId::Senpi.display_name(), "Senpi (OmO Native)");
        assert_eq!(client_ui::compact_display_name(ClientId::Senpi), "Senpi");
    }

    #[test]
    fn truncate_to_width_never_exceeds_its_budget() {
        // The case `truncate_text` gets wrong: 8 code points, 16 cells.
        let cjk = "세션제목한글로";
        assert_eq!(truncate_text(cjk, 10), cjk, "code points say it fits");
        assert!(
            display_width(&truncate_to_width(cjk, 10)) <= 10,
            "cells say it does not"
        );
        for budget in 0..=20 {
            for s in [cjk, "ascii-session-title", "🦞 OpenClaw", ""] {
                assert!(
                    display_width(&truncate_to_width(s, budget)) <= budget,
                    "{s:?} overflowed a {budget}-cell budget"
                );
            }
        }
    }

    #[test]
    fn truncate_to_width_marks_what_it_dropped() {
        assert_eq!(truncate_to_width("abcdefghij", 10), "abcdefghij");
        assert_eq!(truncate_to_width("abcdefghijk", 10), "abcdefg...");
        assert_eq!(truncate_to_width("abcdefghijk", 3), "abc");
        assert_eq!(truncate_to_width("abcdefghijk", 0), "");
        // 7 cells of budget, 3 for the marker: two syllables (4 cells) fit and
        // the third would overshoot, so the result is one cell short, not one
        // cell over.
        assert_eq!(truncate_to_width("세션제목", 7), "세션...");
    }

    #[test]
    fn prefix_to_width_never_splits_a_grapheme() {
        assert_eq!(prefix_to_width("세션제목", 5), "세션");
        assert_eq!(prefix_to_width("세션제목", 6), "세션제");
        assert_eq!(prefix_to_width("abc", 10), "abc");
        assert_eq!(prefix_to_width("abc", 0), "");

        // The cases the name actually promises. Hangul is one char per
        // grapheme, so the assertions above hold under char iteration too and
        // never exercised the claim. These are multi-char graphemes: a flag is
        // two regional indicators and a family is a ZWJ sequence, so char-wise
        // truncation yields half a flag or a dangling ZWJ.
        let flag = "\u{1F1F0}\u{1F1F7}"; // KR
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

        for (label, s) in [("flag", flag), ("family", family)] {
            assert_eq!(
                prefix_to_width(s, 1),
                "",
                "{label}: a 2-cell grapheme must not be halved to fit 1 cell"
            );
            assert_eq!(prefix_to_width(s, 2), s, "{label}: fits whole at 2 cells");
            // 'a' and 'b' take 2 of the 3 cells, leaving 1 — not enough for the
            // 2-cell cluster, which must therefore be dropped whole rather than
            // contributing a leading component.
            assert_eq!(
                prefix_to_width(&format!("ab{s}"), 3),
                "ab",
                "{label}: a cluster that does not fit is dropped, not split"
            );
        }
    }

    #[test]
    fn display_width_counts_cells_not_code_points() {
        assert_eq!(display_width("OpenCode"), 8);
        assert_eq!(display_width("🦞 OpenClaw"), 11);
        assert_eq!("🦞 OpenClaw".chars().count(), 10, "the disagreement");
    }

    #[test]
    fn scrollbar_position_maps_bottom_offset_to_last_position() {
        assert_eq!(scrollbar_position(15, 20, 5), 19);
    }

    #[test]
    fn scrollbar_position_keeps_top_at_zero() {
        assert_eq!(scrollbar_position(0, 20, 5), 0);
    }

    #[test]
    fn scrollbar_position_clamps_overscroll_to_bottom() {
        assert_eq!(scrollbar_position(999, 20, 5), 19);
    }

    #[test]
    fn scrollbar_position_single_page_stays_at_zero() {
        assert_eq!(scrollbar_position(0, 5, 10), 0);
    }

    #[test]
    fn scrollbar_position_uses_wide_math_for_large_lengths() {
        // With usize math, max_scroll * (content_len - 1) would overflow and
        // panic (debug) or wrap (release). These would fail either way.
        let content_len = usize::MAX;
        let viewport_len = 2;
        let max_scroll = content_len - viewport_len; // usize::MAX - 2

        // Top of the scroll range maps to position 0.
        assert_eq!(scrollbar_position(0, content_len, viewport_len), 0);
        // Bottom of the scroll range maps to the last position:
        // max_scroll * (content_len - 1) / max_scroll == content_len - 1.
        assert_eq!(
            scrollbar_position(max_scroll, content_len, viewport_len),
            usize::MAX - 1
        );
        // Overscroll past max_scroll clamps to the same last position.
        assert_eq!(
            scrollbar_position(usize::MAX, content_len, viewport_len),
            usize::MAX - 1
        );
    }

    #[test]
    fn viewport_scrollbar_state_handles_zero_viewport() {
        // The helper clamps viewport_len to 1, so a zero-height viewport must
        // not panic and must still produce a usable state.
        let state = viewport_scrollbar_state(20, 5, 0);
        assert_eq!(
            state,
            ScrollbarState::new(20)
                .position(5)
                .viewport_content_length(1)
        );
    }

    #[test]
    fn shade_from_base_rank_0_equals_base() {
        let base = Color::Rgb(255, 0, 0);
        assert_eq!(shade_from_base(base, 0), base);
    }

    #[test]
    fn shade_from_base_lightens_monotonically_toward_white() {
        let base = Color::Rgb(0, 0, 0);
        let mut prev_r: u8 = 0;
        for rank in 0..7 {
            let Color::Rgb(r, _, _) = shade_from_base(base, rank) else {
                panic!("expected Rgb")
            };
            assert!(
                r >= prev_r,
                "shade at rank {} should not be darker than rank {}",
                rank,
                rank - 1
            );
            prev_r = r;
        }
    }

    #[test]
    fn shade_from_base_clamps_beyond_palette_length() {
        let base = Color::Rgb(100, 100, 100);
        // Rank beyond FACTORS.len() saturates to the lightest shade.
        assert_eq!(shade_from_base(base, 100), shade_from_base(base, 6));
    }

    #[test]
    fn shade_from_base_passes_through_non_rgb() {
        // Indexed terminal colors can't be lightened channel-wise — return as-is.
        assert_eq!(shade_from_base(Color::Indexed(42), 5), Color::Indexed(42));
    }

    #[test]
    fn unknown_provider_returns_gray_ramp_not_pure_white() {
        // Regression: the old fallback was [(255,255,255)], collapsing all
        // unknown-provider models to pure white. Now each rank is a distinct
        // gray so models stay distinguishable.
        let rank_0 = get_provider_shade("some-new-provider", 0);
        let rank_3 = get_provider_shade("some-new-provider", 3);
        assert_ne!(rank_0, rank_3);
        assert_ne!(rank_0, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn cursor_provider_has_distinct_shades_per_rank() {
        // Regression: CURSOR_SHADES used to be a single-entry palette so all
        // Cursor models collapsed to one color.
        let rank_0 = get_provider_shade("cursor", 0);
        let rank_6 = get_provider_shade("cursor", 6);
        assert_ne!(rank_0, rank_6);
    }

    #[test]
    fn get_provider_shade_saturates_at_palette_end() {
        let last = get_provider_shade("anthropic", 6);
        let past_end = get_provider_shade("anthropic", 99);
        assert_eq!(last, past_end);
    }

    #[test]
    fn fable_is_recognized_as_anthropic() {
        assert_eq!(get_provider_from_model("fable-5"), "anthropic");
        assert_eq!(get_provider_from_model("claude-fable-5"), "anthropic");
        assert_eq!(get_provider_from_model("claude-fable-5[1m]"), "anthropic");
    }

    #[test]
    fn glm_and_kimi_use_their_vendor_color_ramps() {
        assert_eq!(get_provider_from_model("glm-5.2"), "zai");
        assert_eq!(get_provider_from_model("kimi-k2.7-code"), "moonshotai");
        assert!(provider_has_palette("zai"));
        assert!(provider_has_palette("moonshotai"));
        assert_eq!(get_provider_from_model("glmish-1"), "unknown");
        assert_eq!(get_provider_from_model("kimiko-1"), "unknown");
    }

    #[test]
    fn fable_gets_same_base_color_as_opus() {
        // Fable is a flagship Claude model and must render in the Anthropic
        // palette at the same base level as Opus — not the gray UNKNOWN shade.
        let fable = get_model_color("fable-5");
        let opus = get_model_color("claude-opus-4-1");
        assert_eq!(fable, opus);
        assert_eq!(fable, get_model_color("claude-fable-5"));
        // Don't assert default-palette inequality against an unknown model:
        // user color overrides from ~/.tokscale could make the two colors
        // equal at runtime, which would flake this test. Assert provider
        // classification instead, which is independent of config palettes.
        assert_eq!(get_provider_from_model("some-unknown-model"), "unknown");
    }

    #[test]
    fn fable_substring_does_not_misattribute_to_anthropic() {
        // Regression: raw substring matching for "fable" would misclassify
        // unrelated model names. Matching must be on delimited tokens only,
        // consistent with core provider_identity inference.
        assert_eq!(get_provider_from_model("unfabled-model"), "unknown");
        assert_eq!(get_provider_from_model("fableton-1"), "unknown");
        // But genuine fable tokens still resolve to Anthropic.
        assert_eq!(get_provider_from_model("fable-5"), "anthropic");
        assert_eq!(get_provider_from_model("claude-fable-5[1m]"), "anthropic");
    }

    #[test]
    fn provider_display_name_target_cases() {
        // The four cases the user reported as rendering wrong.
        assert_eq!(get_provider_display_name("openai"), "OpenAI");
        assert_eq!(get_provider_display_name("kimi-for-coding"), "Kimi");
        assert_eq!(get_provider_display_name("google-vertex"), "Google Vertex");
        assert_eq!(get_provider_display_name("opencode"), "OpenCode");
    }

    #[test]
    fn provider_display_name_openai_family() {
        // Any openai* id collapses to the brand name.
        assert_eq!(get_provider_display_name("openai"), "OpenAI");
        assert_eq!(get_provider_display_name("openai-codex"), "OpenAI");
        assert_eq!(get_provider_display_name("OpenAI"), "OpenAI");
    }

    #[test]
    fn provider_display_name_kimi_family() {
        assert_eq!(get_provider_display_name("kimi"), "Kimi");
        assert_eq!(get_provider_display_name("kimi-code"), "Kimi");
        assert_eq!(get_provider_display_name("kimi-for-coding"), "Kimi");
    }

    #[test]
    fn provider_display_name_google_vertex_both_spellings() {
        // `canonical_provider` rewrites the hyphen to an underscore, so both
        // spellings must map to the same clean label.
        assert_eq!(get_provider_display_name("google-vertex"), "Google Vertex");
        assert_eq!(get_provider_display_name("google_vertex"), "Google Vertex");
    }

    #[test]
    fn provider_display_name_smart_fallback_multiword() {
        // Unknown multi-word providers get split + title-cased instead of the
        // old naive capitalize-first ("Some-new-provider").
        assert_eq!(
            get_provider_display_name("some-new-provider"),
            "Some New Provider"
        );
        assert_eq!(
            get_provider_display_name("some_new_provider"),
            "Some New Provider"
        );
    }

    #[test]
    fn provider_display_name_known_regressions() {
        assert_eq!(get_provider_display_name("anthropic"), "Anthropic");
        assert_eq!(get_provider_display_name("google"), "Google");
        assert_eq!(get_provider_display_name("xai"), "xAI");
        assert_eq!(get_provider_display_name("deepseek"), "DeepSeek");
        assert_eq!(get_provider_display_name("meta"), "Meta");
        assert_eq!(get_provider_display_name("mistral"), "Mistral");
        assert_eq!(get_provider_display_name("cohere"), "Cohere");
        assert_eq!(get_provider_display_name("cursor"), "Cursor");
        assert_eq!(get_provider_display_name("orcarouter"), "OrcaRouter");
        assert_eq!(
            get_provider_display_name("github-copilot"),
            "GitHub Copilot"
        );
        assert_eq!(get_provider_display_name("copilot"), "GitHub Copilot");
    }

    #[test]
    fn provider_display_name_acronym_words_in_fallback() {
        // Per-word acronym map applies inside the smart fallback.
        assert_eq!(get_provider_display_name("acme-ai"), "Acme AI");
        assert_eq!(get_provider_display_name("foo-api"), "Foo API");
    }

    #[test]
    fn provider_display_name_merged_list_maps_each_segment() {
        // Merged Models rows store providers as a ", "-joined string
        // (aggregate_model_usage_entries). Each segment must be mapped
        // independently — a prefix/contains-family branch on the whole string
        // would otherwise silently drop the rest. Regression guard for that.
        assert_eq!(
            get_provider_display_name("openai, openrouter"),
            "OpenAI, OpenRouter"
        );
        assert_eq!(
            get_provider_display_name("kimi, anthropic"),
            "Kimi, Anthropic"
        );
        // A `copilot` segment must not swallow its siblings via the contains()
        // branch.
        assert_eq!(
            get_provider_display_name("anthropic, copilot"),
            "Anthropic, GitHub Copilot"
        );
        assert_eq!(
            get_provider_display_name("anthropic, openai"),
            "Anthropic, OpenAI"
        );
    }

    #[test]
    fn provider_display_name_empty_is_empty() {
        assert_eq!(get_provider_display_name(""), "");
        // A string of only separators collapses to empty rather than panicking.
        assert_eq!(get_provider_display_name("--_-"), "");
    }

    #[test]
    fn get_provider_shade_fuzzy_matching() {
        assert_eq!(
            get_provider_shade("test-anthropic", 0),
            get_provider_shade("anthropic", 0)
        );
        assert_eq!(
            get_provider_shade("company-google", 0),
            get_provider_shade("google", 0)
        );
        assert_eq!(
            get_provider_shade("openrouter-gemini-prod", 0),
            get_provider_shade("google", 0)
        );
        assert_eq!(
            get_provider_shade("deepseek-api", 0),
            get_provider_shade("deepseek", 0)
        );
        assert_eq!(
            get_provider_shade("meta-llama-endpoint", 0),
            get_provider_shade("meta", 0)
        );
    }

    /// Neither fitter may ever exceed the cell budget it was given, at any width,
    /// including widths that land mid-grapheme, or the table overflows its
    /// column and the row's numbers shift.
    ///
    /// Checked in both ambients. `unicode-width` resolves East-Asian-Ambiguous
    /// characters to one cell by default and to two under `width_cjk`, which is
    /// what a terminal in a CJK locale does, so a fitter whose own markers are
    /// ambiguous keeps its budget on one machine and blows it on another.
    #[test]
    fn label_fitters_never_exceed_their_cell_budget() {
        let mut ambient_independent_elisions = 0usize;
        for label in [
            "tokscale ⑃ worker-1",
            "tokscale-2 ⑃ wf_2429b20d-2d5-1",
            "swebench-matplotlib__matplotlib-25775-dven5vd8/matplotlib",
            "continue-the-native-s-fcebfeb0/worktrees/worker-2",
            "agent-runtime (/Users/junhoyeo/agent-runtime)",
            "//server/share/team/app ⑃ feature",
            "a/b/c/d/e/f/g/h",
            "プロジェクト ⑃ 機能ブランチ",
            "日本語/プロジェクト/機能",
            "a",
            "/",
            "",
            "👩‍👩‍👧‍👦-family ⑃ 🇰🇷-flag-worktree",
            // Ambiguous in its own right: the label already measures differently
            // in the two ambients before the fitter touches it.
            "notes·drafts/α-release ⑃ β",
        ] {
            let ambient_independent = UnicodeWidthStr::width_cjk(label) == display_width(label);
            for width in 0usize..=80 {
                for (name, out) in [
                    ("elide_middle", elide_middle_to_width(label, width)),
                    ("fit_workspace", fit_workspace_label_to_width(label, width)),
                ] {
                    assert!(
                        display_width(&out) <= width,
                        "{name}({label:?}, {width}) rendered {} cells: {out:?}",
                        display_width(&out)
                    );
                    // A label that is already ambient-dependent is beyond the
                    // fitter's control; what it must never do is ADD width that
                    // only appears on a CJK-locale terminal.
                    if !ambient_independent {
                        continue;
                    }
                    assert!(
                        UnicodeWidthStr::width_cjk(out.as_str()) <= width,
                        "{name}({label:?}, {width}) rendered {} cells under width_cjk: {out:?}",
                        UnicodeWidthStr::width_cjk(out.as_str())
                    );
                    if out.contains(MIDDLE_ELLIPSIS) {
                        ambient_independent_elisions += 1;
                    }
                }
            }
        }
        assert!(
            ambient_independent_elisions > 0,
            "the width_cjk check never saw an elision marker"
        );
    }

    /// A label only a little too wide must lose those cells from its longest
    /// segment alone. Spreading the loss over every segment would put an
    /// ellipsis through parts of the path that fit perfectly well.
    #[test]
    fn fit_workspace_label_clips_only_the_segments_that_must_shrink() {
        let label = "continue-the-native-s-fcebfeb0/worktrees/worker-2";
        let out = fit_workspace_label_to_width(label, 44);
        assert!(out.ends_with("/worktrees/worker-2"), "{out:?}");
        assert!(out.starts_with("continue-the"), "{out:?}");
        assert_eq!(display_width(&out), 44, "{out:?}");

        // Nothing to do once the whole label fits.
        assert_eq!(fit_workspace_label_to_width(label, 60), label);
    }

    /// Too many segments for any of them to say anything: the interior collapses
    /// to a marker rather than silently rendering a path that never existed.
    #[test]
    fn fit_workspace_label_marks_a_collapsed_interior() {
        let out = fit_workspace_label_to_width("a/b/c/d/e/f/g/hello-world-long-name", 18);
        assert!(out.contains(&format!("/{MIDDLE_ELLIPSIS}/")), "{out:?}");
        assert!(display_width(&out) <= 18, "{out:?}");

        // The outer separators survive the collapse, so a label whose parts are
        // joined by different separators still reads as what it is.
        let mixed = fit_workspace_label_to_width("org/team/project/repo ⑃ feature", 18);
        assert!(mixed.starts_with("org/"), "{mixed:?}");
        assert!(mixed.ends_with(" ⑃ feature"), "{mixed:?}");
        assert!(mixed.contains(MIDDLE_ELLIPSIS), "{mixed:?}");
    }

    /// `repo ⑃ worktree` is two names the same way `parent/repo` is. Budgeting
    /// them as one segment cut through the worktree name — the only part that
    /// differs between worktrees of one repo — and the rows collapsed together
    /// again at the width the column actually gets.
    #[test]
    fn fit_workspace_label_budgets_across_the_worktree_separator() {
        let labels: Vec<String> = [
            "pirka-runtime-replacement-plan",
            "followup-attachments-plan",
        ]
        .iter()
        .map(|worktree| format!("wrks-sisyphus ⑃ {worktree} (/Users/junhoyeo/{worktree})"))
        .collect();
        let fitted: Vec<String> = labels
            .iter()
            .map(|label| fit_workspace_label_to_width(label, 18))
            .collect();
        assert_ne!(fitted[0], fitted[1], "{fitted:?}");
        for out in &fitted {
            assert!(display_width(out) <= 18, "{out:?}");
            assert!(out.contains(WORKTREE_SEPARATOR), "{out:?}");
        }
    }

    /// The parenthesised key is the only unique thing on a row the filesystem
    /// could not separate. Water-filling it as if it were the name's own path
    /// segments let them outbid it, and once its interior collapsed to `/⋯/` the
    /// qualifier said nothing: two different repos both rendered
    /// `tok⋯-2 (/⋯/tok⋯-2)`, which is worse than the labels they started from.
    #[test]
    fn fit_workspace_label_keeps_a_key_qualifier_distinguishing() {
        let a = "tokscale-2 (/Users/junhoyeo/tokscale-2)";
        let b = "tokscale-giwa-2 (/Users/junhoyeo/tokscale-giwa-2)";
        for width in [18usize, 24, 30, 44] {
            let fitted_a = fit_workspace_label_to_width(a, width);
            let fitted_b = fit_workspace_label_to_width(b, width);
            assert_ne!(fitted_a, fitted_b, "at {width} cells");
            assert!(display_width(&fitted_a) <= width, "{fitted_a:?}");
            assert!(display_width(&fitted_b) <= width, "{fitted_b:?}");
            // The qualifier still reads as one, and still shows both ends of the
            // key rather than a marker where its middle used to be.
            assert!(fitted_a.ends_with("-2)"), "{fitted_a:?}");
            assert!(fitted_a.contains(" (/"), "{fitted_a:?}");
        }
    }

    /// The point of the elision: both ends survive, so the segment that tells two
    /// rows apart is still on screen wherever it sits in the label.
    #[test]
    fn elide_middle_keeps_both_ends_of_a_workspace_label() {
        let long = "tokscale-2 ⑃ wf_2429b20d-2d5-1";
        let out = elide_middle_to_width(long, 18);
        assert_eq!(display_width(&out), 18, "{out:?}");
        assert!(out.starts_with("tokscale"), "{out:?}");
        assert!(out.ends_with("2d5-1"), "{out:?}");
        assert!(out.contains(MIDDLE_ELLIPSIS), "{out:?}");

        // Nothing is elided when it already fits.
        assert_eq!(elide_middle_to_width("tokscale", 18), "tokscale");
    }

    /// Six worktrees of one repo: head-first truncation renders them as six
    /// identical rows at the width the Workspace column actually gets, and the
    /// fitter does not.
    #[test]
    fn fit_workspace_label_separates_labels_a_head_cut_collapses() {
        let labels: Vec<String> = (1..=6).map(|n| format!("tokscale ⑃ worker-{n}")).collect();

        let head: std::collections::HashSet<String> = labels
            .iter()
            .map(|label| truncate_to_width(label, 18))
            .collect();
        assert_eq!(head.len(), 1, "the regression this replaces: {head:?}");

        let fitted: std::collections::HashSet<String> = labels
            .iter()
            .map(|label| fit_workspace_label_to_width(label, 18))
            .collect();
        assert_eq!(fitted.len(), labels.len(), "{fitted:?}");
    }

    #[test]
    fn suffix_to_width_mirrors_prefix_to_width() {
        assert_eq!(suffix_to_width("abcdef", 3), "def");
        assert_eq!(suffix_to_width("abcdef", 0), "");
        assert_eq!(suffix_to_width("abcdef", 99), "abcdef");
        // A full-width grapheme is never split in half.
        assert_eq!(suffix_to_width("ab日", 1), "");
        assert_eq!(suffix_to_width("ab日", 2), "日");
    }

    #[test]
    fn provider_has_palette_flags_only_known_vendors() {
        // Known vendors (and fuzzy substrings of them) have their own ramp.
        assert!(provider_has_palette("anthropic"));
        assert!(provider_has_palette("openai"));
        assert!(provider_has_palette("openrouter-gemini-prod"));
        assert!(provider_has_palette("zai"));
        assert!(provider_has_palette("moonshotai"));
        // Gateway/reseller providers do not — callers must color by the
        // model's own vendor instead of the neutral "unknown" gray.
        assert!(!provider_has_palette("github-copilot"));
        assert!(!provider_has_palette("openrouter"));
        assert!(!provider_has_palette("unknown"));
        assert!(!provider_has_palette(""));
    }
}
