//! Input events. A dedicated thread reads the terminal and forwards events, so
//! the UI task never blocks on input (and background tasks can share the
//! channel later).

use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event as TermEvent, KeyEvent, KeyEventKind};
use tokio::sync::mpsc::UnboundedSender;

use crate::catalog::Catalog;
use crate::command::DocId;
use crate::db::{ConnId, ConnectionInfo, DbError, QueryOutput, SessionStats};

#[derive(Debug, Clone)]
pub enum Event {
    Key(KeyEvent),
    Resize(u16, u16),
    /// Text pasted through bracketed paste. It is data, never keystrokes.
    Paste(String),
    /// A timer tick, sent only while something needs the screen refreshed
    /// (a running query's elapsed time).
    Tick,
    /// A result from work running in the background.
    Background(Background),
}

/// What background tasks report back.
#[derive(Debug, Clone)]
pub enum Background {
    /// No stored password was found. The app should ask (R2.1).
    PasswordNeeded {
        conn: ConnId,
        notes: Vec<String>,
    },
    Connected {
        conn: ConnId,
        info: ConnectionInfo,
    },
    ConnectFailed {
        conn: ConnId,
        error: DbError,
    },
    ConnectionClosed {
        conn: ConnId,
        reason: String,
    },
    QueryFinished {
        conn: ConnId,
        doc: DocId,
        output: QueryOutput,
    },
    /// A window of rows of a paged result.
    PageLoaded {
        conn: ConnId,
        doc: DocId,
        cursor: String,
        start: u64,
        result: Result<Vec<Vec<Option<String>>>, DbError>,
    },
    Stats {
        conn: ConnId,
        result: Result<SessionStats, DbError>,
    },
    Catalog {
        conn: ConnId,
        result: Result<Catalog, DbError>,
    },
    ProfilesSaved(Result<(), String>),
}

/// Starts the input thread. It stops when the receiver is dropped or the
/// terminal can no longer be read.
pub fn spawn_input(tx: UnboundedSender<Event>) {
    thread::spawn(move || loop {
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => {}
            Ok(false) => {
                if tx.is_closed() {
                    return;
                }
                continue;
            }
            Err(e) => {
                tracing::error!(error = %e, "terminal poll failed");
                return;
            }
        }
        let forwarded = match event::read() {
            // Some terminals also report key releases. Only presses and repeats count.
            Ok(TermEvent::Key(key)) if key.kind != KeyEventKind::Release => {
                tx.send(Event::Key(key))
            }
            Ok(TermEvent::Resize(w, h)) => tx.send(Event::Resize(w, h)),
            Ok(TermEvent::Paste(text)) => tx.send(Event::Paste(text)),
            Ok(_) => Ok(()),
            Err(e) => {
                tracing::error!(error = %e, "terminal read failed");
                return;
            }
        };
        if forwarded.is_err() {
            return;
        }
    });
}
