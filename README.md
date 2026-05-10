# Dumbgram TUI

A small terminal Telegram client written in Rust with Ratatui/Crossterm and Grammers.

## What works

- Login with Telegram API credentials, phone code, and optional 2FA.
- Reuse a local Telegram session file.
- Browse folders, chats, and messages.
- Send, edit, reply to, and delete supported messages.
- Receive incoming, edited, deleted, and typing updates.
- Run without credentials using the mock backend.
- Run a mock-only smoke test for automated validation.

## Current limits

- Large accounts are loaded in bounded pages, but full chat pagination UI is still limited.
- Telegram folder names are best-effort from dialog filters/folder metadata; folder-rule editing is not implemented.
- Media, downloads, search, and multiple accounts are not implemented.
- Some Telegram delete updates do not include enough context to identify the chat precisely.

## Setup

Use Devbox:

```bash
devbox install
```

Create local config:

```bash
cp config.example.toml config.toml
```

Edit `config.toml`:

```toml
[telegram]
api_id = 12345
api_hash = "your_api_hash"
session_file = "session.dat"
```

Get `api_id` and `api_hash` from <https://my.telegram.org>. `session_file` may also use a `~/...` path.

## Run

```bash
devbox run run
```

First launch prompts for phone, login code, and 2FA password if needed. After login, the session is saved and reused.

Useful launch checks:

```bash
devbox run run:mock       # UI with built-in mock Telegram data
devbox run smoke          # mock-only render and interaction smoke test
devbox run check-config   # validate config.toml without connecting
devbox run check-auth     # verify saved Telegram session authorization
```

Direct CLI examples:

```bash
dumbgram_tui --help
dumbgram_tui --mock
dumbgram_tui --mock --smoke
dumbgram_tui --check-config --config config.toml
dumbgram_tui --check-auth --config config.toml
dumbgram_tui --config config.toml
dumbgram_tui --config config.toml --log dumbgram.log
```

`--smoke` is mock-only and never uses real Telegram.

## Diagnostics

When reproducing a freeze or real-account bug, launch with logging:

```bash
devbox run -- cargo run -- --config config.toml --log dumbgram.log
```

Then share the tail around the issue:

```bash
tail -n 100 dumbgram.log
```

The log records timings, counts, selected IDs, and UI events. It does not log message text, typed input, API hashes, phone numbers, or login codes. Local `*.log` files are ignored by git.

## Controls

The bottom help bar shows available actions for the current focus. Press `?` outside input to hide or show it.

Navigation:

- `Tab` — cycle focus: folders, chats, messages, input.
- Arrow keys — move selection or focus depending on the active panel.
- `PageUp` / `PageDown` / `Home` / `End` — move through messages.
- `Up` or `PageUp` on the first loaded message — load older history.
- `<` / `>` — resize chat/message split outside input.
- `q` — quit outside input.

Message actions:

- `Enter` in input — send, save edit, or send reply when text is present.
- `e` — edit selected own editable message.
- `r` — reply to selected message.
- `d` — delete selected own deletable message, or dismiss a failed local send.
- `y` — confirm delete.
- `n`, `Esc`, `Ctrl-C` — cancel delete.
- `Esc` / `Ctrl-C` — leave input or cancel edit/reply where applicable.

Mouse:

- Click folders, chats, messages, or input to focus/select.
- Click a chat to load it. Chat-list scrolling only focuses the chat list; use keyboard arrows or clicks to change chats.
- Scroll messages to move selection. Use `Up` or `PageUp` at the first loaded message to request older history.
- Mouse input is ignored while a delete confirmation prompt is open.

## Development

```bash
devbox run fmt
devbox run fmt:check
devbox run check
devbox run clippy
devbox run test
devbox run smoke
```

Local runtime files such as `config.toml`, `session.dat`, `*.dat`, `.devbox/`, and `target/` are ignored by Git. Do not commit Telegram credentials or session files.

## License

No license file is currently included.
