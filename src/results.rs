//! What the results pane shows for a buffer, and how it is navigated.
//! Pure logic: rendering lives in `ui`.

use crate::db::{DbError, QueryOutput, ResultSet};

/// One tab: a statement's result, or the error that stopped the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab<'a> {
    Set(&'a ResultSet),
    Error(&'a DbError),
}

/// Where the view goes once a requested window of rows arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// Keep this row (counted in the whole result) at the top.
    Keep(u64),
    Top,
    Bottom,
}

/// A window of rows to read from a paged result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub cursor: String,
    pub start: u64,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Results {
    pub output: QueryOutput,
    pub tab: usize,
    /// First visible row and first visible column of the current tab.
    /// The row counts from the start of the loaded window.
    pub row: usize,
    pub col: usize,
    /// A window of a paged result is on its way.
    pub loading: Option<Anchor>,
}

impl Results {
    pub fn new(output: QueryOutput) -> Self {
        let mut results = Self {
            output,
            tab: 0,
            row: 0,
            col: 0,
            loading: None,
        };
        // Show the error first, since it is what the user must act on.
        // Otherwise show the last statement that returned rows, or the last one.
        results.tab = match (&results.output.error, results.output.sets.len()) {
            (Some(_), n) => n,
            (None, 0) => 0,
            (None, n) => results
                .output
                .sets
                .iter()
                .rposition(ResultSet::returns_rows)
                .unwrap_or(n - 1),
        };
        results
    }

    pub fn tab_count(&self) -> usize {
        self.output.sets.len() + usize::from(self.output.error.is_some())
    }

    pub fn tab(&self, index: usize) -> Option<Tab<'_>> {
        match self.output.sets.get(index) {
            Some(set) => Some(Tab::Set(set)),
            None if index == self.output.sets.len() => self.output.error.as_ref().map(Tab::Error),
            None => None,
        }
    }

    pub fn current(&self) -> Option<Tab<'_>> {
        self.tab(self.tab)
    }

    fn current_set(&self) -> Option<&ResultSet> {
        match self.current()? {
            Tab::Set(set) => Some(set),
            Tab::Error(_) => None,
        }
    }

    pub fn row_count(&self) -> usize {
        self.current_set().map_or(0, |s| s.rows.len())
    }

    fn column_count(&self) -> usize {
        self.current_set().map_or(0, |s| s.columns.len())
    }

    pub fn select_tab(&mut self, step: isize) {
        let count = self.tab_count() as isize;
        if count > 0 {
            self.tab = (self.tab as isize + step).rem_euclid(count) as usize;
            self.row = 0;
            self.col = 0;
        }
    }

    /// Moves the first visible row. `page` is how many rows fit.
    pub fn scroll_rows(&mut self, delta: isize, page: usize) {
        let last_start = self.row_count().saturating_sub(page.max(1));
        let moved = (self.row as isize + delta).clamp(0, last_start as isize);
        self.row = moved as usize;
    }

    pub fn first_row(&mut self) {
        self.row = 0;
    }

    pub fn last_row(&mut self, page: usize) {
        self.row = self.row_count().saturating_sub(page.max(1));
    }

    /// The current tab, when it is a paged result.
    fn paged_set(&self) -> Option<&ResultSet> {
        self.current_set().filter(|s| s.is_paged())
    }

    fn request(&mut self, start: u64, count: usize, anchor: Anchor) -> Option<PageRequest> {
        let cursor = self.paged_set()?.cursor.clone()?;
        self.loading = Some(anchor);
        Some(PageRequest {
            cursor,
            start,
            count,
        })
    }

    /// Scrolls like `scroll_rows`. At the edge of a paged result's window it
    /// asks for the next or previous window, which the caller must fetch.
    pub fn scroll_rows_paged(
        &mut self,
        delta: isize,
        page: usize,
        limit: usize,
    ) -> Option<PageRequest> {
        self.scroll_rows(delta, page);
        if self.loading.is_some() {
            return None;
        }
        let set = self.paged_set()?;
        let (offset, len, total) = (set.offset, set.rows.len(), set.total_rows);
        let last_start = len.saturating_sub(page.max(1));
        if delta > 0 && self.row >= last_start && offset + (len as u64) < total {
            let top = offset + self.row as u64;
            let start = top.min(total.saturating_sub(limit as u64));
            return self.request(start, limit, Anchor::Keep(top));
        }
        if delta < 0 && self.row == 0 && offset > 0 {
            let start = (offset + page as u64).saturating_sub(limit as u64);
            return self.request(start, limit, Anchor::Keep(offset));
        }
        None
    }

    /// Jumps to the first row of the whole result.
    pub fn first_row_paged(&mut self, limit: usize) -> Option<PageRequest> {
        if self.loading.is_none() && self.paged_set().is_some_and(|s| s.offset > 0) {
            return self.request(0, limit, Anchor::Top);
        }
        self.first_row();
        None
    }

    /// Jumps to the last row of the whole result.
    pub fn last_row_paged(&mut self, page: usize, limit: usize) -> Option<PageRequest> {
        let more_below = self
            .paged_set()
            .is_some_and(|s| s.offset + (s.rows.len() as u64) < s.total_rows);
        if self.loading.is_none() && more_below {
            let total = self.paged_set()?.total_rows;
            return self.request(total.saturating_sub(limit as u64), limit, Anchor::Bottom);
        }
        self.last_row(page);
        None
    }

    /// Puts a window that arrived in place. False when the rows belong to a
    /// result that is no longer shown.
    pub fn apply_page(
        &mut self,
        cursor: &str,
        start: u64,
        rows: Vec<Vec<Option<String>>>,
        page: usize,
    ) -> bool {
        let Some(set) = self
            .output
            .sets
            .iter_mut()
            .find(|s| s.cursor.as_deref() == Some(cursor))
        else {
            return false;
        };
        set.offset = start;
        set.rows = rows;
        let last_start = set.rows.len().saturating_sub(page.max(1));
        self.row = match self.loading.take() {
            Some(Anchor::Keep(top)) => (top.saturating_sub(start) as usize).min(last_start),
            Some(Anchor::Top) => 0,
            Some(Anchor::Bottom) => last_start,
            None => self.row.min(last_start),
        };
        true
    }

    /// A requested window could not be read.
    pub fn page_failed(&mut self) {
        self.loading = None;
    }

    pub fn scroll_columns(&mut self, delta: isize) {
        let last = self.column_count().saturating_sub(1);
        self.col = (self.col as isize + delta).clamp(0, last as isize) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn set(rows: usize, columns: usize) -> ResultSet {
        ResultSet {
            columns: (0..columns).map(|i| format!("c{i}")).collect(),
            rows: (0..rows)
                .map(|r| (0..columns).map(|c| Some(format!("{r}.{c}"))).collect())
                .collect(),
            total_rows: rows as u64,
            tag: format!("SELECT {rows}"),
            offset: 0,
            cursor: None,
        }
    }

    fn command_only() -> ResultSet {
        ResultSet {
            columns: Vec::new(),
            rows: Vec::new(),
            total_rows: 0,
            tag: "CREATE TABLE".to_string(),
            offset: 0,
            cursor: None,
        }
    }

    fn output(sets: Vec<ResultSet>, error: Option<DbError>) -> QueryOutput {
        QueryOutput {
            sets,
            error,
            elapsed: Duration::from_millis(5),
            tx: crate::db::TxState::Idle,
        }
    }

    #[test]
    fn opens_on_the_last_statement_that_returned_rows() {
        let r = Results::new(output(
            vec![set(2, 1), command_only(), set(3, 1), command_only()],
            None,
        ));
        assert_eq!(r.tab, 2);
        let r = Results::new(output(vec![command_only(), command_only()], None));
        assert_eq!(r.tab, 1);
    }

    #[test]
    fn an_error_gets_its_own_last_tab_and_opens_first() {
        let r = Results::new(output(vec![set(1, 1)], Some(DbError::new("boom"))));
        assert_eq!(r.tab_count(), 2);
        assert!(matches!(r.current(), Some(Tab::Error(e)) if e.message == "boom"));
        assert!(matches!(r.tab(0), Some(Tab::Set(_))));
        let only_error = Results::new(output(Vec::new(), Some(DbError::new("x"))));
        assert_eq!((only_error.tab_count(), only_error.tab), (1, 0));
    }

    #[test]
    fn tabs_wrap_and_reset_the_scroll() {
        let mut r = Results::new(output(vec![set(50, 5), set(50, 5)], None));
        r.scroll_rows(10, 10);
        r.scroll_columns(2);
        r.select_tab(1);
        assert_eq!((r.tab, r.row, r.col), (0, 0, 0));
        r.select_tab(-1);
        assert_eq!(r.tab, 1);
    }

    #[test]
    fn rows_scroll_within_bounds() {
        let mut r = Results::new(output(vec![set(30, 2)], None));
        r.scroll_rows(-5, 10);
        assert_eq!(r.row, 0);
        r.scroll_rows(7, 10);
        assert_eq!(r.row, 7);
        r.scroll_rows(1000, 10);
        assert_eq!(r.row, 20, "the last page stays full");
        r.first_row();
        assert_eq!(r.row, 0);
        r.last_row(10);
        assert_eq!(r.row, 20);
    }

    fn paged(total: u64, window: usize) -> Results {
        let mut s = set(window, 2);
        s.total_rows = total;
        s.cursor = Some("c".to_string());
        Results::new(output(vec![s], None))
    }

    fn rows(from: u64, count: usize) -> Vec<Vec<Option<String>>> {
        (from..from + count as u64)
            .map(|r| vec![Some(r.to_string()), None])
            .collect()
    }

    #[test]
    fn reaching_the_end_of_a_window_asks_for_the_next_one() {
        let mut r = paged(100, 30);
        // Inside the window nothing is requested.
        assert_eq!(r.scroll_rows_paged(5, 10, 30), None);
        // Reaching the last full page of the window asks for the next one.
        let request = r.scroll_rows_paged(15, 10, 30).expect("next window");
        assert_eq!(r.row, 20);
        assert_eq!((request.start, request.count), (20, 30));
        assert_eq!(r.loading, Some(Anchor::Keep(20)));
        // While it loads, no second request.
        assert_eq!(r.scroll_rows_paged(1, 10, 30), None);
        assert!(r.apply_page("c", 20, rows(20, 30), 10));
        assert_eq!((r.row, r.loading), (0, None));
        assert_eq!(r.output.sets[0].offset, 20);
    }

    #[test]
    fn the_last_window_is_full_and_the_end_stops_asking() {
        let mut r = paged(100, 30);
        r.row = 20;
        let request = r.scroll_rows_paged(1, 10, 30).expect("window");
        assert_eq!(request.start, 20);
        assert!(r.apply_page("c", 70, rows(70, 30), 10));
        // Window 70..100 is the end: no more to ask for.
        assert_eq!(r.scroll_rows_paged(50, 10, 30), None);
    }

    #[test]
    fn scrolling_up_past_the_start_of_a_window_asks_for_the_previous_one() {
        let mut r = paged(100, 30);
        r.row = 20;
        r.scroll_rows_paged(1, 10, 30);
        assert!(r.apply_page("c", 40, rows(40, 30), 10));
        assert_eq!(r.row, 0);
        let request = r.scroll_rows_paged(-1, 10, 30).expect("previous window");
        assert_eq!((request.start, request.count), (20, 30));
        assert!(r.apply_page("c", 20, rows(20, 30), 10));
        // Row 40 is where it was: 20 rows into the new window.
        assert_eq!(r.row, 20);
    }

    #[test]
    fn first_and_last_row_reach_the_ends_of_the_whole_result() {
        let mut r = paged(100, 30);
        let request = r.last_row_paged(10, 30).expect("last window");
        assert_eq!(request.start, 70);
        assert!(r.apply_page("c", 70, rows(70, 30), 10));
        assert_eq!(r.row, 20);
        assert_eq!(r.last_row_paged(10, 30), None);
        let request = r.first_row_paged(30).expect("first window");
        assert_eq!(request.start, 0);
        assert!(r.apply_page("c", 0, rows(0, 30), 10));
        assert_eq!(r.row, 0);
    }

    #[test]
    fn rows_for_a_result_that_is_gone_are_dropped_and_failures_clear_loading() {
        let mut r = paged(100, 30);
        r.row = 20;
        r.scroll_rows_paged(1, 10, 30);
        assert!(!r.apply_page("other", 20, rows(20, 30), 10));
        r.page_failed();
        assert_eq!(r.loading, None);
    }

    #[test]
    fn a_result_that_is_not_paged_never_asks() {
        let mut r = Results::new(output(vec![set(30, 2)], None));
        assert_eq!(r.scroll_rows_paged(100, 10, 30), None);
        assert_eq!(r.last_row_paged(10, 30), None);
        assert_eq!(r.first_row_paged(30), None);
    }

    #[test]
    fn a_short_result_does_not_scroll() {
        let mut r = Results::new(output(vec![set(3, 2)], None));
        r.scroll_rows(5, 10);
        r.last_row(10);
        assert_eq!(r.row, 0);
    }

    #[test]
    fn columns_scroll_within_bounds() {
        let mut r = Results::new(output(vec![set(3, 4)], None));
        r.scroll_columns(10);
        assert_eq!(r.col, 3);
        r.scroll_columns(-10);
        assert_eq!(r.col, 0);
    }

    #[test]
    fn navigation_on_an_error_or_empty_tab_is_harmless() {
        let mut r = Results::new(output(Vec::new(), Some(DbError::new("x"))));
        r.scroll_rows(3, 10);
        r.scroll_columns(3);
        r.last_row(10);
        assert_eq!((r.row, r.col), (0, 0));
        let mut empty = Results::new(output(Vec::new(), None));
        empty.select_tab(1);
        assert_eq!(empty.tab_count(), 0);
    }
}
