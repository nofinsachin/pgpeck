//! Oxpecker: a terminal client for PostgreSQL.
//!
//! The binary in `main.rs` is a thin shell around this library, which lets
//! integration tests use the same code.

pub mod app;
pub mod catalog;
pub mod command;
pub mod complete;
pub mod config;
pub mod db;
pub mod diag;
pub mod editor;
pub mod events;
pub mod form;
pub mod fuzzy;
pub mod keymap;
pub mod layout;
pub mod logging;
pub mod paths;
pub mod picker;
pub mod profile;
pub mod results;
pub mod runtime;
pub mod secrets;
pub mod sql;
pub mod terminal;
pub mod theme;
pub mod ui;
