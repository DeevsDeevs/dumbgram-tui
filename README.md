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
- Photo/image messages are shown with text placeholders such as `[photo]`. In Ghostty/Kitty-compatible terminals, selected downloaded thumbnails are also displayed with the Kitty graphics protocol. Full media browser controls, multi-account support, and server-side search are not implemented.
- Some Telegram delete updates do not include enough context to identify the chat precisely.

## Setup

Use Devbox:

```bash
devbox install
```

Create local config in Dumbgram's config directory:

```bash
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/dumbgram"
cp config.example.toml "${XDG_CONFIG_HOME:-$HOME/.config}/dumbgram/config.toml"
```

Dumbgram uses `~/.config/dumbgram/config.toml` by default on macOS/Linux. Set `DUMBGRAM_CONFIG_HOME` to override the directory explicitly.

Edit `config.toml`:

```toml
[telegram]
api_id = 12345
api_hash = "your_api_hash"
session_file = "session.dat"
```

Get `api_id` and `api_hash` from <https://my.telegram.org>. A relative `session_file` is resolved next to `config.toml`, so the example stores the session in the same Dumbgram config directory.

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
dumbgram_tui --check-config
dumbgram_tui --check-auth
dumbgram_tui
dumbgram_tui --config ./config.toml --log dumbgram.log
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

The bottom help bar shows available actions for the current focus. Press `?` outside input to hide or show it. Dumbgram stores that UI preference, along with the chat/message split width, in a local `*.state.toml` file next to your config.

Navigation:

- `Tab` — cycle focus: folders, chats, messages, input.
- Arrow keys — move selection; explicit Left/Right panel controls are shown in the help bar, and list boundaries stop instead of changing focus.
- `PageUp` / `PageDown` / `Home` / `End` — move through messages.
- `Up` or `PageUp` on the first loaded message — load older history.
- `<` / `>` — resize chat/message split outside input.
- `q` — quit outside input.

Chat actions:

- `/` — search loaded chats by name while the chat list is focused.
- Type in chat search — filter loaded chats; substring and simple fuzzy/subsequence matches are supported.
- `Up` / `Down` in chat search — browse matches without opening them.
- `Enter` — open the highlighted search result and leave chat search.
- `Esc` — clear chat search.

Message actions:

- `Enter` in messages — focus the input; `Enter` in input sends, saves an edit, or sends a reply when text is present.
- `e` — edit selected own editable message.
- `r` — reply to selected message.
- `d` — delete selected own deletable message, or dismiss a failed local send.
- `c` — copy selected message text using OSC52 clipboard support.
- `o` — open the first web link in the selected message.
- `s` — save selected downloadable media to Downloads.
- `v` — open the saved file for the selected message after saving it.
- Click a visible `http://` or `https://` link in a message to open it.
- `y` — confirm delete.
- `n`, `Esc`, `Ctrl-C` — cancel delete.
- `Esc` / `Ctrl-C` — leave input or cancel edit/reply where applicable.

Mouse:

- Click folders, chats, messages, or input to focus/select.
- Click a chat to load it; scroll the chat list without opening another conversation.
- Right-click a chat or message for its available actions. Use the mouse or `Up`/`Down` and `Enter`; `Esc` closes the menu.
- Drag the divider between chats and messages to resize the conversation panes.
- Scroll messages to move selection. Use `Up` or `PageUp` at the first loaded message to request older history.
- Mouse input is ignored while a delete confirmation prompt is open.

## Nix / Devbox install

This repository exposes a Nix flake package and app for `aarch64-darwin`, `x86_64-darwin`, `aarch64-linux`, and `x86_64-linux`:

```bash
nix run .#dumbgram_tui -- --help
nix build .#dumbgram_tui
```

For a global Devbox install from a local checkout or Git flake URL:

```bash
devbox global add path:$PWD#dumbgram_tui
```

## Development

```bash
devbox run fmt
devbox run fmt:check
devbox run check
devbox run clippy
devbox run test
devbox run smoke
```

Local runtime files such as repository-local `config.toml`, `session.dat`, `*.dat`, `*.state.toml`, `.devbox/`, and `target/` are ignored by Git. Do not commit Telegram credentials or session files.

## License

No license file is currently included.
