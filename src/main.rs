//! Entry point: open the repo, set up the terminal, run the event loop, restore.

mod app;
mod git;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::{App, Mode};

fn main() -> Result<()> {
    let repo = git::open_repo()?;
    let mut app = App::new(repo)?;

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            // crossterm reports Press and Release on Windows; only act on Press.
            if key.kind != KeyEventKind::Press {
                continue;
            }
            handle_key(app, key.code, key.modifiers)?;
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Result<()> {
    // The page step is approximate; the renderer re-clamps the offset to keep the
    // cursor visible regardless of exact terminal height.
    const PAGE: usize = 10;

    match &app.mode {
        Mode::Confirm(_) => match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => app.confirm_delete()?,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.cancel_confirm(),
            _ => {}
        },
        Mode::Browsing => match code {
            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => app.move_up(1),
            KeyCode::Down | KeyCode::Char('j') => app.move_down(1),
            KeyCode::PageUp => app.move_up(PAGE),
            KeyCode::PageDown => app.move_down(PAGE),
            KeyCode::Home => app.move_up(usize::MAX),
            KeyCode::End => app.move_down(usize::MAX),
            KeyCode::Char(' ') => app.toggle_selection(),
            KeyCode::Enter => app.request_delete()?,
            KeyCode::Char('r') => app.refresh()?,
            _ => {}
        },
    }
    Ok(())
}
