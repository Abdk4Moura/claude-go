use std::io::stdout;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self as cterm_event, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod app;
mod input;
mod style;
mod ui;

pub use app::App;

/// Run the TUI. Returns when the user quits (q, Ctrl-C, or Esc on
/// the provider screen).
pub fn run(mut app: App) -> Result<()> {
    enable_raw_mode().context("enable raw mode")?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).context("init terminal")?;

    let res = event_loop(&mut terminal, &mut app);

    // Always restore the terminal -- even on error.
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    res
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    let tick = Duration::from_millis(150);
    loop {
        terminal.draw(|f| ui::render(f, app))?;
        if cterm_event::poll(tick).context("event poll")? {
            if let cterm_event::Event::Key(key) = cterm_event::read().context("event read")? {
                let key_event: KeyEvent = key;
                if let Some(action) = input::map_event(app, cterm_event::Event::Key(key_event)) {
                    if !input::apply(app, action) {
                        break;
                    }
                }
            }
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}
