//! Work the application asks the runtime to do. `App::update` stays pure: it
//! returns commands instead of performing I/O (steering: `app` must not do I/O).

use crate::db::ConnId;
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
    },
    /// Read the per-database statistics for the session pane.
    Stats {
        conn: ConnId,
    },
    Cancel {
        conn: ConnId,
    },
    Disconnect {
        conn: ConnId,
    },
    /// Replace `connections.toml` with these contents.
    SaveProfiles {
        contents: String,
    },
}
