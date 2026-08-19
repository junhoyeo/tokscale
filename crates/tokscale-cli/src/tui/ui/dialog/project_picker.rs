use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crossterm::event::KeyCode;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::themes::Theme;

use super::{DialogContent, DialogResult};

/// A project row: the stable key usage is grouped on, plus the short name
/// shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectOption {
    /// The `workspace_key` on the underlying messages. Selection is stored
    /// against this, never against the label: two checkouts of the same repo
    /// share a label but are different projects.
    pub key: String,
    pub label: String,
}

/// TUI dialog for narrowing reports to one or more projects.
///
/// Selection semantics differ deliberately from [`super::ClientPickerDialog`].
/// The client picker refuses to empty its set, because scanning no clients is
/// never meaningful. Here an empty set means *every* project: the list is
/// discovered from scan results and routinely runs to a couple of hundred
/// entries, so requiring the user to deselect all but one would make the
/// common case the most laborious one. Empty is therefore the unfiltered
/// default, and selecting rows narrows from there.
pub struct ProjectPickerDialog {
    /// Every project discovered in the loaded data, ordered by descending
    /// usage so the projects worth filtering to are reachable without typing.
    projects: Vec<ProjectOption>,
    selected_keys: Rc<RefCell<HashSet<String>>>,
    needs_reload: Rc<RefCell<bool>>,
    selected: usize,
    filter: String,
    /// Indices into `projects` matching the current type-to-filter substring.
    /// `selected` indexes into this vec, not into `projects`.
    filtered_indices: Vec<usize>,
}

impl ProjectPickerDialog {
    pub fn new(
        projects: Vec<ProjectOption>,
        selected_keys: Rc<RefCell<HashSet<String>>>,
        needs_reload: Rc<RefCell<bool>>,
    ) -> Self {
        let filtered_indices: Vec<usize> = (0..projects.len()).collect();
        Self {
            projects,
            selected_keys,
            needs_reload,
            selected: 0,
            filter: String::new(),
            filtered_indices,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered_indices.is_empty() {
            self.selected = 0;
            return;
        }
        let max = self.filtered_indices.len() as isize;
        let mut next = self.selected as isize + delta;
        if next < 0 {
            next = max - 1;
        } else if next >= max {
            next = 0;
        }
        self.selected = next as usize;
    }

    fn toggle_selected(&mut self) {
        let Some(&idx) = self.filtered_indices.get(self.selected) else {
            return;
        };
        let key = self.projects[idx].key.clone();
        let mut selected = self.selected_keys.borrow_mut();
        if !selected.remove(&key) {
            selected.insert(key);
        }
        *self.needs_reload.borrow_mut() = true;
    }

    /// Drop every selection, returning the report to all projects.
    fn clear_selection(&mut self) {
        let mut selected = self.selected_keys.borrow_mut();
        if selected.is_empty() {
            return;
        }
        selected.clear();
        *self.needs_reload.borrow_mut() = true;
    }

    /// Select every project the current filter matches, so a substring like
    /// `worktrees` can be scoped in one keystroke instead of many.
    fn select_all_filtered(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let mut selected = self.selected_keys.borrow_mut();
        for &idx in &self.filtered_indices {
            selected.insert(self.projects[idx].key.clone());
        }
        *self.needs_reload.borrow_mut() = true;
    }

    fn rebuild_filter(&mut self) {
        let needle = self.filter.to_lowercase();
        if needle.is_empty() {
            self.filtered_indices = (0..self.projects.len()).collect();
        } else {
            self.filtered_indices = self
                .projects
                .iter()
                .enumerate()
                .filter(|(_, project)| {
                    project.label.to_lowercase().contains(&needle)
                        || project.key.to_lowercase().contains(&needle)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.selected >= self.filtered_indices.len() {
            self.selected = 0;
        }
    }
}

impl DialogContent for ProjectPickerDialog {
    fn desired_size(&self, viewport: Rect) -> (u16, u16) {
        // Wider than the client picker: project labels are directory names
        // and truncate badly at 50 columns.
        let width = 64u16.min(viewport.width.saturating_sub(4));
        let height = 22u16.min(viewport.height.saturating_sub(4));
        (width, height)
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let selected_count = self.selected_keys.borrow().len();
        let title = if selected_count == 0 {
            " Projects (all) ".to_string()
        } else {
            format!(" Projects ({selected_count} selected) ")
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accent));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(inner);

        let filter_text = if self.filter.is_empty() {
            Span::styled("Type to filter...", Style::default().fg(theme.muted))
        } else {
            Span::styled(&self.filter, Style::default().fg(theme.foreground))
        };
        let filter_line = Paragraph::new(Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(theme.accent)),
            filter_text,
        ]));
        frame.render_widget(filter_line, rows[0]);

        let divider = Paragraph::new("-".repeat(rows[1].width as usize))
            .style(Style::default().fg(theme.border));
        frame.render_widget(divider, rows[1]);

        let list_area = rows[2];
        let visible_height = list_area.height as usize;
        let scroll = if self.selected >= visible_height && visible_height > 0 {
            self.selected.saturating_sub(visible_height - 1)
        } else {
            0
        };

        let mut items: Vec<ListItem> = Vec::new();
        for (flat_idx, &idx) in self.filtered_indices.iter().enumerate() {
            if flat_idx < scroll {
                continue;
            }
            if items.len() >= visible_height {
                break;
            }

            let project = &self.projects[idx];
            let is_selected = flat_idx == self.selected;
            let is_enabled = self.selected_keys.borrow().contains(&project.key);

            let checkbox = if is_enabled { "[●]" } else { "[ ]" };
            let usable = list_area.width.saturating_sub(4) as usize;
            let left = format!("{} {}", checkbox, project.label);
            let left: String = left.chars().take(usable).collect();
            let padding = usable.saturating_sub(left.chars().count());

            let base_style = if is_selected {
                Style::default()
                    .bg(theme.accent)
                    .fg(theme.background)
                    .add_modifier(Modifier::BOLD)
            } else if is_enabled {
                Style::default().fg(theme.foreground)
            } else {
                Style::default().fg(theme.muted)
            };

            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", left), base_style),
                Span::styled(" ".repeat(padding), base_style),
            ])));
        }

        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "  No results",
                Style::default().fg(theme.muted),
            ))));
        }

        frame.render_widget(List::new(items), list_area);

        let hint = Paragraph::new(
            "↑↓ navigate • Enter toggle • Ctrl+A all shown • Ctrl+X clear • Esc close",
        )
        .alignment(Alignment::Center)
        .style(Style::default().fg(theme.muted));
        frame.render_widget(hint, rows[3]);
    }

    fn handle_key(&mut self, key: KeyCode) -> DialogResult {
        match key {
            KeyCode::Esc => DialogResult::Close,
            KeyCode::Up => {
                self.move_selection(-1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.move_selection(1);
                DialogResult::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.toggle_selected();
                DialogResult::None
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.rebuild_filter();
                DialogResult::None
            }
            // Non-character keys, because `DialogContent::handle_key` only
            // receives a `KeyCode` with no modifier state: a Ctrl+A binding
            // would arrive indistinguishable from a plain `a` and be swallowed
            // as filter input.
            KeyCode::Tab => {
                self.select_all_filtered();
                DialogResult::None
            }
            KeyCode::Delete => {
                self.clear_selection();
                DialogResult::None
            }
            // Unlike the client picker there are no per-row hotkeys: project
            // names are user data, so every printable character is filter
            // input and nothing is reserved.
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.rebuild_filter();
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(projects: &[(&str, &str)]) -> (ProjectPickerDialog, Rc<RefCell<HashSet<String>>>) {
        let selected = Rc::new(RefCell::new(HashSet::new()));
        let reload = Rc::new(RefCell::new(false));
        let options = projects
            .iter()
            .map(|(key, label)| ProjectOption {
                key: (*key).to_string(),
                label: (*label).to_string(),
            })
            .collect();
        (
            ProjectPickerDialog::new(options, Rc::clone(&selected), reload),
            selected,
        )
    }

    #[test]
    fn empty_selection_means_every_project() {
        let (dialog, selected) = dialog(&[("/a", "a"), ("/b", "b")]);

        assert!(selected.borrow().is_empty());
        assert_eq!(dialog.filtered_indices.len(), 2);
    }

    #[test]
    fn toggling_twice_returns_to_unfiltered() {
        let (mut dialog, selected) = dialog(&[("/a", "a"), ("/b", "b")]);

        dialog.toggle_selected();
        assert_eq!(selected.borrow().len(), 1);
        assert!(selected.borrow().contains("/a"));

        dialog.toggle_selected();
        assert!(selected.borrow().is_empty());
    }

    #[test]
    fn selection_is_keyed_on_the_workspace_key_not_the_label() {
        // Two checkouts of one repo share a label; selecting one must not
        // silently select the other.
        let (mut dialog, selected) = dialog(&[("/repo", "app"), ("/worktrees/repo", "app")]);

        dialog.toggle_selected();

        assert_eq!(selected.borrow().len(), 1);
        assert!(selected.borrow().contains("/repo"));
        assert!(!selected.borrow().contains("/worktrees/repo"));
    }

    #[test]
    fn filter_matches_label_or_key() {
        let (mut dialog, _) = dialog(&[("/srv/api", "api"), ("/srv/web", "web")]);

        dialog.filter = "web".to_string();
        dialog.rebuild_filter();
        assert_eq!(dialog.filtered_indices, vec![1]);

        dialog.filter = "/srv".to_string();
        dialog.rebuild_filter();
        assert_eq!(dialog.filtered_indices, vec![0, 1]);
    }

    #[test]
    fn select_all_filtered_scopes_to_the_visible_subset() {
        let (mut dialog, selected) = dialog(&[
            ("/srv/api", "api"),
            ("/srv/web", "web"),
            ("/other", "other"),
        ]);

        dialog.filter = "/srv".to_string();
        dialog.rebuild_filter();
        dialog.select_all_filtered();

        assert_eq!(selected.borrow().len(), 2);
        assert!(!selected.borrow().contains("/other"));
    }

    #[test]
    fn clearing_returns_to_all_projects() {
        let (mut dialog, selected) = dialog(&[("/a", "a"), ("/b", "b")]);

        dialog.toggle_selected();
        assert!(!selected.borrow().is_empty());

        dialog.clear_selection();
        assert!(selected.borrow().is_empty());
    }

    #[test]
    fn tab_selects_every_filtered_row_and_delete_clears() {
        // Guards the binding, not just the helper: these were advertised in
        // the hint line while unreachable through `handle_key`.
        let (mut dialog, selected) = dialog(&[
            ("/srv/api", "api"),
            ("/srv/web", "web"),
            ("/other", "other"),
        ]);

        dialog.filter = "/srv".to_string();
        dialog.rebuild_filter();
        dialog.handle_key(KeyCode::Tab);
        assert_eq!(selected.borrow().len(), 2);

        dialog.handle_key(KeyCode::Delete);
        assert!(selected.borrow().is_empty());
    }

    #[test]
    fn moving_selection_wraps_in_both_directions() {
        let (mut dialog, _) = dialog(&[("/a", "a"), ("/b", "b")]);

        dialog.move_selection(-1);
        assert_eq!(dialog.selected, 1);

        dialog.move_selection(1);
        assert_eq!(dialog.selected, 0);
    }
}
