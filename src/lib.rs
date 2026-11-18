//! pgpeck: a terminal client for PostgreSQL.
//!
//! The binary in `main.rs` is a thin shell around this library, which lets
//! integration tests use the same code.

pub mod app;
pub mod catalog;
pub mod clipboard;
pub mod command;
pub mod complete;
pub mod config;
pub mod db;
pub mod diag;
pub mod diagram;
pub mod editor;
pub mod events;
pub mod form;
pub mod fuzzy;
pub mod history;
pub mod keymap;
pub mod layout;
pub mod logging;
pub mod paths;
pub mod peck;
pub mod picker;
pub mod profile;
pub mod relations;
pub mod results;
pub mod runtime;
pub mod secrets;
pub mod sql;
pub mod terminal;
pub mod theme;
pub mod ui;
