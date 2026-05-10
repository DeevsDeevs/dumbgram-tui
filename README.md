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
- Unread counts are not currently populated.
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

Run the app:

```bash
devbox run run
```

On the first run, Dumbgram TUI prompts for your phone number, verification code, and 2FA password if your Telegram account requires one. After a successful login, the Telegram session is saved locally and reused on later runs.

## Development

Use Devbox for all project commands:

```bash
devbox run check       # Type-check the crate
devbox run build       # Build the binary
devbox run test        # Run tests
devbox run fmt         # Format Rust source
devbox run fmt:check   # Check Rust formatting
devbox run clippy      # Run Clippy
devbox run run         # Run the client
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
- `docs/config-schema.md` — expanded configuration schema notes.
- `AGENTS.md` — contributor and coding-agent guidance.
- `devbox.json` / `devbox.lock` — reproducible development environment.

## Controls

General navigation:

- `q` — quit.
- `Tab` — cycle focus through folders, chats, messages, and input.
- Arrow keys — move selection or focus depending on the active panel.
- `<` / `>` — adjust the split between chat list and message view.

Message actions:

- `Enter` in the input panel — send the current message.
- `e` on a selected message — edit the message when allowed.
- `r` on a selected message — reply to the message.
- `d` on a selected message — request deletion.
- `y` / `n` — confirm or cancel deletion.
- `Esc` — leave input mode or cancel pending actions.

Mouse support is available for selecting folders, chats, messages, and the input area.

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
