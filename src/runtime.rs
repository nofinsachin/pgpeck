//! Runs `Command`s in the background and reports back with events. The UI
//! never waits on any of this (steering: the UI thread never blocks on I/O).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::UnboundedSender;

use crate::command::Command;
use crate::db::{self, ConnId, Session};
use crate::events::{Background, Event};
use crate::profile::{write_private, Profile};
use crate::secrets::{self, Environment, Outcome, Source};

#[derive(Clone)]
pub struct Runtime {
    events: UnboundedSender<Event>,
    sessions: Arc<Mutex<HashMap<ConnId, Session>>>,
    password_sources: Arc<Vec<Source>>,
    row_limit: usize,
    profiles_path: Option<PathBuf>,
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Login name used when a profile does not name a user.
pub fn default_user() -> String {
    ["PGUSER", "USER", "LOGNAME"]
        .iter()
        .find_map(|v| env_var(v).filter(|u| !u.is_empty()))
        .unwrap_or_else(|| "postgres".to_string())
}

impl Runtime {
    pub fn new(
        events: UnboundedSender<Event>,
        password_sources: Vec<Source>,
        row_limit: usize,
        profiles_path: Option<PathBuf>,
    ) -> Self {
        Self {
            events,
            sessions: Arc::default(),
            password_sources: Arc::new(password_sources),
            row_limit,
            profiles_path,
        }
    }

    fn send(&self, background: Background) {
        // The receiver is gone only while the application is shutting down.
        let _ = self.events.send(Event::Background(background));
    }

    fn session(&self, conn: ConnId) -> Option<Session> {
        self.sessions.lock().ok()?.get(&conn).cloned()
    }

    /// Starts the work. Must be called inside a Tokio runtime.
    pub fn execute(&self, command: Command) {
        let this = self.clone();
        match command {
            Command::Connect {
                conn,
                profile,
                password,
                lookup,
            } => {
                tokio::spawn(async move { this.connect(conn, *profile, password, lookup).await });
            }
            Command::Run {
                conn,
                doc,
                sql,
                paging,
            } => {
                tokio::spawn(async move {
                    let output = match this.session(conn) {
                        Some(session) => db::run(&session, &sql, this.row_limit, &paging).await,
                        None => db::QueryOutput {
                            sets: Vec::new(),
                            error: Some(db::DbError::new(
                                "the connection is closed. Reconnect and try again",
                            )),
                            elapsed: std::time::Duration::ZERO,
                            tx: db::TxState::Idle,
                        },
                    };
                    this.send(Background::QueryFinished { conn, doc, output });
                });
            }
            Command::FetchPage {
                conn,
                doc,
                cursor,
                start,
                count,
            } => {
                tokio::spawn(async move {
                    let result = match this.session(conn) {
                        Some(session) => db::fetch_page(&session, &cursor, start, count)
                            .await
                            .map(|(_, rows)| rows),
                        None => Err(db::DbError::new("the connection is closed")),
                    };
                    this.send(Background::PageLoaded {
                        conn,
                        doc,
                        cursor,
                        start,
                        result,
                    });
                });
            }
            Command::Stats { conn } => {
                tokio::spawn(async move {
                    let result = match this.session(conn) {
                        Some(session) => db::stats(&session).await,
                        None => Err(db::DbError::new("the connection is closed")),
                    };
                    this.send(Background::Stats { conn, result });
                });
            }
            Command::Catalog { conn } => {
                tokio::spawn(async move {
                    let result = match this.session(conn) {
                        Some(session) => db::catalog(&session).await,
                        None => Err(db::DbError::new("the connection is closed")),
                    };
                    this.send(Background::Catalog { conn, result });
                });
            }
            Command::Cancel { conn } => {
                tokio::spawn(async move {
                    if let Some(session) = this.session(conn) {
                        if let Err(e) = db::cancel(&session).await {
                            tracing::warn!(error = %e.message, "cancel request failed");
                        }
                    }
                });
            }
            Command::Disconnect { conn } => {
                // Dropping the session closes the connection.
                if let Ok(mut sessions) = this.sessions.lock() {
                    sessions.remove(&conn);
                }
            }
            Command::SaveProfiles { contents } => {
                let Some(path) = this.profiles_path.clone() else {
                    this.send(Background::ProfilesSaved(Err(
                        "no configuration directory".to_string()
                    )));
                    return;
                };
                tokio::spawn(async move {
                    let result =
                        tokio::task::spawn_blocking(move || write_private(&path, &contents))
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r.map_err(|e| e.to_string()));
                    this.send(Background::ProfilesSaved(result));
                });
            }
        }
    }

    async fn connect(
        &self,
        conn: ConnId,
        profile: Profile,
        password: Option<secrets::Secret>,
        lookup: bool,
    ) {
        let user = profile.user.clone().unwrap_or_else(default_user);
        let password = if lookup {
            match self.find_password(&profile).await {
                Lookup::Found(secret) => Some(secret),
                Lookup::None => None,
                Lookup::Ask(notes) => {
                    self.send(Background::PasswordNeeded { conn, notes });
                    return;
                }
            }
        } else {
            password
        };

        let events = self.events.clone();
        let on_close = move |reason: String| {
            let _ = events.send(Event::Background(Background::ConnectionClosed {
                conn,
                reason,
            }));
        };
        match db::connect(&profile, &user, password.as_ref(), on_close).await {
            Ok((session, info)) => {
                if let Ok(mut sessions) = self.sessions.lock() {
                    sessions.insert(conn, session);
                }
                self.send(Background::Connected { conn, info });
            }
            Err(error) => self.send(Background::ConnectFailed { conn, error }),
        }
    }

    /// Searches the configured sources. Reading a file can block, so lookups
    /// run on a blocking thread.
    async fn find_password(&self, profile: &Profile) -> Lookup {
        let order = self.password_sources.clone();
        let target = profile.target(&default_user());
        let result = tokio::task::spawn_blocking(move || {
            let environment = Environment { env: &env_var };
            secrets::resolve(&order, &target, &environment)
        })
        .await;
        match result {
            Ok(resolution) => match resolution.outcome {
                Outcome::Found(secret, _) => Lookup::Found(secret),
                Outcome::NeedsPrompt => Lookup::Ask(resolution.notes),
                Outcome::NotFound => Lookup::None,
            },
            Err(_) => Lookup::None,
        }
    }
}

enum Lookup {
    Found(secrets::Secret),
    None,
    Ask(Vec<String>),
}
