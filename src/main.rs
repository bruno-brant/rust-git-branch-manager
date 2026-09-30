use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use git_branch_manager::app::{self, App};
use git_branch_manager::git::{self, GitOperations};
use git_branch_manager::ui;

fn main() -> Result<()> {
    let repo = git::open_repo()?;
    let mut app = App::new(git::RealGit::new(repo))?;

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result?;

    // Switching to a branch that lives in a linked worktree means changing
    // directory, which a child process can't do for its parent shell. Print the
    // path on stdout — the only thing this program ever writes there — so a
    // wrapper can act on it:  cd "$(git-branch-manager)"
    if let Some(path) = app.switch_to_worktree {
        eprintln!(
            "branch is checked out in a worktree; cd there with: cd \"$(git-branch-manager)\""
        );
        println!("{}", path.display());
    }
    Ok(())
}

/// The UI is drawn on **stderr**, not stdout: stdout is reserved for the one
/// machine-readable line we may print on exit (the worktree path), so that
/// `cd "$(git-branch-manager)"` captures the path and nothing else.
fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stderr>>> {
    enable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stderr))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stderr>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run<G: GitOperations + Send + 'static>(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    app: &mut App<G>,
) -> Result<()> {
    loop {
        // Picks up worker-thread progress and advances the spinner; a no-op when
        // nothing is running.
        app.tick()?;
        terminal.draw(|f| ui::draw(f, app))?;

        if !event::poll(app.poll_interval())? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            // crossterm reports Press and Release on Windows; only act on Press.
            if key.kind != KeyEventKind::Press {
                continue;
            }
            app::handle_key(app, key.code, key.modifiers)?;
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
