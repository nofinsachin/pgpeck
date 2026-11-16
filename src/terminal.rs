//! Terminal setup and teardown, including bracketed paste (R20.10).

use std::io;

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use ratatui::DefaultTerminal;

/// Enters the alternate screen and raw mode. Also installs a panic hook that
/// leaves the terminal usable.
pub fn enter() -> io::Result<DefaultTerminal> {
    // ratatui installs a panic hook that restores the screen and raw mode.
    let terminal = ratatui::try_init()?;
    if let Err(e) = execute!(io::stdout(), EnableBracketedPaste) {
        ratatui::restore();
        return Err(e);
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        previous(info);
    }));
    Ok(terminal)
}

pub fn leave() {
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
}
