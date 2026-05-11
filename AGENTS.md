# Agent Guide

## Repository overview

Dumbgram TUI is a Rust terminal client for Telegram. It uses:

- `grammers-client` and `grammers-session` for Telegram authentication, sessions, chat history, message sends, edits, replies, deletes, and updates.
- `ratatui` and `crossterm` for the terminal interface, input events, mouse support, and alternate-screen rendering.
- `tokio` for async runtime and update handling.
- TOML configuration loaded from the app config directory by default, or from `--config PATH`.

The binary crate is `dumbgram_tui` and the entry point is `src/main.rs`.

## Development environment

Use Devbox for all local development commands. The Devbox environment installs the Rust toolchain, formatter, Clippy, rust-analyzer, and `pkg-config`.

```bash
devbox install
```

Run commands through Devbox instead of relying on host-installed tools:

```bash
devbox run check       # cargo check
devbox run build       # cargo build
devbox run test        # cargo test
devbox run fmt         # cargo fmt
devbox run fmt:check   # cargo fmt -- --check
devbox run clippy      # cargo clippy --all-targets --all-features
devbox run run         # cargo run
devbox run run:mock    # cargo run -- --mock
devbox run smoke       # cargo run -- --mock --smoke
devbox run check-config # cargo run -- --check-config
devbox run check-auth   # cargo run -- --check-auth
```

For an interactive shell:

```bash
devbox shell
```

## Running the app

1. Copy the sample config:

   ```bash
   mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/dumbgram"
   cp config.example.toml "${XDG_CONFIG_HOME:-$HOME/.config}/dumbgram/config.toml"
   ```

   On macOS/Linux this defaults to `~/.config/dumbgram/config.toml`, or pass `--config PATH`.

2. Fill in Telegram credentials from <https://my.telegram.org>:

   ```toml
   [telegram]
   api_id = 12345
   api_hash = "your_api_hash"
   session_file = "session.dat"
   ```

3. Start the real Telegram client:

   ```bash
   devbox run run
   ```

   Or launch the UI with built-in mock data, which does not require credentials:

   ```bash
   devbox run run:mock
   ```

   For automated credential-free validation, render the mock-only UI off-screen and exercise keyboard/mouse interactions:

   ```bash
   devbox run smoke
   ```

   Validate real Telegram configuration without connecting or entering the TUI:

   ```bash
   devbox run check-config
   ```

   Explicitly connect and verify that the saved Telegram session is authorized without entering the TUI or starting login:

   ```bash
   devbox run check-auth
   ```

On the first real Telegram run, the app prompts for phone number, login code, and a 2FA password when required. The session is saved to `session.dat` for later runs.

## Important files

- `src/main.rs` — application startup, Telegram login flow, event loop, input dispatch.
- `src/app.rs` — high-level application container and mode management.
- `src/state.rs` — UI state, selection, focus, input, and transient errors.
- `src/telegram/` — Telegram abstraction, real Grammers client, mock client, shared data types.
- `src/ui/` — Ratatui widgets for folders, chats, messages, input, and layout.
- `src/config/` — config loading and theme defaults.
- `config.example.toml` — safe example config for the app config directory.
- `docs/config-schema.md` — larger planned configuration schema notes.
- `devbox.json` and `devbox.lock` — reproducible development environment.

## Safety and repo hygiene

- Do not commit `config.toml`, `session.dat`, `*.dat`, `target/`, or `.devbox/`.
- Treat Telegram API credentials and session files as secrets.
- Prefer small, focused changes and verify with `devbox run check` at minimum.
- Use `devbox run run:mock` for manual credential-free UI smoke testing.
- Use `devbox run smoke` for automated credential-free render and keyboard/mouse interaction validation, including the on-screen controls help bar, status/error banners, chat/message position indicators, selected-chat typing indicator, per-chat drafts, unread reconciliation, mouse wheel chat/message scrolling, delete-confirmation mouse blocking, and edit-cancel draft restoration. `--smoke` is mock-only and must not touch real Telegram data.
- Use `devbox run check-config` before real Telegram runs to validate local credentials and session path without network access.
- Use `devbox run check-auth` only when explicitly opting into a Telegram connection; it verifies an existing saved session without starting login or the TUI.
- If changing Rust source, run `devbox run fmt` before final validation.
- Keep README and docs factual. Avoid claiming unreleased features as stable.
