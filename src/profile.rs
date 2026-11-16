//! Connection profiles (R1): everything needed to connect except the password,
//! which oxpecker never stores.
//!
//! Profiles live in `connections.toml` in the config directory, one table per
//! profile. The file is written with owner-only permissions (R2.6). Entries
//! this version cannot interpret, and options it does not know, are kept and
//! written back, so editing one profile never destroys another.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;

use ratatui::style::Color;

use crate::diag::Diagnostic;
use crate::secrets::{Secret, Target};

pub const FILE: &str = "connections.toml";
pub const DEFAULT_PORT: u16 = 5432;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SslMode {
    Disable,
    /// libpq's default: use TLS when the server offers it.
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub fn name(self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        }
    }

    pub fn from_name(name: &str) -> Option<SslMode> {
        [
            SslMode::Disable,
            SslMode::Prefer,
            SslMode::Require,
            SslMode::VerifyCa,
            SslMode::VerifyFull,
        ]
        .into_iter()
        .find(|m| m.name() == name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// Unique name, also the table name in the file.
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: Option<String>,
    pub user: Option<String>,
    pub ssl_mode: SslMode,
    /// CA certificate file for `verify-ca` and `verify-full`.
    pub ssl_root_cert: Option<String>,
    /// Shown instead of the name.
    pub label: Option<String>,
    /// Color name or `#rrggbb` for the status bar.
    pub color: Option<String>,
    /// Marks the connection as production (R1.6, R15).
    pub production: bool,
    /// Environment variable that holds the password. `PGPASSWORD` when unset.
    pub password_env: Option<String>,
    /// Seconds since the Unix epoch of the last connection.
    pub last_used: Option<u64>,
    /// Options from a newer version, kept so saving does not lose them.
    extra: toml::Table,
}

impl Profile {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            host: "localhost".to_string(),
            port: DEFAULT_PORT,
            database: None,
            user: None,
            ssl_mode: SslMode::default(),
            ssl_root_cert: None,
            label: None,
            color: None,
            production: false,
            password_env: None,
            last_used: None,
            extra: toml::Table::new(),
        }
    }

    /// What the UI calls this profile.
    pub fn display_name(&self) -> &str {
        self.label
            .as_deref()
            .filter(|l| !l.is_empty())
            .unwrap_or(&self.name)
    }

    /// The details a password lookup needs. `fallback_user` is the OS login
    /// name, used when the profile names no user.
    pub fn target(&self, fallback_user: &str) -> Target {
        let user = self
            .user
            .clone()
            .unwrap_or_else(|| fallback_user.to_string());
        let database = self.database.clone().unwrap_or_else(|| user.clone());
        Target {
            host: self.host.clone(),
            port: self.port,
            database,
            user,
            env_var: self.password_env.clone(),
        }
    }

    pub fn color_value(&self) -> Option<Color> {
        self.color.as_deref().and_then(|c| Color::from_str(c).ok())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("the profile needs a name".to_string());
        }
        if self.name.chars().any(char::is_control) {
            return Err("the profile name cannot contain control characters".to_string());
        }
        if self.host.trim().is_empty() {
            return Err("the host cannot be empty".to_string());
        }
        if self.port == 0 {
            return Err("the port must be between 1 and 65535".to_string());
        }
        if let Some(color) = &self.color {
            if Color::from_str(color).is_err() {
                return Err(format!("`{color}` is not a color name or #rrggbb value"));
            }
        }
        if let Some(var) = &self.password_env {
            let valid = !var.is_empty()
                && !var.starts_with(|c: char| c.is_ascii_digit())
                && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid {
                return Err("`password_env` must be an environment variable name".to_string());
            }
        }
        Ok(())
    }

    fn to_table(&self) -> toml::Table {
        let mut t = self.extra.clone();
        let mut put = |key: &str, value: toml::Value| {
            t.insert(key.to_string(), value);
        };
        put("host", self.host.clone().into());
        put("port", i64::from(self.port).into());
        put("ssl_mode", self.ssl_mode.name().into());
        let optional = [
            ("database", &self.database),
            ("user", &self.user),
            ("ssl_root_cert", &self.ssl_root_cert),
            ("label", &self.label),
            ("color", &self.color),
            ("password_env", &self.password_env),
        ];
        for (key, value) in optional {
            if let Some(v) = value {
                put(key, v.clone().into());
            }
        }
        if self.production {
            put("production", true.into());
        }
        if let Some(t) = self.last_used {
            // TOML integers are signed 64-bit. A timestamp always fits.
            put("last_used", i64::try_from(t).unwrap_or(i64::MAX).into());
        }
        t
    }

    /// Reads one profile table. Problems that make the profile unusable are
    /// errors. Cosmetic problems are returned as notes and the option is dropped.
    fn from_table(name: &str, table: &toml::Table) -> Result<(Profile, Vec<String>), String> {
        let mut p = Profile::new(name);
        let mut notes = Vec::new();
        for (key, value) in table {
            let text = || {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("`{key}` must be text"))
            };
            match key.as_str() {
                "host" => p.host = text()?,
                "port" => {
                    p.port = value
                        .as_integer()
                        .and_then(|n| u16::try_from(n).ok())
                        .filter(|n| *n != 0)
                        .ok_or("`port` must be a number from 1 to 65535")?;
                }
                "database" => p.database = Some(text()?),
                "user" => p.user = Some(text()?),
                "ssl_mode" => {
                    p.ssl_mode = SslMode::from_name(&text()?).ok_or(
                        "`ssl_mode` must be disable, prefer, require, verify-ca or verify-full",
                    )?;
                }
                "ssl_root_cert" => p.ssl_root_cert = Some(text()?),
                "label" => p.label = Some(text()?),
                "color" => {
                    let color = text()?;
                    if Color::from_str(&color).is_ok() {
                        p.color = Some(color);
                    } else {
                        notes.push(format!(
                            "`color` `{color}` is not a color name or #rrggbb value. Ignored"
                        ));
                    }
                }
                "production" => {
                    p.production = value
                        .as_bool()
                        .ok_or("`production` must be true or false")?
                }
                "password_env" => p.password_env = Some(text()?),
                "last_used" => p.last_used = value.as_integer().and_then(|n| u64::try_from(n).ok()),
                other => {
                    notes.push(format!("unknown option `{other}`. Kept as it is"));
                    p.extra.insert(other.to_string(), value.clone());
                }
            }
        }
        p.validate()?;
        Ok((p, notes))
    }
}

/// All saved profiles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profiles {
    list: Vec<Profile>,
    /// File entries this version could not use, written back unchanged.
    preserved: BTreeMap<String, toml::Value>,
}

impl Profiles {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn all(&self) -> &[Profile] {
        &self.list
    }

    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.list.iter().find(|p| p.name == name)
    }

    fn name_taken(&self, name: &str) -> bool {
        self.list.iter().any(|p| p.name == name) || self.preserved.contains_key(name)
    }

    /// Profiles for the picker: most recently used first, then by name (R1.3).
    pub fn for_picker(&self) -> Vec<&Profile> {
        let mut list: Vec<&Profile> = self.list.iter().collect();
        list.sort_by(|a, b| {
            b.last_used
                .cmp(&a.last_used)
                .then_with(|| a.name.cmp(&b.name))
        });
        list
    }

    pub fn add(&mut self, profile: Profile) -> Result<(), String> {
        profile.validate()?;
        if self.name_taken(&profile.name) {
            return Err(format!("a profile named `{}` already exists", profile.name));
        }
        self.list.push(profile);
        self.list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }

    /// Replaces the profile called `old_name`, which may also rename it.
    pub fn update(&mut self, old_name: &str, profile: Profile) -> Result<(), String> {
        profile.validate()?;
        let index = self
            .list
            .iter()
            .position(|p| p.name == old_name)
            .ok_or_else(|| format!("no profile named `{old_name}`"))?;
        if profile.name != old_name && self.name_taken(&profile.name) {
            return Err(format!("a profile named `{}` already exists", profile.name));
        }
        // Keep options this version does not know about.
        let mut profile = profile;
        for (k, v) in &self.list[index].extra {
            profile.extra.entry(k.clone()).or_insert_with(|| v.clone());
        }
        self.list[index] = profile;
        self.list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }

    pub fn duplicate(&mut self, name: &str, new_name: &str) -> Result<(), String> {
        let mut copy = self
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no profile named `{name}`"))?;
        copy.name = new_name.to_string();
        copy.last_used = None;
        self.add(copy)
    }

    pub fn delete(&mut self, name: &str) -> Result<(), String> {
        let index = self
            .list
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| format!("no profile named `{name}`"))?;
        self.list.remove(index);
        Ok(())
    }

    /// Records that a profile was just used.
    pub fn touch(&mut self, name: &str, now: u64) {
        if let Some(p) = self.list.iter_mut().find(|p| p.name == name) {
            p.last_used = Some(now);
        }
    }

    pub fn parse(src: &str) -> (Profiles, Vec<Diagnostic>) {
        let mut profiles = Profiles::default();
        let mut diags = Vec::new();
        let table = match toml::from_str::<toml::Table>(src) {
            Ok(t) => t,
            Err(e) => {
                let line = e.span().map(|s| crate::diag::line_of_offset(src, s.start));
                diags.push(Diagnostic::new(FILE, line, e.message().to_string()));
                return (profiles, diags);
            }
        };
        for (name, value) in table {
            let line = crate::diag::line_containing(src, &format!("[{name}]"))
                .or_else(|| crate::diag::line_containing(src, &name));
            let Some(body) = value.as_table() else {
                diags.push(Diagnostic::new(
                    FILE,
                    line,
                    format!("`{name}` is not a profile table. Kept as it is"),
                ));
                profiles.preserved.insert(name, value);
                continue;
            };
            match Profile::from_table(&name, body) {
                Ok((profile, notes)) => {
                    for note in notes {
                        diags.push(Diagnostic::new(
                            FILE,
                            line,
                            format!("profile `{name}`: {note}"),
                        ));
                    }
                    profiles.list.push(profile);
                }
                Err(message) => {
                    diags.push(Diagnostic::new(
                        FILE,
                        line,
                        format!("profile `{name}` is not usable: {message}. It is kept in the file but not offered"),
                    ));
                    profiles.preserved.insert(name, value);
                }
            }
        }
        (profiles, diags)
    }

    /// The file contents for these profiles.
    pub fn to_toml(&self) -> String {
        let mut table = toml::Table::new();
        for (name, value) in &self.preserved {
            table.insert(name.clone(), value.clone());
        }
        for profile in &self.list {
            table.insert(profile.name.clone(), toml::Value::Table(profile.to_table()));
        }
        format!("# Connection profiles. Passwords are never stored here.\n\n{table}")
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// Writes `contents` to `path` through a temporary file, so a crash cannot
/// leave a half-written file. The file and any new directory are private to
/// the owner (R2.6).
#[cfg(unix)]
pub fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::fs::{DirBuilder, OpenOptions};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let name = path
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// A message when `path` can be read by other users (R2.6).
#[cfg(unix)]
pub fn permission_problem(path: &Path) -> Option<Diagnostic> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode();
    (mode & 0o077 != 0).then(|| {
        Diagnostic::new(
            FILE,
            None,
            format!(
                "{} can be read by other users. Run: chmod 600 {}",
                path.display(),
                path.display()
            ),
        )
    })
}

// ---------------------------------------------------------------------------
// Connection URIs
// ---------------------------------------------------------------------------

/// A connection given as a URI (R1.5). It is used once and never saved.
#[derive(Debug, Clone, PartialEq)]
pub struct UriConnection {
    pub profile: Profile,
    pub password: Option<Secret>,
}

/// Parses `postgres://user:password@host:port/database?sslmode=require`.
///
/// Error messages describe the problem without quoting any part of the
/// input, because the input can contain a password.
pub fn parse_uri(uri: &str) -> Result<UriConnection, String> {
    let rest = uri
        .strip_prefix("postgresql://")
        .or_else(|| uri.strip_prefix("postgres://"))
        .ok_or("a connection URI starts with postgres:// or postgresql://")?;
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));

    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let mut profile = Profile::new("(uri)");
    let mut password = None;
    if let Some(info) = userinfo {
        let (user, pass) = match info.split_once(':') {
            Some((u, p)) => (u, Some(p)),
            None => (info, None),
        };
        if !user.is_empty() {
            profile.user = Some(decode(user).ok_or("the user name has invalid % escapes")?);
        }
        if let Some(pass) = pass {
            password = Some(Secret::new(
                decode(pass).ok_or("the password has invalid % escapes")?,
            ));
        }
    }

    if hostport.contains(',') {
        return Err("only one host is supported".to_string());
    }
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let (addr, after) = v6
            .split_once(']')
            .ok_or("the IPv6 host is missing its closing ]")?;
        (addr, after.strip_prefix(':'))
    } else {
        match hostport.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    if !host.is_empty() {
        profile.host = decode(host).ok_or("the host has invalid % escapes")?;
    }
    if let Some(port) = port.filter(|p| !p.is_empty()) {
        profile.port = port
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or("the port must be a number from 1 to 65535")?;
    }
    if !path.is_empty() {
        profile.database = Some(decode(path).ok_or("the database name has invalid % escapes")?);
    }

    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = decode(value).ok_or("a parameter has invalid % escapes")?;
        match key {
            "sslmode" => {
                profile.ssl_mode = SslMode::from_name(&value)
                    .ok_or("sslmode must be disable, prefer, require, verify-ca or verify-full")?;
            }
            "sslrootcert" => profile.ssl_root_cert = Some(value),
            // Not quoted: the input may contain a password in any position.
            _ => {
                return Err("only the sslmode and sslrootcert parameters are supported".to_string())
            }
        }
    }
    profile.validate()?;
    Ok(UriConnection { profile, password })
}

/// Decodes `%XX` escapes. `None` when an escape is malformed or the result is
/// not UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(src: &str) -> Profiles {
        let (profiles, diags) = Profiles::parse(src);
        assert!(diags.is_empty(), "{diags:?}");
        profiles
    }

    fn full() -> Profile {
        let mut p = Profile::new("prod");
        p.host = "db.example.com".to_string();
        p.port = 6432;
        p.database = Some("app".to_string());
        p.user = Some("alice".to_string());
        p.ssl_mode = SslMode::VerifyFull;
        p.ssl_root_cert = Some("/etc/ssl/ca.pem".to_string());
        p.label = Some("Production".to_string());
        p.color = Some("#ff0000".to_string());
        p.production = true;
        p.password_env = Some("PROD_PW".to_string());
        p.last_used = Some(1_700_000_000);
        p
    }

    // -- profile --------------------------------------------------------------

    #[test]
    fn new_profiles_have_libpq_defaults() {
        let p = Profile::new("x");
        assert_eq!(
            (p.host.as_str(), p.port, p.ssl_mode),
            ("localhost", 5432, SslMode::Prefer)
        );
        assert!(!p.production);
        assert_eq!(p.display_name(), "x");
    }

    #[test]
    fn label_replaces_the_name_for_display() {
        assert_eq!(full().display_name(), "Production");
        let mut p = Profile::new("x");
        p.label = Some(String::new());
        assert_eq!(p.display_name(), "x");
    }

    #[test]
    fn target_falls_back_to_the_os_user_and_a_same_named_database() {
        let t = Profile::new("x").target("sachin");
        assert_eq!((t.user.as_str(), t.database.as_str()), ("sachin", "sachin"));
        let t = full().target("sachin");
        assert_eq!(
            (t.user.as_str(), t.database.as_str(), t.env_var.as_deref()),
            ("alice", "app", Some("PROD_PW"))
        );
    }

    #[test]
    fn validation() {
        assert!(full().validate().is_ok());
        type Change = fn(&mut Profile);
        let cases: [(&str, Change); 7] = [
            ("name", |p| p.name = "  ".to_string()),
            ("control", |p| p.name = "a\nb".to_string()),
            ("host", |p| p.host = String::new()),
            ("port", |p| p.port = 0),
            ("color", |p| p.color = Some("nope".to_string())),
            ("env var", |p| p.password_env = Some("A-B".to_string())),
            ("env var digit", |p| p.password_env = Some("1A".to_string())),
        ];
        for (what, change) in cases {
            let mut p = full();
            change(&mut p);
            assert!(p.validate().is_err(), "{what}");
        }
    }

    #[test]
    fn ssl_mode_names_round_trip() {
        for mode in [
            SslMode::Disable,
            SslMode::Prefer,
            SslMode::Require,
            SslMode::VerifyCa,
            SslMode::VerifyFull,
        ] {
            assert_eq!(SslMode::from_name(mode.name()), Some(mode));
        }
        assert_eq!(SslMode::from_name("allow"), None);
    }

    // -- file format ----------------------------------------------------------

    #[test]
    fn a_minimal_profile_gets_defaults() {
        let profiles = parsed("[dev]\nhost = \"localhost\"\n");
        let p = profiles.get("dev").expect("dev");
        assert_eq!(
            (p.port, p.ssl_mode, p.production),
            (5432, SslMode::Prefer, false)
        );
    }

    #[test]
    fn profiles_round_trip_through_the_file_format() {
        let mut profiles = Profiles::default();
        profiles.add(full()).expect("add");
        profiles.add(Profile::new("dev")).expect("add");
        let text = profiles.to_toml();
        assert_eq!(parsed(&text), profiles);
        assert!(
            !text.to_lowercase().contains("password ="),
            "no password field exists"
        );
    }

    #[test]
    fn names_with_dots_and_spaces_survive() {
        let mut profiles = Profiles::default();
        profiles.add(Profile::new("my.db 1")).expect("add");
        assert_eq!(
            parsed(&profiles.to_toml())
                .get("my.db 1")
                .map(|p| p.name.as_str()),
            Some("my.db 1")
        );
    }

    #[test]
    fn unusable_profiles_are_reported_and_kept_on_save() {
        let src = "[good]\nhost = \"a\"\n[bad]\nport = 99999\n";
        let (mut profiles, diags) = Profiles::parse(src);
        assert_eq!(profiles.len(), 1);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("`bad` is not usable"));
        assert!(diags[0].message.contains("port"));
        assert_eq!(diags[0].line, Some(3));

        profiles.add(Profile::new("new")).expect("add");
        let saved = profiles.to_toml();
        assert!(
            saved.contains("[bad]") && saved.contains("99999"),
            "{saved}"
        );
        assert!(
            profiles.add(Profile::new("bad")).is_err(),
            "a kept entry still owns its name"
        );
    }

    #[test]
    fn a_top_level_value_that_is_not_a_table_is_kept() {
        let (profiles, diags) = Profiles::parse("version = 2\n[a]\nhost = \"h\"\n");
        assert_eq!(diags.len(), 1);
        assert!(profiles.to_toml().contains("version = 2"));
    }

    #[test]
    fn unknown_options_are_reported_and_survive_an_edit() {
        let (mut profiles, diags) = Profiles::parse("[a]\nhost = \"h\"\nfuture_option = 7\n");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("future_option"));
        let mut edited = profiles.get("a").cloned().expect("a");
        edited.host = "other".to_string();
        profiles.update("a", edited).expect("update");
        assert!(profiles.to_toml().contains("future_option = 7"));
    }

    #[test]
    fn a_bad_color_is_dropped_but_the_profile_stays() {
        let (profiles, diags) = Profiles::parse("[a]\nhost = \"h\"\ncolor = \"plaid\"\n");
        assert_eq!(profiles.get("a").and_then(|p| p.color.clone()), None);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn wrong_types_make_the_profile_unusable() {
        for body in [
            "host = 5",
            "port = \"5432\"",
            "port = 0",
            "ssl_mode = \"sometimes\"",
            "production = \"yes\"",
            "password_env = \"not valid\"",
        ] {
            let (profiles, diags) = Profiles::parse(&format!("[a]\n{body}\n"));
            assert!(profiles.is_empty(), "{body}");
            assert_eq!(diags.len(), 1, "{body}");
        }
    }

    #[test]
    fn a_syntax_error_yields_no_profiles_and_a_line() {
        let (profiles, diags) = Profiles::parse("[a\n");
        assert!(profiles.is_empty());
        assert_eq!(diags[0].line, Some(1));
    }

    // -- editing --------------------------------------------------------------

    #[test]
    fn add_rejects_duplicates_and_invalid_profiles() {
        let mut profiles = Profiles::default();
        profiles.add(Profile::new("a")).expect("add");
        assert!(profiles.add(Profile::new("a")).is_err());
        let mut bad = Profile::new("b");
        bad.port = 0;
        assert!(profiles.add(bad).is_err());
        assert_eq!(profiles.len(), 1);
    }

    #[test]
    fn update_edits_and_renames() {
        let mut profiles = Profiles::default();
        profiles.add(Profile::new("a")).expect("add");
        profiles.add(Profile::new("b")).expect("add");
        let mut changed = Profile::new("c");
        changed.host = "new-host".to_string();
        profiles.update("a", changed).expect("rename");
        assert!(profiles.get("a").is_none());
        assert_eq!(profiles.get("c").map(|p| p.host.as_str()), Some("new-host"));
        assert!(
            profiles.update("c", Profile::new("b")).is_err(),
            "cannot rename onto another profile"
        );
        assert!(profiles.update("zzz", Profile::new("q")).is_err());
        assert!(
            profiles.get("c").is_some(),
            "a failed update changes nothing"
        );
    }

    #[test]
    fn duplicate_copies_everything_but_the_history() {
        let mut profiles = Profiles::default();
        profiles.add(full()).expect("add");
        profiles.duplicate("prod", "prod-copy").expect("duplicate");
        let copy = profiles.get("prod-copy").expect("copy");
        assert_eq!(
            (copy.host.as_str(), copy.production, copy.last_used),
            ("db.example.com", true, None)
        );
        assert!(profiles.duplicate("prod", "prod-copy").is_err());
        assert!(profiles.duplicate("none", "x").is_err());
    }

    #[test]
    fn delete_removes_only_that_profile() {
        let mut profiles = Profiles::default();
        profiles.add(Profile::new("a")).expect("add");
        profiles.add(Profile::new("b")).expect("add");
        profiles.delete("a").expect("delete");
        assert_eq!(profiles.len(), 1);
        assert!(profiles.delete("a").is_err());
    }

    #[test]
    fn the_picker_lists_recent_profiles_first() {
        let mut profiles = Profiles::default();
        for name in ["delta", "alpha", "charlie", "bravo"] {
            profiles.add(Profile::new(name)).expect("add");
        }
        let names = |p: &Profiles| {
            p.for_picker()
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&profiles), ["alpha", "bravo", "charlie", "delta"]);
        profiles.touch("charlie", 100);
        profiles.touch("delta", 200);
        assert_eq!(names(&profiles), ["delta", "charlie", "alpha", "bravo"]);
        profiles.touch("nobody", 300);
    }

    // -- files ----------------------------------------------------------------

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("oxpecker-profile-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn saved_files_and_new_directories_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("private");
        let path = dir.join("nested").join(FILE);
        write_private(&path, "hello\n").expect("write");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "hello\n");
        let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().expect("parent")), 0o700);
        assert!(permission_problem(&path).is_none());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn saving_replaces_the_file_and_leaves_no_temporary_files() {
        let dir = temp_dir("replace");
        let path = dir.join(FILE);
        write_private(&path, "one").expect("first");
        write_private(&path, "two").expect("second");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "two");
        let entries: Vec<_> = std::fs::read_dir(&dir).expect("dir").collect();
        assert_eq!(entries.len(), 1);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_file_other_users_can_read_is_flagged() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("open");
        let path = dir.join(FILE);
        write_private(&path, "x").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let problem = permission_problem(&path).expect("problem");
        assert!(problem.message.contains("chmod 600"));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    // -- URIs -----------------------------------------------------------------

    #[test]
    fn a_full_uri() {
        let c = parse_uri("postgresql://alice:s3cret@db.example.com:6432/app?sslmode=verify-full&sslrootcert=%2Fetc%2Fca.pem")
            .expect("parse");
        let p = &c.profile;
        assert_eq!(p.user.as_deref(), Some("alice"));
        assert_eq!((p.host.as_str(), p.port), ("db.example.com", 6432));
        assert_eq!(p.database.as_deref(), Some("app"));
        assert_eq!(p.ssl_mode, SslMode::VerifyFull);
        assert_eq!(p.ssl_root_cert.as_deref(), Some("/etc/ca.pem"));
        assert_eq!(c.password.as_ref().map(Secret::expose), Some("s3cret"));
    }

    #[test]
    fn uri_parts_are_optional() {
        let c = parse_uri("postgres://").expect("parse");
        assert_eq!(
            (c.profile.host.as_str(), c.profile.port),
            ("localhost", 5432)
        );
        assert!(c.password.is_none() && c.profile.user.is_none() && c.profile.database.is_none());
        let c = parse_uri("postgres://host/db").expect("parse");
        assert_eq!(
            (c.profile.host.as_str(), c.profile.database.as_deref()),
            ("host", Some("db"))
        );
        let c = parse_uri("postgres://bob@host").expect("parse");
        assert_eq!(c.profile.user.as_deref(), Some("bob"));
        assert!(c.password.is_none());
    }

    #[test]
    fn uri_escapes_and_special_characters_in_the_password() {
        let c = parse_uri("postgres://u:p%40ss%3Aw%2Fd%25@h/d").expect("parse");
        assert_eq!(c.password.as_ref().map(Secret::expose), Some("p@ss:w/d%"));
        // An unescaped @ in the password: the last @ separates the host.
        let c = parse_uri("postgres://u:a@b@h/d").expect("parse");
        assert_eq!(c.password.as_ref().map(Secret::expose), Some("a@b"));
        assert_eq!(c.profile.host, "h");
    }

    #[test]
    fn uri_ipv6_hosts() {
        let c = parse_uri("postgres://[::1]:5433/d").expect("parse");
        assert_eq!((c.profile.host.as_str(), c.profile.port), ("::1", 5433));
        let c = parse_uri("postgres://[fe80::1]/d").expect("parse");
        assert_eq!((c.profile.host.as_str(), c.profile.port), ("fe80::1", 5432));
        assert!(parse_uri("postgres://[::1/d").is_err());
    }

    #[test]
    fn bad_uris_are_rejected() {
        for bad in [
            "mysql://h/d",
            "h/d",
            "postgres://h:0/d",
            "postgres://h:99999/d",
            "postgres://h:abc/d",
            "postgres://a,b/d",
            "postgres://u:%zz@h/d",
            "postgres://h/d?sslmode=maybe",
            "postgres://h/d?application_name=x",
            "postgres://u:%ff@h/d",
        ] {
            assert!(parse_uri(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn uri_errors_never_quote_the_input() {
        for bad in [
            "postgres://alice:topsecret@h:0/d",
            "postgres://alice:topsecret@h:99999/d",
            "postgres://alice:top%zzsecret@h/d",
            "postgres://alice:topsecret@a,b/d",
            "postgres://alice:topsecret@h/d?sslmode=topsecret",
            "postgres://alice:topsecret@h/d?topsecret=1&sslmode=x",
            "mysql://alice:topsecret@h/d",
        ] {
            let message = parse_uri(bad).expect_err(bad);
            assert!(
                !message.contains("topsecret") && !message.contains("alice"),
                "{bad}: {message}"
            );
        }
    }

    #[test]
    fn a_parsed_uri_does_not_print_its_password() {
        let c = parse_uri("postgres://alice:topsecret@h/d").expect("parse");
        assert!(!format!("{c:?}").contains("topsecret"));
    }
}
