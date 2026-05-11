# Configuration Schema

This document describes the configuration that the current Dumbgram TUI binary actually reads.
It intentionally does **not** list planned layout, keybinding, sync, or theme-file settings as supported options.

## Main configuration file

By default the app reads `config.toml` from Dumbgram's app config directory:

- macOS/Linux: `${XDG_CONFIG_HOME:-~/.config}/dumbgram/config.toml`
- Windows: `%APPDATA%\\dumbgram\\config.toml`

Set `DUMBGRAM_CONFIG_HOME` to override the directory explicitly. Use `--config PATH` to load a different file:

```bash
dumbgram_tui --config ./config.toml
```

## Supported TOML schema

```toml
[telegram]
# Required. Get this from https://my.telegram.org.
# Accepts either an integer or a quoted numeric string.
api_id = 12345

# Required. Get this from https://my.telegram.org.
api_hash = "YOUR_API_HASH"

# Optional. Telegram session file to create/reuse.
# Defaults to "session.dat".
# Relative paths are resolved next to the loaded config.toml.
# ~/... is expanded from HOME.
session_file = "session.dat"

# Backward-compatible alias accepted by the parser:
# session_path = "~/.config/dumbgram/session.dat"
```

Only `telegram.api_id`, `telegram.api_hash`, and `telegram.session_file` / `telegram.session_path` are currently used by the app.

## Validation rules

`dumbgram_tui --check-config --config PATH` validates without connecting to Telegram:

1. `telegram.api_id` must parse to a positive integer.
2. `telegram.api_hash` must be non-empty after trimming whitespace.
3. `telegram.session_file` / `telegram.session_path`, when present, must be non-empty after trimming whitespace.
4. If the resolved session-file parent path already exists, it must be a directory.
5. Missing session-file parent directories are allowed; real launch and `--check-auth` create them before opening the Grammers session.

## Session path behavior

- Omitting `session_file` uses `session.dat` next to the loaded config file.
- `session_file = "session.dat"` also stores the session next to the loaded config file.
- `session_file = "~/.config/dumbgram/session.dat"` expands `~` using `HOME`.
- Absolute paths are used as-is.
- `session_path` is accepted as an alias for compatibility with earlier docs.
- Session files contain Telegram login state and must not be committed.

## Diagnostics

```bash
dumbgram_tui --check-config
dumbgram_tui --check-auth
```

- `--check-config` parses and validates the file only.
- `--check-auth` connects to Telegram and verifies that the saved session is authorized. It requires valid credentials, network access, and an existing authorized session.

## Local UI state

Dumbgram keeps runtime UI preferences out of `config.toml`. It stores preferences in `<config-stem>.state.toml` next to the loaded config file. Currently persisted values are:

```toml
[ui]
show_help_bar = true
split_ratio = 0.3
```

The `?` key updates `ui.show_help_bar`; `<` and `>` update `ui.split_ratio`. These `*.state.toml` files are local runtime state and should not be committed.

## Not yet configurable

The current app still uses built-in behavior for keybindings, theme colors, message fetch count, chat sorting, and sync/cache policy. Those may become config settings later, but adding them to `config.toml` today has no effect.
