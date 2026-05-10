# Dumbgram TUI

Dumbgram TUI is a terminal Telegram client written in Rust. It focuses on a small, keyboard-friendly interface for reading chats and performing common message actions from the terminal.

The project currently provides a working Telegram integration through the Grammers client libraries and a Ratatui/Crossterm terminal UI.

## Features

- Telegram login with phone code and optional 2FA password.
- Session persistence through a local `session.dat` file.
- Chat list and message view backed by real Telegram data.
- Send, edit, reply to, and delete messages.
- Real-time update handling for incoming, edited, and deleted messages.
- Keyboard and mouse navigation.
- Configurable Telegram credentials through `config.toml`.

## Status

This is early-stage software. The main chat and message flows work, but some Telegram and UI capabilities are intentionally limited while the client remains small and maintainable.

Known limitations:

- Custom Telegram folders are not fully implemented; the primary view is the combined chat list.
- Unread counts are populated from Telegram dialog metadata and local updates, but custom folder support is still limited.
- Media preview, downloads, search, and multiple account support are not implemented.
- Message deletion updates may not include enough Telegram-side context to identify the chat in every case.

## Requirements

- [Devbox](https://www.jetify.com/devbox/) for the development environment.
- Telegram API credentials from <https://my.telegram.org>.

The repository includes `devbox.json` and `devbox.lock` so contributors can use the same Rust tooling without installing it globally.

## Quick start

Install the development environment:

```bash
devbox install
```

Create a local config file:

```bash
cp config.example.toml config.toml
```

Edit `config.toml` with your Telegram API credentials:

```toml
[telegram]
api_id = 12345
api_hash = "your_api_hash"
session_file = "session.dat"
```

`session_file` also accepts `~/...` paths and the backward-compatible key name `session_path`.

Run the app:

```bash
devbox run run
```

On the first run, Dumbgram TUI prompts for your phone number, verification code, and 2FA password if your Telegram account requires one. After a successful login, the Telegram session is saved locally and reused on later runs.

To launch the UI without Telegram credentials, use the built-in mock backend:

```bash
devbox run run:mock
```

To run the credential-free mock-only smoke check used for automated validation:

```bash
devbox run smoke
```

To explicitly connect and verify that the saved Telegram session is authorized without entering the TUI or starting login:

```bash
devbox run check-auth
```

## Development

Use Devbox for all project commands:

```bash
devbox run check       # Type-check the crate
devbox run build       # Build the binary
devbox run test        # Run tests
devbox run fmt         # Format Rust source
devbox run fmt:check   # Check Rust formatting
devbox run clippy      # Run Clippy
devbox run run         # Run the real Telegram client
devbox run run:mock    # Run the UI with built-in mock data
devbox run smoke       # Render mock UI off-screen and exercise keyboard/mouse interactions
devbox run check-config # Validate config.toml without connecting to Telegram
devbox run check-auth   # Connect and verify the saved Telegram session without entering the TUI
```

You can also enter the environment interactively:

```bash
devbox shell
```

Minimum validation before submitting code changes:

```bash
devbox run fmt:check
devbox run check
devbox run clippy
```

## Project layout

```text
src/
├── main.rs             # Startup, login flow, terminal setup, event loop
├── app.rs              # Application container and mode management
├── state.rs            # UI state, focus, selection, input, transient errors
├── config/             # TOML config loading and theme defaults
├── telegram/           # Telegram trait, Grammers client, mock client, data types
└── ui/                 # Ratatui layout and widgets
```

Additional files:

- `config.example.toml` — safe starter config.
- `docs/config-schema.md` — implemented configuration schema and validation notes.
- `AGENTS.md` — contributor and coding-agent guidance.
- `devbox.json` / `devbox.lock` — reproducible development environment.

## Controls

The bottom help bar shows the current focus and the most relevant shortcuts while the app is running.

Command-line options:

```bash
dumbgram_tui --help
dumbgram_tui --mock
dumbgram_tui --mock --smoke   # --smoke always uses mock data and never real Telegram
dumbgram_tui --check-config --config path/to/config.toml
dumbgram_tui --check-auth --config path/to/config.toml
dumbgram_tui --config path/to/config.toml
```

General navigation:

- `q` — quit.
- `Tab` — cycle focus through folders, chats, messages, and input, including while typing a draft.
- Plain unsent drafts are kept per chat while the app is running and restored after cancelling edit/reply mode.
- Arrow keys — move selection or focus depending on the active panel.
- `PageUp` / `PageDown` / `Home` / `End` — jump through the message list when messages are focused.
- `<` / `>` — adjust the split between chat list and message view.

Message actions:

- `Enter` in the input panel — send the current message.
- `e` on a selected message — edit the message when allowed.
- `r` on a selected message — reply to the message.
- `d` on a selected message — request deletion; on a failed local send, dismiss the failed row without contacting Telegram.
- `y` / `n` — confirm or cancel deletion; mouse navigation is ignored while the confirmation prompt is open.
- `Esc` / `Ctrl-C` — leave input mode or cancel pending actions.

Mouse support is available for selecting folders, chats, messages, and the input area. Mouse wheel scrolling over the chat list selects/loads chats; scrolling over the message panel moves the selected message. Chat and message panel titles show the current position as `selected/total`; the message title also shows selected-chat typing status when Telegram reports it. Successful message actions show a short status banner, while failures show an error banner.

## Configuration and secrets

Local runtime files are intentionally ignored by Git:

- `config.toml`
- `session.dat`
- `*.dat`
- `target/`
- `.devbox/`

Do not commit Telegram API credentials or session files. If a session file is exposed, revoke the session from Telegram and regenerate local credentials as needed.

## License

No license file is currently included. Add one before distributing or accepting external contributions.
