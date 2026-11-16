# oxpecker

A fast, keyboard-driven terminal client for PostgreSQL, written in Rust.

Status: early development. The screen layout, focus handling, keymap,
configuration loading and the SQL editor work. There is no database
connection yet.

## Build and run

```
cargo run
cargo run -- --check-config
```

## Configuration

Files live in `$XDG_CONFIG_HOME/oxpecker` (usually `~/.config/oxpecker`):

- `config.toml`: see [`config/default.toml`](config/default.toml) for every option.
- `keymap.toml`: laid over [`config/keymap.default.toml`](config/keymap.default.toml).
- `themes/<name>.toml`: see [`config/themes`](config/themes).

`oxpecker --check-config` validates all three and exits non-zero on problems.
Logs go to `$XDG_STATE_HOME/oxpecker` (set `OXPECKER_LOG=debug` for more).

## Development

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Screen snapshots live in `tests/snapshots`. Rewrite them with
`UPDATE_SNAPSHOTS=1 cargo test`. Requirements are in `.kiro/specs/requirements.md`.
