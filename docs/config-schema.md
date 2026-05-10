# Configuration Schema

This document describes the configuration that the current Dumbgram TUI binary actually reads.
It intentionally does **not** list planned layout, keybinding, sync, or theme-file settings as supported options.

## Main configuration file

By default the app reads `config.toml` in the current working directory. Use `--config PATH` to load a different file:

```bash
dumbgram_tui --config ~/.config/dumbgram/config.toml
```

## Supported TOML schema

```toml
[telegram]
# Required. Get this from https://my.telegram.org.
# Accepts either an integer or a quoted numeric string.
api_id = 12345

# Required. Get this from https://my.telegram.org.
api_hash = "YOUR_API_HASH"

# Required. Telegram session file to create/reuse.
# Relative paths are resolved from the process working directory.
# ~/... is expanded from HOME.
session_file = "~/.config/dumbgram/session.dat"

# Backward-compatible alias accepted by the parser:
# session_path = "~/.config/dumbgram/session.dat"
```

Only `telegram.api_id`, `telegram.api_hash`, and `telegram.session_file` / `telegram.session_path` are currently used by the app.

## Validation rules

`dumbgram_tui --check-config --config PATH` validates without connecting to Telegram:

1. `telegram.api_id` must parse to a positive integer.
2. `telegram.api_hash` must be non-empty after trimming whitespace.
3. `telegram.session_file` / `telegram.session_path` must be non-empty after trimming whitespace.
4. If the resolved session-file parent path already exists, it must be a directory.
5. Missing session-file parent directories are allowed; real launch and `--check-auth` create them before opening the Grammers session.

## Session path behavior

- `session_file = "session.dat"` stores the session in the current working directory.
- `session_file = "~/.config/dumbgram/session.dat"` expands `~` using `HOME`.
- `session_path` is accepted as an alias for compatibility with earlier docs.
- Session files contain Telegram login state and must not be committed.

## Diagnostics

```bash
dumbgram_tui --check-config --config config.toml
dumbgram_tui --check-auth --config config.toml
```

- `--check-config` parses and validates the file only.
- `--check-auth` connects to Telegram and verifies that the saved session is authorized. It requires valid credentials, network access, and an existing authorized session.

## Not yet configurable

The current app still uses built-in behavior for UI layout, keybindings, theme colors, message fetch count, chat sorting, and sync/cache policy. Those may become config settings later, but adding them to `config.toml` today has no effect.
