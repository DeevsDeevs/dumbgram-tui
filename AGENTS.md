# Agent Guide

## Repository overview

Dumbgram TUI is a Rust terminal client for Telegram. It uses:

- `grammers-client` and `grammers-session` for Telegram authentication, sessions, chat history, message sends, edits, replies, deletes, and updates.
- `ratatui` and `crossterm` for the terminal interface, input events, mouse support, and alternate-screen rendering.
- `tokio` for async runtime and update handling.
- TOML configuration loaded from `config.toml` at the repository root.

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
```

For an interactive shell:

```bash
devbox shell
```

## Running the app

1. Copy the sample config:

   ```bash
   cp config.example.toml config.toml
   ```

2. Fill in Telegram credentials from <https://my.telegram.org>:

   ```toml
   [telegram]
   api_id = 12345
   api_hash = "your_api_hash"
   session_file = "session.dat"
   ```

3. Start the client:

   ```bash
   devbox run run
   ```

On the first run, the app prompts for phone number, login code, and a 2FA password when required. The session is saved to `session.dat` for later runs.

## Important files

- `src/main.rs` — application startup, Telegram login flow, event loop, input dispatch.
- `src/app.rs` — high-level application container and mode management.
- `src/state.rs` — UI state, selection, focus, input, and transient errors.
- `src/telegram/` — Telegram abstraction, real Grammers client, mock client, shared data types.
- `src/ui/` — Ratatui widgets for folders, chats, messages, input, and layout.
- `src/config/` — config loading and theme defaults.
- `config.example.toml` — safe example config.
- `docs/config-schema.md` — larger planned configuration schema notes.
- `devbox.json` and `devbox.lock` — reproducible development environment.

## Safety and repo hygiene

- Do not commit `config.toml`, `session.dat`, `*.dat`, `target/`, or `.devbox/`.
- Treat Telegram API credentials and session files as secrets.
- Prefer small, focused changes and verify with `devbox run check` at minimum.
- If changing Rust source, run `devbox run fmt` before final validation.
- Keep README and docs factual. Avoid claiming unreleased features as stable.
