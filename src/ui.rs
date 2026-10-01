//! Rendering. The layout is: a title bar, a scrollable branch list (the viewport
//! that does the paging), and a status/help bar. When a delete needs confirmation,
//! a modal is drawn centered over the list.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Mode, UpdateState, Updating, Working};
use crate::git::GitOperations;

pub fn draw<G: GitOperations + Send + 'static>(frame: &mut Frame, app: &mut App<G>) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(1),    // list
            Constraint::Length(2), // status + help
        ])
        .split(frame.area());

    draw_title(frame, chunks[0], app);
    draw_list(frame, chunks[1], app);
    draw_status(frame, chunks[2], app);

    match &app.mode {
        Mode::Confirm(_) => draw_confirm_modal(frame, app),
        Mode::Working(w) => draw_working_modal(frame, w),
        Mode::ConfirmUpdate(version) => draw_update_prompt(frame, version),
        Mode::Updating(u) => draw_updating_modal(frame, u),
        Mode::Browsing => {}
    }
}

/// Braille spinner. The event loop redraws every 80ms while a batch is running,
/// so this turns at a readable speed without any timing logic of its own.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Shown while a delete batch runs. Removing a worktree can take a while; without
/// this the screen would sit on the confirmation modal and look hung.
fn draw_working_modal(frame: &mut Frame, w: &Working) {
    let glyph = SPINNER[w.frame % SPINNER.len()];
    let action = if w.removing_worktree {
        "removing worktree, then deleting"
    } else {
        "deleting"
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{glyph} "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("Working… ({}/{})", w.done + 1, w.total),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(format!("{action} {}", w.current)),
    ];

    let area = centered_box(70, lines.len() as u16 + 2, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Deleting ")
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_title<G: GitOperations>(frame: &mut Frame, area: Rect, app: &App<G>) {
    let total = app.branches.len();
    let pos = if total == 0 { 0 } else { app.cursor + 1 };
    let mut spans = vec![Span::styled(
        format!(" git-branch-manager   {pos}/{total}"),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if let UpdateState::Available(version) = &app.update {
        spans.push(Span::styled(
            format!("   ● {version} available — press u to update"),
            Style::default().fg(Color::Yellow),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_list<G: GitOperations + Send + 'static>(frame: &mut Frame, area: Rect, app: &mut App<G>) {
    if app.branches.is_empty() {
        let msg = Paragraph::new("No local branches found.")
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(msg, area);
        return;
    }

    // The inner height (minus the border) is our viewport — drives paging.
    let view_height = area.height.saturating_sub(2) as usize;
    app.clamp_offset_to_cursor(view_height);

    let end = (app.offset + view_height).min(app.branches.len());
    let items: Vec<ListItem> = (app.offset..end).map(|i| render_row(app, i)).collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Branches "));
    frame.render_widget(list, area);
}

fn render_row<G: GitOperations>(app: &App<G>, i: usize) -> ListItem<'static> {
    let b = &app.branches[i];
    let is_cursor = i == app.cursor;

    let cursor_marker = if is_cursor { ">" } else { " " };
    let head_marker = if b.is_head { "*" } else { " " };
    let checkbox = if app.selected[i] { "[x]" } else { "[ ]" };

    let mut spans = vec![
        Span::raw(format!("{cursor_marker} {head_marker} {checkbox} ")),
        Span::raw(b.name.clone()),
    ];

    if let Some(path) = &b.worktree {
        spans.push(Span::styled(
            format!("  (wt: {})", path.display()),
            Style::default().fg(Color::Cyan),
        ));
    }
    if !b.is_merged && !b.is_head {
        spans.push(Span::styled(
            "  (unmerged — needs force)",
            Style::default().fg(Color::Yellow),
        ));
    }

    let mut style = Style::default();
    if b.is_head {
        style = style.fg(Color::Green).add_modifier(Modifier::DIM);
    }
    if is_cursor {
        style = style.add_modifier(Modifier::REVERSED);
    }

    ListItem::new(Line::from(spans)).style(style)
}

fn draw_status<G: GitOperations>(frame: &mut Frame, area: Rect, app: &App<G>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let status = Paragraph::new(app.status.clone()).style(Style::default().fg(Color::Magenta));
    frame.render_widget(status, rows[0]);

    let help = Paragraph::new(
        "↑/↓ move  PgUp/PgDn page  Space select  Enter switch  d delete  r refresh  u update  q quit",
    )
    .style(Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(help, rows[1]);
}

fn draw_update_prompt(frame: &mut Frame, version: &str) {
    // Keep every line short enough not to wrap in an 80-column terminal: the
    // question is the last line, and a wrap pushes it out of the box.
    let lines = vec![
        Line::from(format!(
            "{version} is available — you have v{}",
            crate::update::CURRENT
        )),
        Line::from(""),
        Line::from("Downloads from GitHub, checks it against the"),
        Line::from("published SHA256SUMS, then replaces this binary."),
        Line::from(""),
        Line::from(Span::styled(
            "Update now?  [y] yes   [n/Esc] no",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];

    let area = centered_box(70, lines.len() as u16 + 2, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Update available ")
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_updating_modal(frame: &mut Frame, u: &Updating) {
    let glyph = SPINNER[u.frame % SPINNER.len()];
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{glyph} "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("Updating to {}…", u.version),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(u.step.clone()),
    ];

    let area = centered_box(70, lines.len() as u16 + 2, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Updating ")
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_confirm_modal<G: GitOperations>(frame: &mut Frame, app: &App<G>) {
    let Mode::Confirm(pending) = &app.mode else {
        return;
    };

    // The block's title already says "Confirm deletion"; repeating it as the first
    // line just pushed the real content down.
    let mut lines: Vec<Line> = Vec::new();

    if !pending.force.is_empty() {
        lines.push(Line::from(Span::styled(
            "These are NOT fully merged — force delete (-D):",
            Style::default().fg(Color::Yellow),
        )));
        for &i in &pending.force {
            lines.push(Line::from(format!("  • {}", app.branches[i].name)));
        }
    }

    if !pending.worktree.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            "These have a worktree that will be REMOVED, then the branch deleted:",
            Style::default().fg(Color::Cyan),
        )));
        for &i in &pending.worktree {
            let b = &app.branches[i];
            let path = b
                .worktree
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            lines.push(Line::from(format!("  • {}  (wt: {})", b.name, path)));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Proceed?  [y] yes   [n/Esc] cancel",
        Style::default().add_modifier(Modifier::BOLD),
    )));

    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Confirm deletion ")
        .border_style(Style::default().fg(Color::Red));
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

/// A centered box `percent_x` wide and exactly `rows` tall (clamped to what the
/// terminal has). Percentage heights silently clip their last line on a short
/// terminal, which for a modal means losing the very line that says which key
/// answers it.
fn centered_box(percent_x: u16, rows: u16, r: Rect) -> Rect {
    let height = rows.min(r.height);
    let width = (r.width * percent_x / 100).min(r.width);
    Rect {
        x: r.x + (r.width.saturating_sub(width)) / 2,
        y: r.y + (r.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// A rectangle `percent_x` × `percent_y` of `r`, centered.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this guards: a percentage-height modal clips its last line on a
    /// short terminal, and the last line is the one naming the key that answers
    /// the prompt.
    #[test]
    fn a_modal_keeps_all_its_rows_on_a_small_terminal() {
        let screen = Rect::new(0, 0, 80, 24);
        let area = centered_box(70, 8, screen);
        assert_eq!(area.height, 8, "every row must survive");
        assert_eq!(area.width, 56);
        assert_eq!(area.x, 12, "centered horizontally");
        assert_eq!(area.y, 8, "centered vertically");
    }

    #[test]
    fn a_modal_taller_than_the_terminal_is_clamped_not_overflowed() {
        let screen = Rect::new(0, 0, 40, 6);
        let area = centered_box(70, 20, screen);
        assert_eq!(area.height, 6, "cannot draw outside the screen");
        assert_eq!(area.y, 0);
        assert!(area.x + area.width <= screen.width);
    }
}
