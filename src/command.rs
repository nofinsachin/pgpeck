//! Work the application asks the runtime to do. `App::update` stays pure: it
//! returns commands instead of performing I/O (steering: `app` must not do I/O).

use crate::db::{ConnId, Paging};
use crate::profile::Profile;
use crate::secrets::Secret;

/// Identifies a buffer for as long as it lives, whatever its position.
pub type DocId = u64;

#[derive(Debug, Clone)]
pub enum Command {
    /// Connect. With `lookup`, the password is first searched in the
    /// configured sources, and the app is asked when none has it.
    Connect {
        conn: ConnId,
        profile: Box<Profile>,
        password: Option<Secret>,
        lookup: bool,
    },
    Run {
        conn: ConnId,
        doc: DocId,
        sql: String,
        /// Whether, and under which name, the result may be paged.
        paging: Paging,
    },
    /// Read another window of rows from a paged result.
    FetchPage {
        conn: ConnId,
        doc: DocId,
        cursor: String,
        start: u64,
        count: usize,
    },
    /// Read the per-database statistics for the session pane.
    Stats {
        conn: ConnId,
    },
    /// Read the schema metadata for the find overlay.
    Catalog {
        conn: ConnId,
    },
    Cancel {
        conn: ConnId,
    },
    Disconnect {
        conn: ConnId,
    },
    /// Add a line to the history file.
    AppendHistory {
        path: std::path::PathBuf,
        line: String,
    },
    /// Replace `connections.toml` with these contents.
    SaveProfiles {
        contents: String,
    },
}
