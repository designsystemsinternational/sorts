//! The event loop. Kept thin: all behavior is in `App` and `view`, which are tested.

use super::{Action, App, view};
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use ratatui::crossterm::execute;
use std::io::stdout;

pub fn run(app: &mut App) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    // ratatui's panic hook restores the terminal, but it does not stop the mouse capture.
    let restore_terminal = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture);
        restore_terminal(info);
    }));
    let result = (|| loop {
        terminal.draw(|frame| view::render(app, frame))?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if app.handle_key(key) == Action::Quit {
                    return Ok(());
                }
            }
            Event::Mouse(mouse) => app.handle_mouse(mouse),
            _ => {}
        }
    })();
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
