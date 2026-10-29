//! The event loop. Kept thin: all behavior is in `App` and `view`, which are tested.

use super::{Action, App, view};
use ratatui::crossterm::event::{self, Event, KeyEventKind};

pub fn run(app: &mut App) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = (|| loop {
        terminal.draw(|frame| view::render(app, frame))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && app.handle_key(key) == Action::Quit
        {
            return Ok(());
        }
    })();
    ratatui::restore();
    result
}
