//! Test-only assertions that a table header fits the cell the layout gives it,
//! in **every** `TuiLanguage`.
//!
//! Every wide table in the TUI solves its column widths against English
//! labels, and #1367 then translated those labels into five languages. Nothing
//! re-checked the budgets: Korean `ColMessages` (`메시지`, 6 display cells)
//! landed in the Sessions `Msgs` column's 5-cell budget and ratatui clipped it
//! to `메시` — a truncated word with no ellipsis and no marker, which is the
//! exact failure mode `sessions.rs` exists to prevent. English was unaffected
//! (`Msgs` is 4 cells) and every table test pinned `TuiLanguage::En`, so no
//! test could see it.
//!
//! Two shapes of assertion live here, because they fail for different reasons:
//!
//! * [`assert_header_fits`] measures a label against the *declared* budget. It
//!   is arithmetic, so it names the column, the language and the overflow in
//!   cells — usable by the tables that expose their budget (`natural()`).
//! * [`assert_headers_render_in_full`] reads a *rendered* header row and
//!   asserts every label survived. It needs no budget literal at all, so it
//!   cannot drift from the `Constraint`s it is checking — which is what the
//!   tables that inline their widths need.

use ratatui::layout::Constraint;
use unicode_width::UnicodeWidthStr;

use crate::tui::i18n::TuiLanguage;

/// Cells a sort arrow costs: one space plus one `▴`/`▾`, both narrow.
pub(crate) const SORT_INDICATOR_WIDTH: usize = 2;

/// Assert one header label fits `budget`, counting the sort arrow when the
/// column is sortable — the arrow shares the header cell, so a column that
/// fits its label and not its arrow still renders a clipped header.
pub(crate) fn assert_header_fits(
    table: &str,
    column: &str,
    lang: TuiLanguage,
    label: &str,
    budget: u16,
    sortable: bool,
) {
    let indicator = if sortable { SORT_INDICATOR_WIDTH } else { 0 };
    let needed = UnicodeWidthStr::width(label) + indicator;
    assert!(
        needed <= budget as usize,
        "{table}/{column} header {label:?} needs {needed} cells in {} \
         (label {} + indicator {indicator}) but the layout budgets {budget}; \
         ratatui clips it with no ellipsis",
        lang.code(),
        UnicodeWidthStr::width(label),
    );
}

/// Undo ratatui's wide-grapheme cell padding so a rendered row can be compared
/// against the strings that were written into it.
///
/// A wide grapheme occupies two terminal cells; ratatui puts the grapheme in
/// the first and `Cell::reset()`s the second, whose default symbol is a single
/// space. A row read back cell-by-cell therefore reads `세 션`, not `세션`, and
/// a plain `contains("세션")` misses a label that rendered perfectly. Dropping
/// exactly one following space per wide grapheme is the exact inverse of that
/// write, so a real space after a wide grapheme survives — it sits in the cell
/// after the reset one.
pub(crate) fn strip_wide_continuation_cells(row: &str) -> String {
    let mut out = String::with_capacity(row.len());
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if UnicodeWidthStr::width(c.to_string().as_str()) == 2 && chars.peek() == Some(&' ') {
            chars.next();
        }
    }
    out
}

/// Assert every label of a rendered header row survived the layout.
///
/// Substring pairs (`Cost` inside `Cost/1M`, `비용` inside `비용/1M`) are handled
/// by counting occurrences rather than testing presence: a label is expected as
/// many times as the label set contains it, so clipping the longer label drops
/// the longer label's own count *and* clipping the shorter one drops a count
/// the longer one cannot supply.
pub(crate) fn assert_headers_render_in_full(
    table: &str,
    lang: TuiLanguage,
    header_line: &str,
    labels: &[&str],
) {
    let compacted = strip_wide_continuation_cells(header_line);
    let header_line = compacted.as_str();
    for label in labels {
        let expected = labels.iter().filter(|other| other.contains(*label)).count();
        let found = header_line.matches(*label).count();
        assert!(
            found >= expected,
            "{table} header lost {label:?} in {}: expected {expected} \
             occurrence(s), found {found}. Rendered header:\n{header_line}",
            lang.code(),
        );
    }
}

/// Assert every fixed-width header label fits the `Constraint::Length` its
/// table declares, sort arrow included, and that a layout which *fits* the
/// terminal renders every label in full.
///
/// Two claims because they fail for different reasons and only together cover
/// the tab:
///
/// * The budget claim is arithmetic over `Constraint::Length`, so it holds at
///   every width and names the overflow in cells. It is what the Korean
///   `메시지`-in-a-5-cell-column regression trips.
/// * The rendered claim needs a width where the requested layout fits, because
///   below that ratatui shrinks *every* column proportionally and clips English
///   headers too (`Cache R` renders as `Cache` at 80 columns on the Daily tab).
///   That is the pre-existing #964-class over-ask those tabs never solved, not a
///   localization bug, so asserting survival there would fail on `en` and pin a
///   defect rather than a property.
pub(crate) fn assert_header_layout_fits(
    table: &str,
    lang: TuiLanguage,
    labels: &[&str],
    constraints: &[Constraint],
    sortable: &[bool],
) {
    assert_eq!(
        labels.len(),
        constraints.len(),
        "{table}: {} labels for {} constraints; the header row and the width \
         row must be the same length or the table is mis-drawn",
        labels.len(),
        constraints.len(),
    );
    assert_eq!(
        labels.len(),
        sortable.len(),
        "{table}: {} labels for {} sortable flags",
        labels.len(),
        sortable.len(),
    );

    for ((label, constraint), sortable) in labels.iter().zip(constraints).zip(sortable) {
        // `Min` and `Percentage` columns have no declared budget to check:
        // what they get depends on the terminal, which is the rendered
        // assertion's job.
        if let Constraint::Length(budget) = constraint {
            assert_header_fits(table, label, lang, label, *budget, *sortable);
        }
    }
}

/// Cells a column set of `Constraint::Length`s occupies, plus `spacing` between
/// each pair — the width at and above which ratatui hands every column exactly
/// what it asked for and no header can be clipped by shrinking.
///
/// `None` when the set contains a `Min` or `Percentage` column, which has no
/// fixed request to sum.
pub(crate) fn fixed_layout_width(constraints: &[Constraint], spacing: u16) -> Option<u16> {
    let mut total = 0u16;
    for constraint in constraints {
        match constraint {
            Constraint::Length(cells) => total = total.saturating_add(*cells),
            _ => return None,
        }
    }
    Some(total + spacing * constraints.len().saturating_sub(1) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_that_fits_its_budget_passes() {
        assert_header_fits("t", "c", TuiLanguage::En, "Msgs", 5, false);
        // `건수` is two wide graphemes, so 4 cells rather than 2 chars.
        assert_header_fits("t", "c", TuiLanguage::Ko, "건수", 5, false);
    }

    #[test]
    #[should_panic(expected = "needs 6 cells in ko")]
    fn a_wide_label_over_its_budget_panics() {
        assert_header_fits("t", "c", TuiLanguage::Ko, "메시지", 5, false);
    }

    #[test]
    #[should_panic(expected = "indicator 2")]
    fn a_label_that_fits_without_its_sort_arrow_still_panics() {
        assert_header_fits("t", "c", TuiLanguage::En, "Total", 5, true);
    }

    #[test]
    fn a_full_header_row_passes() {
        assert_headers_render_in_full(
            "t",
            TuiLanguage::En,
            "Date   Msgs  Total ▾    Cost      Cost/1M",
            &["Date", "Msgs", "Total", "Cost", "Cost/1M"],
        );
    }

    #[test]
    #[should_panic(expected = "lost \"메시지\"")]
    fn a_clipped_wide_label_fails_the_rendered_check() {
        assert_headers_render_in_full(
            "t",
            TuiLanguage::Ko,
            "날짜   메시  합계 ▾",
            &["날짜", "메시지", "합계"],
        );
    }

    /// The occurrence count, not mere presence, is what catches this: `비용`
    /// survives inside `비용/1M` even after its own column was clipped away.
    #[test]
    #[should_panic(expected = "expected 2 occurrence(s), found 1")]
    fn a_clipped_label_that_survives_inside_a_longer_one_fails() {
        assert_headers_render_in_full(
            "t",
            TuiLanguage::Ko,
            "날짜   비  비용/1M",
            &["날짜", "비용", "비용/1M"],
        );
    }

    /// A `Length` column is checked against its declared budget; `Min` and
    /// `Percentage` columns have none and are skipped here.
    #[test]
    fn a_length_column_is_checked_and_a_flexible_one_is_skipped() {
        assert_header_layout_fits(
            "t",
            TuiLanguage::Ko,
            &["세션", "건수"],
            &[Constraint::Min(4), Constraint::Length(5)],
            &[false, false],
        );
    }

    #[test]
    #[should_panic(expected = "needs 6 cells in ko")]
    fn a_length_column_over_its_declared_budget_panics() {
        assert_header_layout_fits(
            "t",
            TuiLanguage::Ko,
            &["날짜", "메시지"],
            &[Constraint::Length(12), Constraint::Length(5)],
            &[false, false],
        );
    }

    /// The width at and above which no header can be clipped by shrinking:
    /// every request granted, plus one separator between each pair.
    #[test]
    fn a_fixed_layout_sums_its_requests_and_its_separators() {
        assert_eq!(
            fixed_layout_width(&[Constraint::Length(4), Constraint::Length(6)], 1),
            Some(11)
        );
        assert_eq!(
            fixed_layout_width(&[Constraint::Length(4), Constraint::Min(6)], 1),
            None
        );
    }

    /// A row read back out of a `TestBackend` buffer has a padding space after
    /// every wide grapheme, so an intact label has to survive that round trip.
    #[test]
    fn a_rendered_row_with_wide_cell_padding_still_matches_its_labels() {
        // `세션  비용` written into a buffer: one reset cell after each wide
        // grapheme, and the two real spaces between the labels survive.
        assert_eq!(strip_wide_continuation_cells("세 션   비 용"), "세션  비용");
        // A real space inside a label is kept too: it lives in the cell after
        // the reset continuation cell.
        assert_eq!(strip_wide_continuation_cells("캐 시  읽 기"), "캐시 읽기");
        assert_headers_render_in_full(
            "t",
            TuiLanguage::Ko,
            "|세 션                          비 용  ▾             |",
            &["세션", "비용"],
        );
    }
}
