mod actions;
mod app;
mod app_keys;
mod chat_keys;
mod config;
mod confirm_keys;
mod folder_keys;
mod global_keys;
mod input_keys;
mod message_keys;
mod mouse_events;
mod state;
mod telegram;
mod text;
mod ui;

use app::App;
use color_eyre::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{CrosstermBackend, TestBackend},
};
use std::path::Path;
use std::time::Duration;
use std::{fs, io};
use telegram::types::Update;
use telegram::{GrammersClient, MockTelegramClient, TelegramClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunMode {
    RealTelegram,
    Mock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cli {
    mode: RunMode,
    config_path: String,
    smoke: bool,
    check_config: bool,
    check_auth: bool,
    help: bool,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            mode: RunMode::RealTelegram,
            config_path: "config.toml".to_string(),
            smoke: false,
            check_config: false,
            check_auth: false,
            help: false,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = parse_args()?;
    if cli.help {
        print_help();
        return Ok(());
    }

    if cli.check_config {
        check_config(&cli.config_path)?;
        return Ok(());
    }

    if cli.check_auth {
        check_auth(&cli.config_path).await?;
        return Ok(());
    }

    let theme = config::Theme::default();

    match cli.mode {
        RunMode::RealTelegram => run_real_telegram(&cli.config_path, &theme).await,
        RunMode::Mock => run_mock(&theme, cli.smoke).await,
    }
}

fn parse_args() -> Result<Cli> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I, S>(args: I) -> Result<Cli>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut cli = Cli::default();
    let mut args = args.into_iter().map(Into::into);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mock" => cli.mode = RunMode::Mock,
            "--smoke" => cli.smoke = true,
            "--check-config" => cli.check_config = true,
            "--check-auth" => cli.check_auth = true,
            "--config" | "-c" => {
                let path = args
                    .next()
                    .ok_or_else(|| color_eyre::eyre::eyre!("{} requires a path argument", arg))?;
                cli.config_path = path;
            }
            "--help" | "-h" => cli.help = true,
            _ => {
                return Err(color_eyre::eyre::eyre!(
                    "unknown argument: {}\nRun `dumbgram_tui --help` for usage.",
                    arg
                ));
            }
        }
    }

    if cli.smoke && cli.check_auth {
        return Err(color_eyre::eyre::eyre!(
            "--smoke cannot be combined with --check-auth because smoke is mock-only"
        ));
    }

    if cli.smoke {
        cli.mode = RunMode::Mock;
    }

    Ok(cli)
}

fn print_help() {
    println!(
        "Dumbgram TUI\n\n\
Usage:\n  dumbgram_tui [OPTIONS]\n\n\
Options:\n  --mock             Run with built-in mock Telegram data for smoke testing\n  --smoke            Load mock data, render off-screen, exercise interactions, and exit\n  --check-config     Validate Telegram config and session path without connecting\n  --check-auth       Connect and verify saved Telegram session without login/TUI\n  -c, --config PATH  Load Telegram config from PATH (default: config.toml)\n  -h, --help         Print this help\n\n\
Examples:\n  dumbgram_tui --mock\n  dumbgram_tui --mock --smoke\n  dumbgram_tui --check-config --config config.toml\n  dumbgram_tui --check-auth --config config.toml\n  dumbgram_tui --config config.toml"
    );
}

fn load_checked_config(config_path: &str) -> Result<config::Config> {
    let config = config::Config::load(config_path).map_err(|e| {
        eprintln!("Failed to load {}: {}", config_path, e);
        eprintln!("Create it from config.example.toml and add your Telegram credentials.");
        e
    })?;

    validate_config(&config, config_path)?;
    Ok(config)
}

fn validate_config(config: &config::Config, config_path: &str) -> Result<()> {
    if config.telegram.api_id <= 0 {
        return Err(color_eyre::eyre::eyre!(
            "telegram.api_id must be set to a positive integer in {}",
            config_path
        ));
    }

    if config.telegram.api_hash.trim().is_empty() {
        return Err(color_eyre::eyre::eyre!(
            "telegram.api_hash must be set in {}",
            config_path
        ));
    }

    if config.telegram.session_file.trim().is_empty() {
        return Err(color_eyre::eyre::eyre!(
            "telegram.session_file must be set in {}",
            config_path
        ));
    }

    let session_path = config.telegram.session_path();
    if let Some(parent) = session_path.parent()
        && !parent.as_os_str().is_empty()
        && parent.exists()
        && !parent.is_dir()
    {
        return Err(color_eyre::eyre::eyre!(
            "telegram.session_file parent path is not a directory: {}",
            parent.display()
        ));
    }

    Ok(())
}

fn ensure_session_parent_dir(session_path: &Path) -> Result<()> {
    if let Some(parent) = session_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| {
            color_eyre::eyre::eyre!(
                "failed to create telegram.session_file parent directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }

    Ok(())
}

fn check_config(config_path: &str) -> Result<()> {
    let config = load_checked_config(config_path)?;
    let session_path = config.telegram.session_path();
    let session_status = if session_path.exists() {
        "exists"
    } else {
        "will be created on login"
    };

    println!(
        "Config OK: {} (api_id={}, session_file={} [{}])",
        config_path,
        config.telegram.api_id,
        session_path.display(),
        session_status
    );

    Ok(())
}

async fn check_auth(config_path: &str) -> Result<()> {
    let config = load_checked_config(config_path)?;
    let session_path = config.telegram.session_path();
    ensure_session_parent_dir(&session_path)?;
    let session_path = session_path.to_string_lossy().into_owned();
    let client = GrammersClient::new(
        config.telegram.api_id,
        config.telegram.api_hash.clone(),
        &session_path,
    )
    .await?;

    if client.inner().is_authorized().await? {
        println!(
            "Auth OK: saved Telegram session is authorized ({})",
            session_path
        );
        Ok(())
    } else {
        Err(color_eyre::eyre::eyre!(
            "Telegram session is not authorized. Run `dumbgram_tui --config {}` to log in.",
            config_path
        ))
    }
}

async fn run_real_telegram(config_path: &str, theme: &config::Theme) -> Result<()> {
    let config = load_checked_config(config_path)?;
    let session_path = config.telegram.session_path();
    ensure_session_parent_dir(&session_path)?;
    let session_path = session_path.to_string_lossy().into_owned();

    let mut client = GrammersClient::new(
        config.telegram.api_id,
        config.telegram.api_hash.clone(),
        &session_path,
    )
    .await?;

    if !client.inner().is_authorized().await? {
        login(&mut client).await?;
    }

    run_with_client(client, theme).await
}

async fn login(client: &mut GrammersClient) -> Result<()> {
    println!("\n=== Telegram Login Required ===\n");

    let phone = prompt_input("Enter phone number (with country code, e.g., +1234567890): ")?;
    println!("Requesting login code…");
    let token = client.inner().request_login_code(&phone).await?;
    println!("OK Code sent to {}\n", phone);

    let code = prompt_input("Enter verification code: ")?;
    println!("Signing in…");

    match client.inner().sign_in(&token, &code).await {
        Ok(user) => {
            println!("OK Signed in as: {}\n", user.first_name());
            client.save_session()?;
        }
        Err(grammers_client::SignInError::PasswordRequired(password_token)) => {
            let hint = password_token.hint().unwrap_or("");
            println!("2FA enabled.");
            if !hint.is_empty() {
                println!("Hint: {}", hint);
            }

            let password = prompt_input("Enter 2FA password: ")?;
            let user = client
                .inner()
                .check_password(password_token, password.as_bytes())
                .await?;
            println!("OK Signed in with 2FA as: {}\n", user.first_name());
            client.save_session()?;
        }
        Err(e) => {
            eprintln!("Sign in failed: {}", e);
            std::process::exit(1);
        }
    }

    println!("OK Session saved! Press Enter to start…");
    prompt_input("")?;

    Ok(())
}

async fn run_mock(theme: &config::Theme, smoke: bool) -> Result<()> {
    if smoke {
        run_smoke_with_client(MockTelegramClient::new(), theme).await
    } else {
        run_with_client(MockTelegramClient::new(), theme).await
    }
}

async fn run_smoke_with_client<C: TelegramClient>(
    mut client: C,
    theme: &config::Theme,
) -> Result<()> {
    client.connect().await?;

    let mut app = App::new();
    app.state.set_status("Loading Telegram data…");
    assert_smoke_render(&mut app, theme)?;
    actions::load_initial_state(&mut app.state, &mut client).await?;
    app.state.clear_status();

    if let Some(chat_id) = app.state.selected_chat_id() {
        app.state
            .typing_users
            .insert(chat_id, vec!["Alice".to_string()]);
    }
    assert_smoke_render(&mut app, theme)?;
    app.state.typing_users.clear();
    run_interaction_smoke(&mut app, &mut client).await?;
    assert_smoke_render(&mut app, theme)?;
    run_mouse_smoke(&mut app, &mut client).await?;
    assert_smoke_render(&mut app, theme)?;

    println!(
        "Smoke OK: rendered {} folders, {} chats, {} messages and exercised keyboard/mouse interactions",
        app.state.folders.len(),
        app.state.chats.len(),
        app.state.messages.len()
    );

    Ok(())
}

fn smoke_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn smoke_key_with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn smoke_click(column: u16, row: u16) -> crossterm::event::MouseEvent {
    smoke_mouse(MouseEventKind::Down(MouseButton::Left), column, row)
}

fn smoke_mouse(kind: MouseEventKind, column: u16, row: u16) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn assert_smoke_render(app: &mut App, theme: &config::Theme) -> Result<()> {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| ui::render_layout(frame, app, theme))?;

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    if !rendered.contains("Chats") {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the Chats panel"
        ));
    }
    if !app.state.chats.is_empty() {
        let chat_position = format!(
            "Chats {}/{}",
            app.state.selected_chat_index.min(app.state.chats.len() - 1) + 1,
            app.state.chats.len()
        );
        if !rendered.contains(&chat_position) {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the chat position indicator"
            ));
        }
        if !rendered.contains('▶') || rendered.contains(">>") {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the restored selected-row marker"
            ));
        }
        if app.state.chats.iter().any(|chat| chat.unread_count > 0)
            && !rendered.contains("unread ·")
        {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the plain unread chat indicator"
            ));
        }
        if app.state.chats.iter().any(|chat| chat.is_group) && !rendered.contains("group ·") {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the plain group chat indicator"
            ));
        }
        if rendered.contains("[G]") {
            return Err(color_eyre::eyre::eyre!(
                "smoke render still included the legacy group chat badge"
            ));
        }
    }
    if !app.state.messages.is_empty() {
        let message_position = format!(
            "{}/{}",
            app.state
                .selected_message_index
                .min(app.state.messages.len() - 1)
                + 1,
            app.state.messages.len()
        );
        if !rendered.contains(&message_position) {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the message position indicator"
            ));
        }
        if app
            .state
            .messages
            .iter()
            .any(|message| message.reply_to_content.is_some())
            && !rendered.contains("└─ Reply:")
        {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the restored reply marker"
            ));
        }
        for status_label in app.state.messages.iter().filter_map(|message| {
            if !message.is_own {
                return None;
            }

            match message.status {
                telegram::types::MessageStatus::Sending => Some("sending"),
                telegram::types::MessageStatus::Sent => Some("sent"),
                telegram::types::MessageStatus::Delivered => Some("delivered"),
                telegram::types::MessageStatus::Read => Some("read"),
                telegram::types::MessageStatus::Failed => Some("failed"),
            }
        }) {
            let status_metadata = format!(" · {status_label}");
            if !rendered.contains(&status_metadata) {
                return Err(color_eyre::eyre::eyre!(
                    "smoke render did not include message status metadata with Unicode separator"
                ));
            }
        }
        if rendered.contains("[edited]") {
            return Err(color_eyre::eyre::eyre!(
                "smoke render still included the legacy edited marker"
            ));
        }
    }
    if let Some(users) = app
        .state
        .selected_chat_id()
        .and_then(|chat_id| app.state.typing_users.get(&chat_id))
        .filter(|users| !users.is_empty())
    {
        let typing_label = if users.len() == 1 {
            format!(" · {} typing", users[0])
        } else {
            format!(" · {} typing", users.len())
        };
        if !rendered.contains(&typing_label) {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the selected chat typing indicator with Unicode separator"
            ));
        }
    }
    if let Some(status) = app.state.status_message.as_ref()
        && !rendered.contains(status)
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the status banner"
        ));
    }
    if !rendered.contains("Input") && !rendered.contains("Type message") {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the input panel"
        ));
    }
    if !rendered.contains("Focus:") {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the controls help bar"
        ));
    }
    if !rendered.contains("Tab focus ·") || rendered.contains(" | ") {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the restored help-bar Unicode separators"
        ));
    }

    Ok(())
}

async fn run_interaction_smoke<C: TelegramClient>(app: &mut App, client: &mut C) -> Result<()> {
    if app.state.chats.len() < 2 || app.state.messages.len() < 2 {
        return Err(color_eyre::eyre::eyre!(
            "smoke data did not include enough chats/messages"
        ));
    }
    if app.state.chats[app.state.selected_chat_index].unread_count != 0 {
        return Err(color_eyre::eyre::eyre!(
            "initial selected chat still showed unread messages after loading"
        ));
    }
    if app.state.folders[app.state.selected_folder_index].unread_count != 2 {
        return Err(color_eyre::eyre::eyre!(
            "initial selected folder unread count did not reconcile loaded chats"
        ));
    }

    handle_key_event(app, smoke_key(KeyCode::Down), client).await?;
    if app.state.focused_panel != state::FocusedPanel::Chats {
        return Err(color_eyre::eyre::eyre!(
            "Down from folders did not focus chats"
        ));
    }

    handle_key_event(app, smoke_key(KeyCode::Down), client).await?;
    if app.state.selected_chat_index != 1 || app.state.messages.len() != 2 {
        return Err(color_eyre::eyre::eyre!(
            "Down in chats did not load the second mock chat"
        ));
    }

    handle_key_event(app, smoke_key(KeyCode::Right), client).await?;
    handle_key_event(app, smoke_key(KeyCode::End), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Char('e')), client).await?;
    if app.state.focused_panel != state::FocusedPanel::Input
        || app.state.editing_message_id != Some(2)
    {
        return Err(color_eyre::eyre::eyre!(
            "edit shortcut did not enter input edit mode for selected own message"
        ));
    }

    handle_key_event(app, smoke_key(KeyCode::Char('!')), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Enter), client).await?;
    let edited = app
        .state
        .messages
        .iter()
        .find(|message| message.id == 2)
        .ok_or_else(|| color_eyre::eyre::eyre!("edited message disappeared"))?;
    if !edited.is_edited || !edited.content.ends_with('!') {
        return Err(color_eyre::eyre::eyre!(
            "edit interaction did not update selected message content"
        ));
    }

    app.state.clear_input_mode();
    app.state.focused_panel = state::FocusedPanel::Input;
    handle_key_event(app, smoke_key(KeyCode::Char('a')), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Char('c')), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Left), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Char('b')), client).await?;
    handle_key_event(app, smoke_key(KeyCode::End), client).await?;
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('a'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('f'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('d'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('e'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('b'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    handle_key_event(app, smoke_key(KeyCode::Char('b')), client).await?;
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('u'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    if app.state.input_buffer != "c" || app.state.input_cursor() != 0 {
        return Err(color_eyre::eyre::eyre!(
            "Ctrl-U did not delete input text before the cursor"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Char('a')), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Char('b')), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Left), client).await?;
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('k'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    if app.state.input_buffer != "a" || app.state.input_cursor() != 1 {
        return Err(color_eyre::eyre::eyre!(
            "Ctrl-K did not delete input text after the cursor"
        ));
    }
    for c in " brave world".chars() {
        handle_key_event(app, smoke_key(KeyCode::Char(c)), client).await?;
    }
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('w'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    if app.state.input_buffer != "a brave " || app.state.input_cursor() != 8 {
        return Err(color_eyre::eyre::eyre!(
            "Ctrl-W did not delete the previous input word"
        ));
    }

    app.state.clear_input_mode();
    app.state.focused_panel = state::FocusedPanel::Input;
    app.state.input_buffer = "smoke send".to_string();
    app.state.move_input_cursor_to_end();
    handle_key_event(app, smoke_key(KeyCode::Enter), client).await?;
    if !app.state.messages.iter().any(|message| {
        message.content == "smoke send" && message.status == telegram::types::MessageStatus::Sent
    }) {
        return Err(color_eyre::eyre::eyre!(
            "send interaction did not append a sent mock message"
        ));
    }
    if app
        .state
        .messages
        .get(app.state.selected_message_index)
        .is_none_or(|message| message.content != "smoke send")
    {
        return Err(color_eyre::eyre::eyre!(
            "send interaction did not select the newly sent message"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Messages;
    app.state.selected_message_index = app
        .state
        .messages
        .iter()
        .position(|message| message.id == 2)
        .ok_or_else(|| color_eyre::eyre::eyre!("message to delete was not found"))?;
    handle_key_event(app, smoke_key(KeyCode::Char('d')), client).await?;
    if app.state.delete_confirmation.is_none_or(|confirmation| {
        confirmation.message_id != 2 || confirmation.chat_id != app.state.chats[1].id
    }) {
        return Err(color_eyre::eyre::eyre!(
            "delete shortcut did not request confirmation"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Char('y')), client).await?;
    if app.state.messages.iter().any(|message| message.id == 2) {
        return Err(color_eyre::eyre::eyre!(
            "delete confirmation did not remove selected message"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Messages;
    app.state.selected_message_index = 0;
    handle_key_event(app, smoke_key(KeyCode::Char('r')), client).await?;
    if app.state.focused_panel != state::FocusedPanel::Input
        || app.state.replying_to_message_id.is_none()
    {
        return Err(color_eyre::eyre::eyre!(
            "reply shortcut did not enter input reply mode"
        ));
    }
    app.state.input_buffer = "smoke reply".to_string();
    handle_key_event(app, smoke_key(KeyCode::Enter), client).await?;
    if !app.state.messages.iter().any(|message| {
        message.content == "smoke reply"
            && message.reply_to_content.is_some()
            && message.status == telegram::types::MessageStatus::Sent
    }) {
        return Err(color_eyre::eyre::eyre!(
            "reply interaction did not append a sent reply mock message"
        ));
    }
    if app.state.replying_to_message_id.is_some()
        || app
            .state
            .messages
            .get(app.state.selected_message_index)
            .is_none_or(|message| message.content != "smoke reply")
    {
        return Err(color_eyre::eyre::eyre!(
            "reply interaction did not clear reply mode and select the new reply"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Input;
    app.state.input_buffer = "discard me".to_string();
    handle_key_event(
        app,
        smoke_key_with_modifiers(KeyCode::Char('c'), KeyModifiers::CONTROL),
        client,
    )
    .await?;
    if app.state.focused_panel != state::FocusedPanel::Messages
        || !app.state.input_buffer.is_empty()
    {
        return Err(color_eyre::eyre::eyre!(
            "Ctrl-C from plain input did not cancel and clear the draft"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Input;
    app.state.input_buffer = "draft stays".to_string();
    handle_key_event(app, smoke_key(KeyCode::Tab), client).await?;
    if app.state.focused_panel != state::FocusedPanel::Folders
        || app.state.input_buffer != "draft stays"
    {
        return Err(color_eyre::eyre::eyre!(
            "Tab from input did not cycle focus while preserving draft text"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Tab), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Tab), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Tab), client).await?;
    if app.state.focused_panel != state::FocusedPanel::Input
        || app.state.input_buffer != "draft stays"
    {
        return Err(color_eyre::eyre::eyre!(
            "Tab focus cycle did not return to input with draft text preserved"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Chats;
    handle_key_event(app, smoke_key(KeyCode::Down), client).await?;
    if app.state.selected_chat_index != 2 || !app.state.input_buffer.is_empty() {
        return Err(color_eyre::eyre::eyre!(
            "chat switch did not save old draft and clear new chat draft"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Input;
    app.state.input_buffer = "team draft".to_string();
    app.state.focused_panel = state::FocusedPanel::Chats;
    handle_key_event(app, smoke_key(KeyCode::Up), client).await?;
    if app.state.selected_chat_index != 1 || app.state.input_buffer != "draft stays" {
        return Err(color_eyre::eyre::eyre!(
            "returning to previous chat did not restore its draft"
        ));
    }

    handle_key_event(app, smoke_key(KeyCode::Down), client).await?;
    if app.state.selected_chat_index != 2 || app.state.input_buffer != "team draft" {
        return Err(color_eyre::eyre::eyre!(
            "returning to next chat did not restore its draft"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Messages;
    app.state.selected_message_index = 1;
    handle_key_event(app, smoke_key(KeyCode::Char('e')), client).await?;
    if app.state.editing_message_id != Some(2) || app.state.input_buffer == "team draft" {
        return Err(color_eyre::eyre::eyre!(
            "edit mode did not replace the draft with selected message text"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Esc), client).await?;
    if app.state.editing_message_id.is_some()
        || app.state.focused_panel != state::FocusedPanel::Messages
        || app.state.input_buffer != "team draft"
    {
        return Err(color_eyre::eyre::eyre!(
            "cancelling edit mode did not restore the underlying chat draft"
        ));
    }

    handle_key_event(app, smoke_key(KeyCode::End), client).await?;
    if app.state.selected_message_index != app.state.messages.len().saturating_sub(1) {
        return Err(color_eyre::eyre::eyre!(
            "End did not jump to the last message"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Home), client).await?;
    if app.state.selected_message_index != 0 || app.state.message_scroll_offset != 0 {
        return Err(color_eyre::eyre::eyre!(
            "Home did not jump to the first message and reset message scroll"
        ));
    }

    Ok(())
}

async fn run_mouse_smoke<C: TelegramClient>(app: &mut App, client: &mut C) -> Result<()> {
    if app.state.chats_area.width < 4 || app.state.chats_area.height < 4 {
        return Err(color_eyre::eyre::eyre!(
            "smoke layout did not expose a clickable chat list"
        ));
    }

    let chat_click = smoke_click(app.state.chats_area.x + 2, app.state.chats_area.y + 1);
    handle_mouse_event(app, chat_click, client).await?;
    if app.state.focused_panel != state::FocusedPanel::Chats || app.state.selected_chat_index != 0 {
        return Err(color_eyre::eyre::eyre!(
            "mouse click in chat list did not focus/select the first chat"
        ));
    }
    if app.state.messages.len() != 3 {
        return Err(color_eyre::eyre::eyre!(
            "mouse chat selection did not load the first mock chat messages"
        ));
    }

    let chat_scroll_down = smoke_mouse(
        MouseEventKind::ScrollDown,
        app.state.chats_area.x + 2,
        app.state.chats_area.y + 1,
    );
    handle_mouse_event(app, chat_scroll_down, client).await?;
    if app.state.focused_panel != state::FocusedPanel::Chats
        || app.state.selected_chat_index != 1
        || app.state.messages.len() != 2
    {
        return Err(color_eyre::eyre::eyre!(
            "mouse wheel down over chats did not select/load the next chat"
        ));
    }

    let chat_scroll_up = smoke_mouse(
        MouseEventKind::ScrollUp,
        app.state.chats_area.x + 2,
        app.state.chats_area.y + 1,
    );
    handle_mouse_event(app, chat_scroll_up, client).await?;
    if app.state.focused_panel != state::FocusedPanel::Chats
        || app.state.selected_chat_index != 0
        || app.state.messages.len() != 3
    {
        return Err(color_eyre::eyre::eyre!(
            "mouse wheel up over chats did not select/load the previous chat"
        ));
    }

    let messages_click = smoke_click(app.state.messages_area.x + 2, app.state.messages_area.y + 2);
    handle_mouse_event(app, messages_click, client).await?;
    if app.state.focused_panel != state::FocusedPanel::Messages
        || app.state.selected_message_index != 1
    {
        return Err(color_eyre::eyre::eyre!(
            "mouse click in messages did not focus/select the clicked message"
        ));
    }

    app.state.delete_confirmation =
        app.state
            .selected_message()
            .map(|message| state::DeleteConfirmation {
                chat_id: message.chat_id,
                message_id: message.id,
            });
    let blocked_chat_index = app.state.selected_chat_index;
    let blocked_focus = app.state.focused_panel;
    let blocked_message_count = app.state.messages.len();
    let blocked_chat_scroll = smoke_mouse(
        MouseEventKind::ScrollDown,
        app.state.chats_area.x + 2,
        app.state.chats_area.y + 1,
    );
    handle_mouse_event(app, blocked_chat_scroll, client).await?;
    if app.state.selected_chat_index != blocked_chat_index
        || app.state.focused_panel != blocked_focus
        || app.state.messages.len() != blocked_message_count
    {
        return Err(color_eyre::eyre::eyre!(
            "mouse event changed chat/focus while delete confirmation was open"
        ));
    }
    app.state.delete_confirmation = None;
    app.state.selected_message_index = 0;
    app.state.ensure_selected_message_visible();

    let scroll_down = smoke_mouse(
        MouseEventKind::ScrollDown,
        app.state.messages_area.x + 2,
        app.state.messages_area.y + 1,
    );
    handle_mouse_event(app, scroll_down, client).await?;
    if app.state.selected_message_index != 1 {
        return Err(color_eyre::eyre::eyre!(
            "mouse wheel down over messages did not move message selection down"
        ));
    }

    let scroll_up = smoke_mouse(
        MouseEventKind::ScrollUp,
        app.state.messages_area.x + 2,
        app.state.messages_area.y + 1,
    );
    handle_mouse_event(app, scroll_up, client).await?;
    if app.state.selected_message_index != 0 {
        return Err(color_eyre::eyre::eyre!(
            "mouse wheel up over messages did not move message selection up"
        ));
    }

    app.state.input_buffer = "a好b".to_string();
    app.state.move_input_cursor_to_end();
    let input_click = smoke_click(app.state.input_area.x + 4, app.state.input_area.y + 1);
    handle_mouse_event(app, input_click, client).await?;
    if app.state.focused_panel != state::FocusedPanel::Input || app.state.input_cursor() != 2 {
        return Err(color_eyre::eyre::eyre!(
            "mouse click in input did not focus input panel and place the cursor"
        ));
    }

    Ok(())
}

async fn run_with_client<C: TelegramClient>(mut client: C, theme: &config::Theme) -> Result<()> {
    client.connect().await?;

    let mut app = App::new();
    let mut terminal = setup_terminal()?;

    app.state.set_status("Loading Telegram data…");
    terminal.draw(|frame| ui::render_layout(frame, &mut app, theme))?;

    let result = async {
        actions::load_initial_state(&mut app.state, &mut client).await?;
        app.state.set_status("Subscribing to Telegram updates…");
        terminal.draw(|frame| ui::render_layout(frame, &mut app, theme))?;
        let mut update_rx = client.subscribe_updates().await?;
        app.state.clear_status();
        run_app(&mut terminal, &mut app, theme, &mut client, &mut update_rx).await
    }
    .await;

    let restore_result = restore_terminal(&mut terminal);
    match (result, restore_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn prompt_input(msg: &str) -> Result<String> {
    use std::io::{self, Write};
    print!("{}", msg);
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_app<C: TelegramClient>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    theme: &config::Theme,
    client: &mut C,
    update_rx: &mut tokio::sync::mpsc::UnboundedReceiver<Update>,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render_layout(f, app, theme))?;

        while let Ok(update) = update_rx.try_recv() {
            app.state.apply_update(update);
        }

        app.state.check_notification_timeout();

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let mut progress = UiProgress::Live { terminal, theme };
                    handle_key_event_with_progress(app, key, client, &mut progress).await?;
                }
                Event::Mouse(mouse_event) => {
                    let mut progress = UiProgress::Live { terminal, theme };
                    handle_mouse_event_with_progress(app, mouse_event, client, &mut progress)
                        .await?;
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OlderMessageNavigation {
    OneLine,
    Page,
}

enum UiProgress<'a> {
    Live {
        terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
        theme: &'a config::Theme,
    },
    Silent,
}

impl UiProgress<'_> {
    fn show(&mut self, app: &mut App, status: impl Into<String>) -> Result<()> {
        app.state.set_status(status);
        match self {
            Self::Live { terminal, theme } => {
                terminal.draw(|frame| ui::render_layout(frame, app, theme))?;
            }
            Self::Silent => {}
        }
        Ok(())
    }
}

async fn handle_key_event<C: TelegramClient>(
    app: &mut App,
    key: KeyEvent,
    client: &mut C,
) -> Result<()> {
    let mut progress = UiProgress::Silent;
    handle_key_event_with_progress(app, key, client, &mut progress).await
}

async fn handle_key_event_with_progress<C: TelegramClient>(
    app: &mut App,
    key: KeyEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
) -> Result<()> {
    if global_keys::handle_global_key(&mut app.state, key) == global_keys::GlobalKeyOutcome::Handled
    {
        return Ok(());
    }

    if app.state.focused_panel == state::FocusedPanel::Input {
        handle_input_focused(app, key, client, progress).await?;
    } else {
        handle_normal_navigation(app, key, client, progress).await?;
    }
    Ok(())
}

async fn handle_normal_navigation<C: TelegramClient>(
    app: &mut App,
    key: KeyEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
) -> Result<()> {
    if app.state.delete_confirmation.is_some() {
        match confirm_keys::handle_confirm_key(key) {
            confirm_keys::ConfirmKeyOutcome::Confirm => {
                progress.show(app, "Deleting message…")?;
                actions::confirm_delete(&mut app.state, client).await?;
            }
            confirm_keys::ConfirmKeyOutcome::Cancel => {
                app.state.cancel_delete_confirmation();
            }
            confirm_keys::ConfirmKeyOutcome::Ignored => {}
        }
        return Ok(());
    }

    if let Some(navigation) = older_message_key_navigation(app, key) {
        progress.show(app, "Loading older messages…")?;
        let loaded = actions::load_older_selected_chat_messages(&mut app.state, client).await?;
        if loaded > 0 {
            app.state.clear_status();
            apply_older_message_navigation(&mut app.state, navigation);
        }
        return Ok(());
    }

    if message_keys::handle_message_key(&mut app.state, key)
        == message_keys::MessageKeyOutcome::Handled
    {
        return Ok(());
    }

    match chat_keys::handle_chat_key(&mut app.state, key) {
        chat_keys::ChatKeyOutcome::Handled => return Ok(()),
        chat_keys::ChatKeyOutcome::OpenNextChat => {
            progress.show(app, "Loading chat messages…")?;
            let result = actions::open_next_chat(&mut app.state, client).await;
            if result.is_ok() {
                app.state.clear_status();
            }
            result?;
            return Ok(());
        }
        chat_keys::ChatKeyOutcome::OpenChatAt(index) => {
            progress.show(app, "Loading chat messages…")?;
            let result = actions::open_chat_at(&mut app.state, client, index).await;
            if result.is_ok() {
                app.state.clear_status();
            }
            result?;
            return Ok(());
        }
        chat_keys::ChatKeyOutcome::Ignored => {}
    }

    match folder_keys::handle_folder_key(&mut app.state, key) {
        folder_keys::FolderKeyOutcome::Handled => return Ok(()),
        folder_keys::FolderKeyOutcome::OpenPreviousFolder => {
            progress.show(app, "Loading folder chats…")?;
            let result = actions::open_previous_folder(&mut app.state, client).await;
            if result.is_ok() {
                app.state.clear_status();
            }
            result?;
            return Ok(());
        }
        folder_keys::FolderKeyOutcome::OpenNextFolder => {
            progress.show(app, "Loading folder chats…")?;
            let result = actions::open_next_folder(&mut app.state, client).await;
            if result.is_ok() {
                app.state.clear_status();
            }
            result?;
            return Ok(());
        }
        folder_keys::FolderKeyOutcome::Ignored => {}
    }

    match app_keys::handle_app_key(&mut app.state, key) {
        app_keys::AppKeyOutcome::Handled | app_keys::AppKeyOutcome::Ignored => {}
        app_keys::AppKeyOutcome::Quit => app.quit(),
    }
    Ok(())
}

fn older_message_key_navigation(app: &App, key: KeyEvent) -> Option<OlderMessageNavigation> {
    if app.state.focused_panel != state::FocusedPanel::Messages
        || app.state.messages.is_empty()
        || app.state.selected_message_index != 0
    {
        return None;
    }

    match key.code {
        KeyCode::Up => Some(OlderMessageNavigation::OneLine),
        KeyCode::PageUp => Some(OlderMessageNavigation::Page),
        _ => None,
    }
}

fn apply_older_message_navigation(state: &mut state::AppState, navigation: OlderMessageNavigation) {
    match navigation {
        OlderMessageNavigation::OneLine => state.select_prev_message(),
        OlderMessageNavigation::Page => state.page_messages_up(),
    }
}

async fn handle_input_focused<C: TelegramClient>(
    app: &mut App,
    key: KeyEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
) -> Result<()> {
    if input_keys::handle_input_key(&mut app.state, key) == input_keys::InputKeyOutcome::Submit {
        let Some(action) = app.state.prepare_message_submit() else {
            return Ok(());
        };

        match action {
            state::MessageSubmitAction::Send { chat_id, content } => {
                let pending = actions::begin_send_message(&mut app.state, chat_id, content);
                progress.show(app, "Sending message…")?;
                actions::finish_send_message(&mut app.state, client, pending).await?;
            }
            other => {
                progress.show(app, message_submit_action_status(&other))?;
                actions::execute_message_submit_action(&mut app.state, client, other).await?;
            }
        }
    }
    Ok(())
}

fn message_submit_action_status(action: &state::MessageSubmitAction) -> &'static str {
    match action {
        state::MessageSubmitAction::Edit { .. } => "Saving edit…",
        state::MessageSubmitAction::Reply { .. } => "Sending reply…",
        state::MessageSubmitAction::Send { .. } => "Sending message…",
    }
}

fn older_message_scroll_requested(app: &App, mouse_event: crossterm::event::MouseEvent) -> bool {
    mouse_event.kind == MouseEventKind::ScrollUp
        && app
            .state
            .messages_area
            .contains(ratatui::layout::Position::new(
                mouse_event.column,
                mouse_event.row,
            ))
        && !app.state.messages.is_empty()
        && app.state.selected_message_index == 0
}

async fn handle_mouse_event<C: TelegramClient>(
    app: &mut App,
    mouse_event: crossterm::event::MouseEvent,
    client: &mut C,
) -> Result<()> {
    let mut progress = UiProgress::Silent;
    handle_mouse_event_with_progress(app, mouse_event, client, &mut progress).await
}

async fn handle_mouse_event_with_progress<C: TelegramClient>(
    app: &mut App,
    mouse_event: crossterm::event::MouseEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
) -> Result<()> {
    if app.state.delete_confirmation.is_some() {
        return Ok(());
    }

    if older_message_scroll_requested(app, mouse_event) {
        progress.show(app, "Loading older messages…")?;
        let loaded = actions::load_older_selected_chat_messages(&mut app.state, client).await?;
        if loaded > 0 {
            app.state.clear_status();
            app.state.select_prev_message();
        }
        return Ok(());
    }

    match mouse_events::handle_mouse_scroll(&mut app.state, mouse_event) {
        mouse_events::MouseScrollOutcome::Handled => return Ok(()),
        mouse_events::MouseScrollOutcome::OpenNextChat => {
            progress.show(app, "Loading chat messages…")?;
            let result = actions::open_next_chat(&mut app.state, client).await;
            if result.is_ok() {
                app.state.clear_status();
            }
            result?;
            return Ok(());
        }
        mouse_events::MouseScrollOutcome::OpenPreviousChat => {
            progress.show(app, "Loading chat messages…")?;
            let result = actions::open_previous_chat_wrapping(&mut app.state, client).await;
            if result.is_ok() {
                app.state.clear_status();
            }
            result?;
            return Ok(());
        }
        mouse_events::MouseScrollOutcome::Ignored => {}
    }

    match mouse_events::handle_mouse_click(&mut app.state, mouse_event) {
        mouse_events::MouseClickOutcome::Handled | mouse_events::MouseClickOutcome::Ignored => {}
        mouse_events::MouseClickOutcome::OpenFolderAt(index) => {
            progress.show(app, "Loading folder chats…")?;
            let result = actions::open_folder_at(&mut app.state, client, index).await;
            if result.is_ok() {
                app.state.clear_status();
            }
            result?;
        }
        mouse_events::MouseClickOutcome::OpenChatAt(index) => {
            progress.show(app, "Loading chat messages…")?;
            let result = actions::open_chat_at(&mut app.state, client, index).await;
            if result.is_ok() {
                app.state.clear_status();
            }
            result?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        OlderMessageNavigation, RunMode, ensure_session_parent_dir, older_message_key_navigation,
        older_message_scroll_requested, parse_args_from, validate_config,
    };
    use crate::app::App;
    use crate::config::telegram::{Config, TelegramConfig};
    use crate::state::FocusedPanel;
    use crate::telegram::types::{Message, MessageStatus};
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_session_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "dumbgram-tui-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ))
    }

    fn message(id: i32) -> Message {
        Message {
            id,
            chat_id: 10,
            sender_name: "Alice".to_string(),
            content: format!("message {id}"),
            timestamp: Utc::now(),
            is_own: false,
            is_edited: false,
            reply_to_content: None,
            status: MessageStatus::Delivered,
            can_edit: false,
            can_delete: false,
            error: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn older_message_key_navigation_requests_history_only_at_loaded_top() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages = vec![message(1), message(2)];
        app.state.selected_message_index = 0;

        assert_eq!(
            older_message_key_navigation(&app, key(KeyCode::Up)),
            Some(OlderMessageNavigation::OneLine)
        );
        assert_eq!(
            older_message_key_navigation(&app, key(KeyCode::PageUp)),
            Some(OlderMessageNavigation::Page)
        );

        app.state.selected_message_index = 1;
        assert_eq!(older_message_key_navigation(&app, key(KeyCode::Up)), None);
    }

    #[test]
    fn older_message_scroll_requests_history_only_at_loaded_top() {
        let mut app = App::new();
        app.state.messages_area = Rect::new(0, 0, 40, 10);
        app.state.messages = vec![message(1), message(2)];
        app.state.selected_message_index = 0;
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };

        assert!(older_message_scroll_requested(&app, scroll_up));

        app.state.selected_message_index = 1;
        assert!(!older_message_scroll_requested(&app, scroll_up));
    }

    #[test]
    fn smoke_flag_forces_mock_mode() {
        let cli = parse_args_from(["--smoke"]).expect("--smoke should parse");

        assert_eq!(cli.mode, RunMode::Mock);
        assert!(cli.smoke);
    }

    #[test]
    fn smoke_flag_overrides_real_config_path_without_real_mode() {
        let cli = parse_args_from(["--config", "real.toml", "--smoke"])
            .expect("--config plus --smoke should parse");

        assert_eq!(cli.mode, RunMode::Mock);
        assert_eq!(cli.config_path, "real.toml");
        assert!(cli.smoke);
    }

    #[test]
    fn config_path_does_not_imply_mock_without_smoke() {
        let cli = parse_args_from(["--config", "real.toml"]).expect("--config should parse");

        assert_eq!(cli.mode, RunMode::RealTelegram);
        assert_eq!(cli.config_path, "real.toml");
        assert!(!cli.smoke);
    }

    #[test]
    fn check_auth_parses_as_real_opt_in_diagnostic() {
        let cli = parse_args_from(["--check-auth", "--config", "real.toml"])
            .expect("--check-auth should parse");

        assert!(cli.check_auth);
        assert_eq!(cli.mode, RunMode::RealTelegram);
        assert_eq!(cli.config_path, "real.toml");
    }

    #[test]
    fn smoke_cannot_be_combined_with_check_auth() {
        let err = parse_args_from(["--smoke", "--check-auth"])
            .expect_err("--smoke plus --check-auth must be rejected to keep smoke mock-only");

        assert!(err.to_string().contains("--smoke cannot be combined"));
    }

    #[test]
    fn config_validation_allows_missing_session_parent_for_launch_creation() {
        let missing_parent = unique_temp_session_path();
        let config = Config {
            telegram: TelegramConfig {
                api_id: 1,
                api_hash: "hash".to_string(),
                session_file: missing_parent
                    .join("session.dat")
                    .to_string_lossy()
                    .into_owned(),
            },
        };

        validate_config(&config, "test-config.toml")
            .expect("missing parent should be creatable later");
        assert!(!missing_parent.exists());
    }

    #[test]
    fn ensure_session_parent_dir_creates_missing_parent() {
        let missing_parent = unique_temp_session_path();
        let session_path = missing_parent.join("session.dat");

        ensure_session_parent_dir(&session_path).expect("missing session parent should be created");

        assert!(missing_parent.is_dir());
        std::fs::remove_dir_all(missing_parent).ok();
    }
}
