//! Where things go on screen. Pure geometry, shared by `ui` (to draw) and
//! `app` (to keep the cursor in view and size pages).

use ratatui::layout::{Constraint, Layout as Split, Margin, Rect};

use crate::app::{Pane, State};
use crate::config::Config;

/// Provisional minimum size. The real value is an open question in the
/// requirements. Below it, one pane is shown at a time (R20.5).
pub const MIN_SIZE: (u16, u16) = (60, 15);

/// Width below which the context strip collapses to a single line (R10.7).
const NARROW_WIDTH: u16 = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Areas {
    /// Set when the terminal is below the minimum size.
    pub message: Option<Rect>,
    pub editor: Option<Rect>,
    pub context: Option<Rect>,
    pub results: Option<Rect>,
    pub hint: Rect,
    pub status: Rect,
}

pub fn compute(area: Rect, state: &State, config: &Config) -> Areas {
    let hint_height = u16::from(config.ui.show_hint_bar);
    let [body, hint, status] = Split::vertical([
        Constraint::Min(1),
        Constraint::Length(hint_height),
        Constraint::Length(1),
    ])
    .areas(area);

    let mut areas = Areas {
        message: None,
        editor: None,
        context: None,
        results: None,
        hint,
        status,
    };

    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        let [message, pane] =
            Split::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
        areas.message = Some(message);
        match state.focus {
            Pane::Editor => areas.editor = Some(pane),
            Pane::Context => areas.context = Some(pane),
            Pane::Results => areas.results = Some(pane),
        }
        return areas;
    }

    let context_height = match (state.show_context, body.width < NARROW_WIDTH) {
        (false, _) => 0,
        (true, true) => 3,
        (true, false) => 7,
    };
    let results = if state.show_results {
        Constraint::Percentage(40)
    } else {
        Constraint::Length(0)
    };
    let [editor, context, results_area] = Split::vertical([
        Constraint::Min(3),
        Constraint::Length(context_height),
        results,
    ])
    .areas(body);
    areas.editor = Some(editor);
    areas.context = state.show_context.then_some(context);
    areas.results = state.show_results.then_some(results_area);
    areas
}

/// Inside an editor pane: the line number gutter and the text area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorArea {
    pub gutter_width: u16,
    pub text: Rect,
}

pub fn editor_area(pane: Rect, line_count: usize) -> EditorArea {
    let inner = pane.inner(Margin::new(1, 1));
    let digits = line_count.max(1).to_string().len() as u16;
    let gutter_width = (digits + 1).min(inner.width);
    EditorArea {
        gutter_width,
        text: Rect {
            x: inner.x + gutter_width,
            y: inner.y,
            width: inner.width - gutter_width,
            height: inner.height,
        },
    }
}

/// Where the parts of the results pane go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultsBody {
    pub summary: Rect,
    pub tabs: Option<Rect>,
    pub header: Rect,
    pub rows: Rect,
}

/// Splits the inside of the results pane: a summary line, an optional tab
/// line, the column header, then the rows.
pub fn results_body(pane: Rect, with_tabs: bool) -> ResultsBody {
    let inner = pane.inner(Margin::new(1, 1));
    let row = |offset: u16| Rect {
        y: inner.y.saturating_add(offset).min(inner.bottom()),
        height: u16::from(offset < inner.height),
        ..inner
    };
    let tab_height = u16::from(with_tabs);
    let rows_top = 2 + tab_height;
    ResultsBody {
        summary: row(0),
        tabs: with_tabs.then(|| row(1)),
        header: row(1 + tab_height),
        rows: Rect {
            y: inner.y.saturating_add(rows_top).min(inner.bottom()),
            height: inner.height.saturating_sub(rows_top),
            ..inner
        },
    }
}

/// A box of `width` x `height` centered in `area`, shrunk to fit.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// The help popup, centered in `area`.
pub fn help_popup(area: Rect, content_lines: usize) -> Rect {
    let width = 74.min(area.width);
    let height = (content_lines as u16)
        .saturating_add(2)
        // Leave the hint bar and status bar visible.
        .min(area.height.saturating_sub(4).max(3))
        .min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_grows_with_the_line_count() {
        let pane = Rect::new(0, 0, 40, 10);
        assert_eq!(editor_area(pane, 9).gutter_width, 2);
        assert_eq!(editor_area(pane, 10).gutter_width, 3);
        assert_eq!(editor_area(pane, 1000).gutter_width, 5);
        let a = editor_area(pane, 9);
        assert_eq!((a.text.x, a.text.width, a.text.height), (3, 36, 8));
    }

    #[test]
    fn tiny_pane_does_not_underflow() {
        let a = editor_area(Rect::new(0, 0, 2, 2), 100);
        assert_eq!((a.text.width, a.text.height), (0, 0));
    }
}
