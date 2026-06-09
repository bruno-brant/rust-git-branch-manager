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

use crate::app::{App, Mode};

pub fn draw(frame: &mut Frame, app: &mut App) {
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

    if let Mode::Confirm(_) = app.mode {
        draw_confirm_modal(frame, app);
    }
}

fn draw_title(frame: &mut Frame, area: Rect, app: &App) {
    let total = app.branches.len();
    let pos = if total == 0 { 0 } else { app.cursor + 1 };
    let title = Paragraph::new(format!(" git-branch-manager   {pos}/{total}"))
        .style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(title, area);
}

fn draw_list(frame: &mut Frame, area: Rect, app: &mut App) {
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
    let items: Vec<ListItem> = (app.offset..end)
        .map(|i| render_row(app, i))
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Branches "));
    frame.render_widget(list, area);
}

fn render_row(app: &App, i: usize) -> ListItem<'static> {
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

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let status = Paragraph::new(app.status.clone())
        .style(Style::default().fg(Color::Magenta));
    frame.render_widget(status, rows[0]);

    let help = Paragraph::new(
        "↑/↓ move  PgUp/PgDn page  Space select  Enter delete  r refresh  q quit",
    )
    .style(Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(help, rows[1]);
}

fn draw_confirm_modal(frame: &mut Frame, app: &App) {
    let Mode::Confirm(pending) = &app.mode else {
        return;
    };

    let mut lines = vec![Line::from(Span::styled(
        "Confirm deletion",
        Style::default().add_modifier(Modifier::BOLD),
    ))];

    if !pending.force.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "These are NOT fully merged — force delete (-D):",
            Style::default().fg(Color::Yellow),
        )));
        for &i in &pending.force {
            lines.push(Line::from(format!("  • {}", app.branches[i].name)));
        }
    }

    if !pending.worktree.is_empty() {
        lines.push(Line::from(""));
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
        .title(" Confirm ")
        .border_style(Style::default().fg(Color::Red));
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(para, area);
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
