//! Passwords. This is the only module that reads them (R2).
//!
//! pgpeck never stores a password. By default it asks every time. Users can
//! opt in to reading one that they already keep elsewhere: a `.pgpass` file or
//! an environment variable. The prompt itself is a UI concern, so lookup only
//! reports that one is needed.
//!
//! A password is held as a `Secret`, which cannot be printed.
//!
//! Limitation: `Secret` does not overwrite its memory when dropped. Doing that
//! reliably needs `unsafe` or a new dependency, and the password is also held
//! by the database driver.

use std::fmt;
use std::path::{Path, PathBuf};

/// A password or token. It has no `Display`, and `Debug` hides the value, so
/// it cannot reach logs, error messages or the screen by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value. Call this only where the password is handed to the
    /// database driver.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Where a password can come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Pgpass,
    Env,
    Prompt,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Pgpass, Source::Env, Source::Prompt];

    pub fn name(self) -> &'static str {
        match self {
            Source::Pgpass => "pgpass",
            Source::Env => "env",
            Source::Prompt => "prompt",
        }
    }

    pub fn from_name(name: &str) -> Option<Source> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The default: always ask. Nothing is read from anywhere else unless the
    /// user lists it in the configuration.
    pub fn default_order() -> Vec<Source> {
        vec![Source::Prompt]
    }
}

// ---------------------------------------------------------------------------
// .pgpass
// ---------------------------------------------------------------------------

/// What a `.pgpass` file says about one connection.
#[derive(Debug, PartialEq, Eq)]
pub enum PgpassResult {
    Found(Secret),
    NoMatch,
    /// The file exists but must not be used, with the reason.
    Ignored(String),
}

/// `$PGPASSFILE`, or `~/.pgpass`.
pub fn pgpass_path(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(path) = env("PGPASSFILE").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    env("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".pgpass"))
}

/// Looks up a connection in `.pgpass` text. The format is
/// `host:port:database:user:password`, where `*` matches anything and `\:` and
/// `\\` are escapes. The first matching line wins.
pub fn pgpass_lookup(
    text: &str,
    host: &str,
    port: u16,
    database: &str,
    user: &str,
) -> Option<Secret> {
    let port = port.to_string();
    let wanted = [host, port.as_str(), database, user];
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .find_map(|line| {
            let (fields, password) = split_pgpass_line(line)?;
            let matches = fields
                .iter()
                .zip(wanted)
                .all(|(have, want)| have == "*" || have == want);
            matches.then(|| Secret::new(password))
        })
}

/// Splits one line into the four matching fields and the password.
fn split_pgpass_line(line: &str) -> Option<([String; 4], String)> {
    let mut fields: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = line.trim_end_matches(['\r', '\n']).chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next) => current.push(next),
                None => current.push('\\'),
            },
            ':' if fields.len() < 4 => fields.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    let [a, b, c, d]: [String; 4] = fields.try_into().ok()?;
    Some(([a, b, c, d], current))
}

/// Reads and searches a `.pgpass` file. Like libpq, a file that other users
/// can read is not used.
pub fn read_pgpass(path: &Path, host: &str, port: u16, database: &str, user: &str) -> PgpassResult {
    let Ok(meta) = std::fs::metadata(path) else {
        return PgpassResult::NoMatch;
    };
    if !meta.is_file() {
        return PgpassResult::NoMatch;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return PgpassResult::Ignored(format!(
                "{} is readable by other users. Run: chmod 0600 {}",
                path.display(),
                path.display()
            ));
        }
    }
    match std::fs::read_to_string(path) {
        Ok(text) => pgpass_lookup(&text, host, port, database, user)
            .map_or(PgpassResult::NoMatch, PgpassResult::Found),
        Err(e) => PgpassResult::Ignored(format!("cannot read {}: {}", path.display(), e.kind())),
    }
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// The connection a password is wanted for.
#[derive(Debug, Clone)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    /// Environment variable to read. `PGPASSWORD` when not set.
    pub env_var: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Found(Secret, Source),
    /// No password was found. The caller should ask the user.
    NeedsPrompt,
    /// Nothing found and prompting is not enabled.
    NotFound,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Resolution {
    pub outcome: Outcome,
    /// Things the user may want to know, for example an unusable `.pgpass`.
    /// They never contain a password.
    pub notes: Vec<String>,
}

/// Where lookups read from. Tests replace it.
pub struct Environment<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
}

/// Tries the sources in order.
pub fn resolve(order: &[Source], target: &Target, environment: &Environment) -> Resolution {
    let mut notes = Vec::new();
    for source in order {
        let found = match source {
            Source::Pgpass => pgpass_path(environment.env).and_then(|path| {
                match read_pgpass(
                    &path,
                    &target.host,
                    target.port,
                    &target.database,
                    &target.user,
                ) {
                    PgpassResult::Found(secret) => Some(secret),
                    PgpassResult::NoMatch => None,
                    PgpassResult::Ignored(reason) => {
                        notes.push(reason);
                        None
                    }
                }
            }),
            Source::Env => {
                let var = target.env_var.as_deref().unwrap_or("PGPASSWORD");
                (environment.env)(var)
                    .filter(|v| !v.is_empty())
                    .map(Secret::new)
            }
            Source::Prompt => {
                return Resolution {
                    outcome: Outcome::NeedsPrompt,
                    notes,
                };
            }
        };
        if let Some(secret) = found {
            return Resolution {
                outcome: Outcome::Found(secret, *source),
                notes,
            };
        }
    }
    Resolution {
        outcome: Outcome::NotFound,
        notes,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn target() -> Target {
        Target {
            host: "db.example.com".to_string(),
            port: 5432,
            database: "app".to_string(),
            user: "alice".to_string(),
            env_var: None,
        }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn found(resolution: &Resolution) -> Option<(&str, Source)> {
        match &resolution.outcome {
            Outcome::Found(secret, source) => Some((secret.expose(), *source)),
            _ => None,
        }
    }

    // -- Secret ---------------------------------------------------------------

    #[test]
    fn a_secret_never_prints_its_value() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(format!("{secret:#?}"), "Secret(***)");
        // Containers that derive Debug stay clean too.
        let outcome = Outcome::Found(secret, Source::Env);
        assert!(!format!("{outcome:?}").contains("hunter2"));
        let resolution = Resolution {
            outcome,
            notes: Vec::new(),
        };
        assert!(!format!("{resolution:?}").contains("hunter2"));
    }

    #[test]
    fn source_names_round_trip() {
        for source in Source::ALL {
            assert_eq!(Source::from_name(source.name()), Some(source));
        }
        assert_eq!(Source::from_name("vault"), None);
        assert_eq!(
            Source::from_name("keychain"),
            None,
            "pgpeck has no keychain support"
        );
    }

    #[test]
    fn by_default_nothing_is_read_and_the_user_is_asked() {
        assert_eq!(Source::default_order(), [Source::Prompt]);
        let env = |_: &str| Some("from-env".to_string());
        let environment = Environment { env: &env };
        let r = resolve(&Source::default_order(), &target(), &environment);
        assert_eq!(
            r.outcome,
            Outcome::NeedsPrompt,
            "the environment is ignored unless enabled"
        );
    }

    // -- .pgpass --------------------------------------------------------------

    const PGPASS: &str = "\
# a comment
localhost:5432:*:bob:bobpass
db.example.com:*:app:alice:with\\:colon\\\\and:more
*:*:*:*:fallback
";

    fn lookup(host: &str, port: u16, db: &str, user: &str) -> Option<String> {
        pgpass_lookup(PGPASS, host, port, db, user).map(|s| s.expose().to_string())
    }

    #[test]
    fn pgpass_matches_exact_wildcard_and_first_line_wins() {
        assert_eq!(
            lookup("localhost", 5432, "anything", "bob").as_deref(),
            Some("bobpass")
        );
        assert_eq!(
            lookup("localhost", 5433, "x", "bob").as_deref(),
            Some("fallback")
        );
        assert_eq!(lookup("other", 1, "x", "y").as_deref(), Some("fallback"));
    }

    #[test]
    fn pgpass_handles_escapes_and_colons_in_the_password() {
        assert_eq!(
            lookup("db.example.com", 5432, "app", "alice").as_deref(),
            Some("with:colon\\and:more")
        );
    }

    #[test]
    fn pgpass_without_a_match_or_with_bad_lines() {
        assert_eq!(pgpass_lookup("a:b:c:d:e", "x", 1, "y", "z"), None);
        assert_eq!(pgpass_lookup("too:few:fields", "too", 1, "y", "z"), None);
        assert_eq!(pgpass_lookup("", "h", 1, "d", "u"), None);
        assert_eq!(pgpass_lookup("# *:*:*:*:x\n", "h", 1, "d", "u"), None);
    }

    #[test]
    fn pgpass_accepts_crlf_and_an_empty_password() {
        let secret = pgpass_lookup("h:1:d:u:pw\r\n", "h", 1, "d", "u");
        assert_eq!(
            secret.map(|s| s.expose().to_string()).as_deref(),
            Some("pw")
        );
        let secret = pgpass_lookup("h:1:d:u:\n", "h", 1, "d", "u");
        assert!(secret.is_some_and(|s| s.is_empty()));
    }

    fn temp_file(name: &str, contents: &str, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("pgpeck-secrets-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("pgpass");
        std::fs::write(&path, contents).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod");
        path
    }

    #[test]
    fn pgpass_file_is_used_only_when_private() {
        let private = temp_file("private", "h:1:d:u:pw\n", 0o600);
        assert!(matches!(
            read_pgpass(&private, "h", 1, "d", "u"),
            PgpassResult::Found(_)
        ));
        assert_eq!(
            read_pgpass(&private, "h", 2, "d", "u"),
            PgpassResult::NoMatch
        );

        let open = temp_file("open", "h:1:d:u:pw\n", 0o644);
        match read_pgpass(&open, "h", 1, "d", "u") {
            PgpassResult::Ignored(reason) => {
                assert!(reason.contains("chmod 0600"));
                assert!(!reason.contains("pw\n"));
            }
            other => panic!("expected Ignored, got {other:?}"),
        }
        let missing = private.with_file_name("nope");
        assert_eq!(
            read_pgpass(&missing, "h", 1, "d", "u"),
            PgpassResult::NoMatch
        );
    }

    #[test]
    fn pgpass_path_prefers_pgpassfile() {
        let env = |k: &str| match k {
            "PGPASSFILE" => Some("/tmp/custom".to_string()),
            "HOME" => Some("/home/u".to_string()),
            _ => None,
        };
        assert_eq!(pgpass_path(&env), Some(PathBuf::from("/tmp/custom")));
        let env = |k: &str| (k == "HOME").then(|| "/home/u".to_string());
        assert_eq!(pgpass_path(&env), Some(PathBuf::from("/home/u/.pgpass")));
        assert_eq!(pgpass_path(&no_env), None);
    }

    // -- resolve --------------------------------------------------------------

    #[test]
    fn order_is_configurable() {
        let env = |k: &str| (k == "PGPASSWORD").then(|| "from-env".to_string());
        let environment = Environment { env: &env };
        let r = resolve(&[Source::Env, Source::Prompt], &target(), &environment);
        assert_eq!(found(&r), Some(("from-env", Source::Env)));
    }

    #[test]
    fn a_profile_can_name_its_own_variable() {
        let env = |k: &str| match k {
            "PGPASSWORD" => Some("wrong".to_string()),
            "PROD_DB_PW" => Some("right".to_string()),
            _ => None,
        };
        let environment = Environment { env: &env };
        let mut t = target();
        t.env_var = Some("PROD_DB_PW".to_string());
        let r = resolve(&[Source::Env], &t, &environment);
        assert_eq!(found(&r), Some(("right", Source::Env)));
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        let env = |k: &str| (k == "PGPASSWORD").then(String::new);
        let environment = Environment { env: &env };
        let r = resolve(&[Source::Env], &target(), &environment);
        assert_eq!(r.outcome, Outcome::NotFound);
    }

    #[test]
    fn nothing_found_asks_when_prompting_is_enabled() {
        let environment = Environment { env: &no_env };
        let r = resolve(&[Source::Env, Source::Prompt], &target(), &environment);
        assert_eq!(r.outcome, Outcome::NeedsPrompt);
        let r = resolve(&[Source::Env], &target(), &environment);
        assert_eq!(r.outcome, Outcome::NotFound);
    }

    #[test]
    fn prompt_in_front_stops_the_search() {
        let env = |k: &str| (k == "PGPASSWORD").then(|| "from-env".to_string());
        let environment = Environment { env: &env };
        let r = resolve(&[Source::Prompt, Source::Env], &target(), &environment);
        assert_eq!(r.outcome, Outcome::NeedsPrompt);
    }

    #[test]
    fn an_unusable_pgpass_is_reported_without_leaking_it() {
        let path = temp_file("note", "db.example.com:5432:app:alice:topsecret\n", 0o666);
        let path_text = path.display().to_string();
        let env = move |k: &str| (k == "PGPASSFILE").then(|| path_text.clone());
        let environment = Environment { env: &env };
        let r = resolve(&[Source::Pgpass], &target(), &environment);
        assert_eq!(r.outcome, Outcome::NotFound);
        assert_eq!(r.notes.len(), 1);
        assert!(!r.notes[0].contains("topsecret"));
    }

    #[test]
    fn pgpass_is_used_in_order() {
        let path = temp_file("use", "db.example.com:5432:app:alice:from-pgpass\n", 0o600);
        let path_text = path.display().to_string();
        let env = move |k: &str| (k == "PGPASSFILE").then(|| path_text.clone());
        let environment = Environment { env: &env };
        let r = resolve(&[Source::Pgpass, Source::Env], &target(), &environment);
        assert_eq!(found(&r), Some(("from-pgpass", Source::Pgpass)));
    }
}
