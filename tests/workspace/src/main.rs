mod app;
mod input;
mod matrix;
mod ui;

use std::io;
use std::time::Duration;

use app::App;
use crossterm::event::{Event, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// [[docs/001-architecture.md#整体架构]]
fn main() -> io::Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let mut app = App::new();
    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("错误: {e}");
    }

    Ok(())
}

/// [[docs/001-architecture.md#整体架构]]
fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        if app.quit {
            return Ok(());
        }

        if crossterm::event::poll(Duration::from_millis(100))? {
            let event = crossterm::event::read()?;
            if let Event::Key(key) = event
                && key.kind == KeyEventKind::Press
            {
                input::handle_key(app, key);
            }
        }
    }
}
