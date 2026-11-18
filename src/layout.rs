//! Where things go on screen. Pure geometry, shared by `ui` (to draw) and
//! `app` (to keep the cursor in view and size pages).

use ratatui::layout::{Constraint, Layout as Split, Margin, Rect};

use crate::app::{Pane, State};
use crate::config::Config;

/// Minimum size for the full layout. Below it, one pane is shown at a time
/// with a message (R20.5).
pub const MIN_SIZE: (u16, u16) = (80, 24);

/// The right column holds the context and the trends, top to bottom. It takes this share of the width, within these limits.
const SIDE_SHARE: u16 = 34;
const SIDE_MIN_WIDTH: u16 = 36;
const SIDE_MAX_WIDTH: u16 = 50;

/// Heights, borders included. The context pane holds the welcome text before
/// you connect, which is three lines. The trends pane shows everything in 13
/// rows (five numbers, a heading and five tables). With fewer it shows the
/// numbers first and cuts the tables, and below 9 rows it is left out.
const CONTEXT_HEIGHT: u16 = 5;
const TRENDS_MIN_HEIGHT: u16 = 9;
const TRENDS_FULL_HEIGHT: u16 = 13;
/// The peck command line sits under the trends, level with the editor. It is
/// never shorter than this, and never taller than `PECK_MAX_HEIGHT`.
const PECK_MIN_HEIGHT: u16 = 6;
const PECK_MAX_HEIGHT: u16 = 10;
/// The logo above the context pane, shown when the column has room for it
/// on top of everything else.
pub const LOGO_HEIGHT: u16 = 7;
/// The session strip above the results: one line of text and the borders.
/// On a wide terminal the key hints share that line, so there is no hint bar
/// under the panes.
const SESSION_HEIGHT: u16 = 3;
/// Width of the left column from which the key hints move onto that line.
const STRIP_HINTS_MIN_WIDTH: u16 = 100;

/// The editor is a strip at the bottom of the left column: this share of its
/// height, and never less than `EDITOR_MIN_HEIGHT` (borders included).
const EDITOR_SHARE: u16 = 28;
const EDITOR_MIN_HEIGHT: u16 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Areas {
    /// Set when the terminal is below the minimum size.
    pub message: Option<Rect>,
    pub editor: Option<Rect>,
    pub context: Option<Rect>,
    pub session: Option<Rect>,
    /// The logo, above the context. Left out when the right column is short.
    pub logo: Option<Rect>,
    /// Largest tables. Left out when the right column is too short.
    pub trends: Option<Rect>,
    /// The peck command line, level with the editor.
    pub peck: Option<Rect>,
    pub results: Option<Rect>,
    pub hint: Rect,
    pub status: Rect,
}

pub fn compute(area: Rect, state: &State, config: &Config) -> Areas {
    let tiny = area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1;
    // With room for it, the strip carries the key hints and the bottom hint
    // bar goes away.
    let strip = !tiny && state.show_context && area.height > SESSION_HEIGHT + 6;
    // The hints join the session line only when the left column is wide enough
    // for both. Otherwise the hint bar stays at the bottom.
    let side_width = (area.width * SIDE_SHARE / 100).clamp(SIDE_MIN_WIDTH, SIDE_MAX_WIDTH);
    let hints_in_strip = strip && area.width.saturating_sub(side_width) >= STRIP_HINTS_MIN_WIDTH;
    let hint_height = u16::from(config.ui.show_hint_bar && !hints_in_strip);
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
        session: None,
        logo: None,
        trends: None,
        peck: None,
        results: None,
        hint,
        status,
    };

    if tiny {
        let [message, pane] =
            Split::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
        areas.message = Some(message);
        match state.focus {
            Pane::Editor => areas.editor = Some(pane),
            Pane::Context => areas.context = Some(pane),
            Pane::Session => areas.session = Some(pane),
            Pane::Results => areas.results = Some(pane),
            Pane::Peck => areas.peck = Some(pane),
        }
        return areas;
    }

    // Left: what came back above, what you write below. Right: where you are.
    let (left, side) = if state.show_context {
        let side_width = (body.width * SIDE_SHARE / 100).clamp(SIDE_MIN_WIDTH, SIDE_MAX_WIDTH);
        let [left, side] =
            Split::horizontal([Constraint::Min(1), Constraint::Length(side_width)]).areas(body);
        (left, Some(side))
    } else {
        (body, None)
    };

    // The session strip heads the left column.
    let left = if strip {
        let [session, rest] =
            Split::vertical([Constraint::Length(SESSION_HEIGHT), Constraint::Min(1)]).areas(left);
        areas.session = Some(session);
        rest
    } else {
        left
    };

    let editor_height = (left.height * EDITOR_SHARE / 100).max(EDITOR_MIN_HEIGHT);
    if state.show_results {
        let [results, editor] = Split::vertical([
            Constraint::Min(3),
            Constraint::Length(editor_height.min(left.height.saturating_sub(3))),
        ])
        .areas(left);
        areas.results = Some(results);
        areas.editor = Some(editor);
    } else {
        areas.editor = Some(left);
    }

    if let Some(side) = side {
        // The peck line takes the bottom of the column, level with the editor.
        let editor_rows = areas.editor.map_or(PECK_MIN_HEIGHT, |e| e.height);
        let peck_height = editor_rows.clamp(PECK_MIN_HEIGHT, PECK_MAX_HEIGHT);
        let with_peck = side.height >= CONTEXT_HEIGHT + peck_height;
        let (top, peck) = if with_peck {
            let [top, peck] =
                Split::vertical([Constraint::Min(0), Constraint::Length(peck_height)]).areas(side);
            (top, Some(peck))
        } else {
            (side, None)
        };
        areas.peck = peck;

        let with_trends = state.show_trends && top.height >= CONTEXT_HEIGHT + TRENDS_MIN_HEIGHT;
        // The logo needs the trends to have all their rows as well.
        let needed = CONTEXT_HEIGHT + if with_trends { TRENDS_FULL_HEIGHT } else { 0 };
        let top = if top.height >= LOGO_HEIGHT + needed {
            let [logo, rest] =
                Split::vertical([Constraint::Length(LOGO_HEIGHT), Constraint::Min(0)]).areas(top);
            areas.logo = Some(logo);
            rest
        } else {
            top
        };
        if with_trends {
            let [context, trends] = Split::vertical([
                Constraint::Length(CONTEXT_HEIGHT),
                Constraint::Min(TRENDS_MIN_HEIGHT),
            ])
            .areas(top);
            areas.context = Some(context);
            areas.trends = Some(trends);
        } else {
            areas.context = Some(top);
        }
    }
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
    /// The line between the header and the rows.
    pub rule: Rect,
    pub rows: Rect,
}

/// Splits the inside of the results pane: an optional tab line, the column
/// header, a rule under it, the rows, then the summary line at the bottom,
/// just above the editor.
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
        summary: row(inner.height.saturating_sub(1)),
        tabs: with_tabs.then(|| row(0)),
        header: row(tab_height),
        rule: row(1 + tab_height),
        rows: Rect {
            y: inner.y.saturating_add(rows_top).min(inner.bottom()),
            height: inner.height.saturating_sub(rows_top + 1),
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
