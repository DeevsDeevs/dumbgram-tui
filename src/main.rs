mod actions;
mod app;
mod app_keys;
mod chat_keys;
mod config;
mod confirm_keys;
mod diagnostics;
mod file_opener;
mod folder_keys;
mod global_keys;
mod input_keys;
mod links;
mod message_keys;
mod mouse_events;
mod paths;
mod preferences;
mod state;
mod telegram;
mod terminal_clipboard;
mod terminal_images;
mod text;
mod ui;

use app::App;
use color_eyre::Result;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_core::Stream;
use ratatui::{
    Terminal,
    backend::{CrosstermBackend, TestBackend},
};
use std::collections::HashMap;
use std::future::{pending, poll_fn};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fs, io};
use telegram::types::{Message, ThreadTopic, Update};
use telegram::{GrammersClient, MockTelegramClient, TelegramClient};
use tokio::time::Instant as TokioInstant;

const LOADING_TELEGRAM_STATUS: &str = "Loading Telegram data…";
const LOADING_OLDER_MESSAGES_STATUS: &str = "Loading older messages…";
const LOADING_CHAT_MESSAGES_STATUS: &str = "Loading chat messages…";
const LOADING_FOLDER_CHATS_STATUS: &str = "Loading folder chats…";
const TELEGRAM_STATE_REFRESHED_STATUS: &str = "Telegram state refreshed";
const TELEGRAM_UPDATES_DISCONNECTED_ERROR: &str =
    "Telegram updates disconnected; retrying subscription";
const RECONCILIATION_INTERVAL: Duration = Duration::from_secs(5 * 60);
const RECONCILIATION_RETRY_DELAY: Duration = Duration::from_secs(10);
const RECONCILIATION_FOCUS_STALE_AFTER: Duration = Duration::from_secs(30);
const UPDATE_SUBSCRIPTION_RETRY_DELAY: Duration = Duration::from_secs(5);
const LINK_OPENED_STATUS: &str = "Link opened";
const MESSAGE_TEXT_COPIED_STATUS: &str = "Message text copied";
const CHAT_NAME_COPIED_STATUS: &str = "Chat name copied";
const DOWNLOADING_MEDIA_STATUS: &str = "Downloading media…";
const MEDIA_DOWNLOADED_STATUS: &str = "Media downloaded to Downloads";
const DOWNLOADED_MEDIA_OPENED_STATUS: &str = "Downloaded media opened";
const NO_DOWNLOADED_MEDIA_STATUS: &str = "No downloaded media for selected message";
const NO_TEXT_IN_SELECTED_MESSAGE_STATUS: &str = "No text in selected message";
const NO_LINK_IN_SELECTED_MESSAGE_STATUS: &str = "No link in selected message";
const NO_MEDIA_IN_SELECTED_MESSAGE_STATUS: &str = "No downloadable media in selected message";
const OPEN_LINK_FAILED_PREFIX: &str = "Open link failed";
const OPEN_DOWNLOADED_MEDIA_FAILED_PREFIX: &str = "Open downloaded media failed";
const DELETING_MESSAGE_STATUS: &str = "Deleting message…";
const SENDING_MESSAGE_STATUS: &str = "Sending message…";
const SAVING_EDIT_STATUS: &str = "Saving edit…";
const SENDING_REPLY_STATUS: &str = "Sending reply…";
const APP_COMMAND: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const SMOKE_CHECK_CONFIG_CONFLICT: &str =
    "--smoke cannot be combined with --check-config because smoke is mock-only";
const SMOKE_CHECK_AUTH_CONFLICT: &str =
    "--smoke cannot be combined with --check-auth because smoke is mock-only";
const CHECK_CONFIG_AUTH_CONFLICT: &str =
    "--check-config cannot be combined with --check-auth; choose one diagnostic";
const CONFIG_PATH_ARGUMENT_REQUIRED: &str = "--config requires a path argument";
const LOG_PATH_ARGUMENT_REQUIRED: &str = "--log requires a path argument";
const SLOW_RENDER_LOG_THRESHOLD_MS: u128 = 100;
const CLI_USAGE_EXIT_CODE: i32 = 2;
const SETUP_ERROR_EXIT_CODE: i32 = 1;
const CONFIG_LOAD_HELP: &str =
    "Create it from config.example.toml and add your Telegram credentials.";
const CHECK_CONFIG_SESSION_EXISTS_STATUS: &str = "exists";
const CHECK_CONFIG_SESSION_WILL_CREATE_STATUS: &str = "will be created on login";
const CHECK_AUTH_OK_PREFIX: &str = "Auth OK: saved Telegram session is authorized";
const LOGIN_HEADER: &str = "\n=== Telegram Login Required ===\n";
const LOGIN_PHONE_PROMPT: &str = "Enter phone number (with country code, e.g., +1234567890): ";
const LOGIN_REQUESTING_CODE_STATUS: &str = "Requesting login code…";
const LOGIN_CODE_SENT_PREFIX: &str = "OK Code sent to";
const LOGIN_CODE_PROMPT: &str = "Enter verification code: ";
const LOGIN_SIGNING_IN_STATUS: &str = "Signing in…";
const LOGIN_SIGNED_IN_PREFIX: &str = "OK Signed in as:";
const LOGIN_2FA_ENABLED_STATUS: &str = "2FA enabled.";
const LOGIN_2FA_HINT_PREFIX: &str = "Hint:";
const LOGIN_2FA_PROMPT: &str = "Enter 2FA password: ";
const LOGIN_2FA_SIGNED_IN_PREFIX: &str = "OK Signed in with 2FA as:";
const LOGIN_FAILED_PREFIX: &str = "Sign in failed";
const LOGIN_SESSION_SAVED_STATUS: &str = "OK Session saved! Press Enter to start…";
const LOGIN_START_PROMPT: &str = "";
const PROMPT_EOF_ERROR: &str = "input ended before a response was entered";
const PROMPT_EMPTY_ERROR: &str = "input cannot be empty";
const SMOKE_OK_PREFIX: &str = "Smoke OK";
const SMOKE_RENDER_WIDTH: u16 = 120;
const SMOKE_RENDER_HEIGHT: u16 = 30;
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

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
    log_path: Option<String>,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            mode: RunMode::RealTelegram,
            config_path: default_config_path_string(),
            smoke: false,
            check_config: false,
            check_auth: false,
            help: false,
            log_path: None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = parse_args().unwrap_or_else(|error| exit_with_error(error, CLI_USAGE_EXIT_CODE));

    if cli.help {
        print_help();
        return Ok(());
    }

    if let Some(log_path) = cli.log_path.as_ref() {
        diagnostics::init(log_path)
            .unwrap_or_else(|error| exit_with_error(error, SETUP_ERROR_EXIT_CODE));
        diagnostics::event(
            "app_start",
            format!(
                "mode={:?} smoke={} check_config={} check_auth={} config_path={}",
                cli.mode, cli.smoke, cli.check_config, cli.check_auth, cli.config_path
            ),
        );
    }

    if cli.check_config {
        check_config(&cli.config_path)
            .unwrap_or_else(|error| exit_with_error(error, SETUP_ERROR_EXIT_CODE));
        return Ok(());
    }

    if cli.check_auth {
        let (config, session_path) = load_checked_config_with_session_parent(&cli.config_path)
            .unwrap_or_else(|error| exit_with_error(error, SETUP_ERROR_EXIT_CODE));
        color_eyre::install()?;
        check_auth(&cli.config_path, config, session_path).await?;
        return Ok(());
    }

    let theme = config::Theme::default();

    match cli.mode {
        RunMode::RealTelegram => {
            let (config, session_path) = load_checked_config_with_session_parent(&cli.config_path)
                .unwrap_or_else(|error| exit_with_error(error, SETUP_ERROR_EXIT_CODE));
            color_eyre::install()?;
            let preferences_path = preferences::state_path_for_config(&cli.config_path);
            run_real_telegram(config, session_path, &theme, Some(preferences_path)).await
        }
        RunMode::Mock => {
            color_eyre::install()?;
            run_mock(&theme, cli.smoke).await
        }
    }
}

fn exit_with_error(error: impl std::fmt::Display, code: i32) -> ! {
    eprintln!("Error: {error}");
    std::process::exit(code);
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
                    .filter(|path| !path.starts_with('-'))
                    .ok_or_else(|| color_eyre::eyre::eyre!(CONFIG_PATH_ARGUMENT_REQUIRED))?;
                cli.config_path = path;
            }
            "--log" => {
                let path = args
                    .next()
                    .filter(|path| !path.starts_with('-'))
                    .ok_or_else(|| color_eyre::eyre::eyre!(LOG_PATH_ARGUMENT_REQUIRED))?;
                cli.log_path = Some(path);
            }
            "--help" | "-h" => cli.help = true,
            _ => {
                return Err(color_eyre::eyre::eyre!(
                    "unknown argument: {}\nRun `{APP_COMMAND} --help` for usage.",
                    arg
                ));
            }
        }
    }

    if cli.smoke && cli.check_config {
        return Err(color_eyre::eyre::eyre!(SMOKE_CHECK_CONFIG_CONFLICT));
    }

    if cli.smoke && cli.check_auth {
        return Err(color_eyre::eyre::eyre!(SMOKE_CHECK_AUTH_CONFLICT));
    }

    if cli.check_config && cli.check_auth {
        return Err(color_eyre::eyre::eyre!(CHECK_CONFIG_AUTH_CONFLICT));
    }

    if cli.smoke {
        cli.mode = RunMode::Mock;
    }

    Ok(cli)
}

fn default_config_path_string() -> String {
    paths::default_config_path().to_string_lossy().into_owned()
}

fn print_help() {
    let default_config_path = default_config_path_string();
    println!(
        "Dumbgram TUI {APP_VERSION}\n\n\
Usage:\n  {APP_COMMAND} [OPTIONS]\n\n\
Options:\n  --mock             Run with built-in mock Telegram data for smoke testing\n  --smoke            Load mock data, render off-screen, exercise interactions, and exit\n  --check-config     Validate Telegram config and session path without connecting\n  --check-auth       Connect and verify saved Telegram session without login/TUI\n  -c, --config PATH  Load Telegram config from PATH (default: {default_config_path})\n  --log PATH         Append privacy-safe runtime diagnostics to PATH\n  -h, --help         Print this help\n\n\
Examples:\n  {APP_COMMAND} --mock\n  {APP_COMMAND} --mock --smoke\n  {APP_COMMAND} --check-config --config \"{default_config_path}\"\n  {APP_COMMAND} --check-auth --config \"{default_config_path}\"\n  {APP_COMMAND} --config \"{default_config_path}\""
    );
}

fn load_checked_config(config_path: &str) -> Result<config::Config> {
    let config = config::Config::load(config_path).map_err(|error| {
        color_eyre::eyre::eyre!(
            "failed to load {}: {}\n{}",
            config_path,
            error,
            CONFIG_LOAD_HELP
        )
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

    let session_path = config
        .telegram
        .session_path_for_config(Path::new(config_path));
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

fn load_checked_config_with_session_parent(config_path: &str) -> Result<(config::Config, String)> {
    let config = load_checked_config(config_path)?;
    let session_path = config
        .telegram
        .session_path_for_config(Path::new(config_path));
    ensure_session_parent_dir(&session_path)?;
    let session_path = session_path.to_string_lossy().into_owned();

    Ok((config, session_path))
}

fn check_config_session_status(session_path: &Path) -> &'static str {
    if session_path.exists() {
        CHECK_CONFIG_SESSION_EXISTS_STATUS
    } else {
        CHECK_CONFIG_SESSION_WILL_CREATE_STATUS
    }
}

fn check_config_message(config_path: &str, config: &config::Config, session_path: &Path) -> String {
    format!(
        "Config OK: {} (api_id={}, session_file={} [{}])",
        config_path,
        config.telegram.api_id,
        session_path.display(),
        check_config_session_status(session_path)
    )
}

fn check_config(config_path: &str) -> Result<()> {
    let config = load_checked_config(config_path)?;
    let session_path = config
        .telegram
        .session_path_for_config(Path::new(config_path));

    println!(
        "{}",
        check_config_message(config_path, &config, &session_path)
    );

    Ok(())
}

fn check_auth_ok_message(session_path: &str) -> String {
    format!("{CHECK_AUTH_OK_PREFIX} ({session_path})")
}

fn check_auth_unauthorized_message(config_path: &str) -> String {
    format!(
        "Telegram session is not authorized. Run `{APP_COMMAND} --config {config_path}` to log in."
    )
}

async fn check_auth(config_path: &str, config: config::Config, session_path: String) -> Result<()> {
    let client = GrammersClient::new(
        config.telegram.api_id,
        config.telegram.api_hash.clone(),
        &session_path,
    )
    .await?;

    if client.inner().is_authorized().await? {
        println!("{}", check_auth_ok_message(&session_path));
        Ok(())
    } else {
        Err(color_eyre::eyre::eyre!(check_auth_unauthorized_message(
            config_path
        )))
    }
}

async fn run_real_telegram(
    config: config::Config,
    session_path: String,
    theme: &config::Theme,
    preferences_path: Option<PathBuf>,
) -> Result<()> {
    diagnostics::event(
        "real_client_create_start",
        format!("session_path={session_path}"),
    );
    let started = Instant::now();
    let mut client = GrammersClient::new(
        config.telegram.api_id,
        config.telegram.api_hash.clone(),
        &session_path,
    )
    .await?;
    diagnostics::event(
        "real_client_create_finish",
        format!("elapsed_ms={}", started.elapsed().as_millis()),
    );

    let authorized = client.inner().is_authorized().await?;
    diagnostics::event("auth_status", format!("authorized={authorized}"));
    if !authorized {
        diagnostics::event("login_start", "authorized=false");
        login(&mut client).await?;
        diagnostics::event("login_finish", "authorized=true");
    }

    run_with_client(client, theme, preferences_path).await
}

fn login_code_sent_message(phone: &str) -> String {
    format!("{LOGIN_CODE_SENT_PREFIX} {phone}")
}

fn login_signed_in_message(first_name: &str) -> String {
    format!("{LOGIN_SIGNED_IN_PREFIX} {first_name}")
}

fn login_2fa_hint_message(hint: &str) -> String {
    format!("{LOGIN_2FA_HINT_PREFIX} {hint}")
}

fn login_2fa_signed_in_message(first_name: &str) -> String {
    format!("{LOGIN_2FA_SIGNED_IN_PREFIX} {first_name}")
}

fn login_failed_message(error: impl std::fmt::Display) -> String {
    format!("{LOGIN_FAILED_PREFIX}: {error}")
}

async fn login(client: &mut GrammersClient) -> Result<()> {
    println!("{LOGIN_HEADER}");

    let phone = prompt_input(LOGIN_PHONE_PROMPT)?;
    println!("{LOGIN_REQUESTING_CODE_STATUS}");
    let token = client.inner().request_login_code(&phone).await?;
    println!("{}\n", login_code_sent_message(&phone));

    let code = prompt_input(LOGIN_CODE_PROMPT)?;
    println!("{LOGIN_SIGNING_IN_STATUS}");

    match client.inner().sign_in(&token, &code).await {
        Ok(user) => {
            println!("{}\n", login_signed_in_message(user.first_name()));
            client.save_session()?;
        }
        Err(grammers_client::SignInError::PasswordRequired(password_token)) => {
            let hint = password_token.hint().unwrap_or("");
            println!("{LOGIN_2FA_ENABLED_STATUS}");
            if !hint.is_empty() {
                println!("{}", login_2fa_hint_message(hint));
            }

            let password = prompt_input_preserving_spaces(LOGIN_2FA_PROMPT)?;
            let user = client
                .inner()
                .check_password(password_token, password.as_bytes())
                .await?;
            println!("{}\n", login_2fa_signed_in_message(user.first_name()));
            client.save_session()?;
        }
        Err(e) => exit_with_error(login_failed_message(e), SETUP_ERROR_EXIT_CODE),
    }

    println!("{LOGIN_SESSION_SAVED_STATUS}");
    wait_for_enter_to_start()?;

    Ok(())
}

async fn run_mock(theme: &config::Theme, smoke: bool) -> Result<()> {
    if smoke {
        run_smoke_with_client(MockTelegramClient::new(), theme).await
    } else {
        run_with_client(MockTelegramClient::new(), theme, None).await
    }
}

async fn run_smoke_with_client<C: TelegramClient + Clone + Send + Sync + 'static>(
    mut client: C,
    theme: &config::Theme,
) -> Result<()> {
    client.connect().await?;

    let mut app = App::new();
    app.state.set_status(LOADING_TELEGRAM_STATUS);
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
    run_interaction_smoke(&mut app, &mut client, theme).await?;
    assert_smoke_render(&mut app, theme)?;
    run_mouse_smoke(&mut app, &mut client).await?;
    assert_smoke_render(&mut app, theme)?;
    run_lazy_preview_smoke(&mut app, &mut client, theme).await?;
    assert_smoke_render(&mut app, theme)?;

    println!(
        "{}",
        smoke_ok_message(
            app.state.folders.len(),
            app.state.chats.len(),
            app.state.messages.len()
        )
    );

    Ok(())
}

fn smoke_ok_message(folder_count: usize, chat_count: usize, message_count: usize) -> String {
    format!(
        "{SMOKE_OK_PREFIX}: rendered {folder_count} folders, {chat_count} chats, {message_count} messages and exercised keyboard/mouse interactions"
    )
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
    let backend = TestBackend::new(SMOKE_RENDER_WIDTH, SMOKE_RENDER_HEIGHT);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| ui::render_layout(frame, app, theme))?;

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert_smoke_folder_render(app, &rendered)?;
    assert_smoke_chat_render(app, &rendered)?;
    assert_smoke_message_render(app, &rendered)?;
    assert_smoke_typing_render(app, &rendered)?;
    assert_smoke_status_render(app, &rendered)?;
    assert_smoke_shell_render(&rendered)?;

    Ok(())
}

fn assert_smoke_folder_render(app: &App, rendered: &str) -> Result<()> {
    if !rendered.contains(ui::folders::FOLDER_PANEL_LABEL)
        || !rendered.contains(ui::folders::FOLDER_PANEL_TITLE.trim())
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the Folders panel"
        ));
    }
    let (visible_folders, has_left, has_right) = app.state.get_visible_folders();
    if visible_folders.len() > 1 && !rendered.contains(state::FOLDER_SEPARATOR) {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the folder separator glyph"
        ));
    }
    if has_left && !rendered.contains(state::FOLDER_LEFT_SCROLL_INDICATOR) {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the folder left-scroll glyph"
        ));
    }
    if has_right && !rendered.contains(state::FOLDER_RIGHT_SCROLL_INDICATOR) {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the folder right-scroll glyph"
        ));
    }

    Ok(())
}

fn assert_smoke_chat_render(app: &App, rendered: &str) -> Result<()> {
    if !rendered.contains(ui::chats::CHAT_PANEL_LABEL) {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the Chats panel"
        ));
    }

    let chat_position =
        ui::chats::chat_panel_title(app.state.selected_chat_index, app.state.chats.len());
    if !rendered.contains(chat_position.trim()) {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the chat position indicator"
        ));
    }
    if app.state.chats.is_empty() {
        return Ok(());
    }

    if !rendered.contains(ui::SELECTED_ROW_SYMBOL)
        || rendered.contains(ui::LEGACY_SELECTED_ROW_SYMBOL)
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the restored selected-row marker"
        ));
    }
    let has_rendered_unread_indicator = app.state.chats.iter().any(|chat| {
        let indicator = ui::chats::chat_unread_indicator(chat.unread_count);
        !indicator.is_empty() && rendered.contains(&indicator)
    });
    if app.state.chats.iter().any(|chat| chat.unread_count > 0) && !has_rendered_unread_indicator {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the plain unread chat indicator"
        ));
    }
    let has_rendered_group_indicator = app.state.chats.iter().any(|chat| {
        let indicator = ui::chats::chat_group_indicator(chat.is_group);
        !indicator.is_empty() && rendered.contains(indicator)
    });
    if app.state.chats.iter().any(|chat| chat.is_group) && !has_rendered_group_indicator {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the plain group chat indicator"
        ));
    }
    if rendered.contains("[G]") {
        return Err(color_eyre::eyre::eyre!(
            "smoke render still included the legacy group chat badge"
        ));
    }

    Ok(())
}

fn assert_smoke_message_render(app: &App, rendered: &str) -> Result<()> {
    if app.state.messages.is_empty() {
        return Ok(());
    }
    if app
        .state
        .selected_message()
        .and_then(|message| message.media.as_ref())
        .and_then(|media| media.local_image_path())
        .is_some()
        && !rendered.contains(ui::layout::IMAGE_VIEWPORT_TITLE)
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the lazily loaded image viewport"
        ));
    }

    let message_position = ui::messages::message_position_label(
        app.state.selected_message_index,
        app.state.messages.len(),
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
        && !rendered.contains(ui::messages::REPLY_MARKER)
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the restored reply marker"
        ));
    }
    let edited_metadata = format!(
        "{}{}",
        ui::messages::MESSAGE_METADATA_SEPARATOR,
        ui::messages::EDITED_METADATA_LABEL
    );
    if app.state.messages.iter().any(|message| message.is_edited)
        && !rendered.contains(&edited_metadata)
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include edited metadata with Unicode separator"
        ));
    }
    for status_label in app.state.messages.iter().filter_map(|message| {
        let label = ui::messages::message_status_label(&message.status, message.is_own);
        (!label.is_empty()).then_some(label)
    }) {
        let status_metadata = format!("{}{status_label}", ui::messages::MESSAGE_METADATA_SEPARATOR);
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
    if app.state.delete_confirmation().is_some()
        && (!rendered.contains(ui::messages::DELETE_CONFIRMATION_TEXT.trim())
            || !rendered.contains(ui::messages::DELETE_CONFIRMATION_TITLE.trim()))
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the delete confirmation popup"
        ));
    }

    Ok(())
}

fn assert_smoke_typing_render(app: &App, rendered: &str) -> Result<()> {
    if let Some(users) = app
        .state
        .selected_typing_users()
        .filter(|users| !users.is_empty())
    {
        let typing_label = ui::messages::typing_label(users);
        if !rendered.contains(&typing_label) {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the selected chat typing indicator with Unicode separator"
            ));
        }
    }

    Ok(())
}

fn assert_smoke_status_render(app: &App, rendered: &str) -> Result<()> {
    if let Some(status) = app.state.status_message.as_ref() {
        let status_banner = format!("{} {}", ui::layout::STATUS_BANNER_PREFIX, status);
        if !rendered.contains(&status_banner) {
            return Err(color_eyre::eyre::eyre!(
                "smoke render did not include the status banner"
            ));
        }
    }

    Ok(())
}

fn assert_smoke_shell_render(rendered: &str) -> Result<()> {
    if !rendered.contains(ui::input::INPUT_TITLE.trim())
        && !rendered.contains(ui::input::TYPE_MESSAGE_TITLE.trim())
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the input panel"
        ));
    }
    if !rendered.contains(ui::layout::FOCUS_LABEL_PREFIX) {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the controls help bar"
        ));
    }
    if !rendered.contains(ui::layout::HELP_SEPARATOR)
        || rendered.contains(ui::layout::LEGACY_HELP_SEPARATOR)
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke render did not include the restored help-bar Unicode separators"
        ));
    }

    Ok(())
}

async fn run_lazy_preview_smoke<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    client: &mut C,
    theme: &config::Theme,
) -> Result<()> {
    actions::load_initial_state(&mut app.state, client).await?;
    let chat_index = app
        .state
        .chats
        .iter()
        .position(|chat| chat.id == 4)
        .ok_or_else(|| color_eyre::eyre::eyre!("smoke data did not include image chat"))?;
    actions::begin_open_chat_at(&mut app.state, chat_index);
    actions::load_selected_chat_messages(&mut app.state, client).await?;
    app.state.selected_message_index = 0;
    let (chat_id, message_id) = app
        .state
        .selected_media_preview_request()
        .ok_or_else(|| color_eyre::eyre::eyre!("smoke image was not text-first"))?;
    assert_smoke_render(app, theme)?;

    let path = client
        .load_message_media_preview(chat_id, message_id)
        .await?
        .ok_or_else(|| color_eyre::eyre::eyre!("mock preview was unavailable"))?;
    if !app
        .state
        .apply_selected_media_preview(chat_id, message_id, path)
    {
        return Err(color_eyre::eyre::eyre!(
            "smoke preview did not attach to selected message"
        ));
    }
    Ok(())
}

async fn run_interaction_smoke<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    client: &mut C,
    theme: &config::Theme,
) -> Result<()> {
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

    handle_key_event(app, smoke_key(KeyCode::Up), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Up), client).await?;
    if app.state.selected_chat_index != 0 || app.state.focused_panel != state::FocusedPanel::Chats {
        return Err(color_eyre::eyre::eyre!(
            "Up at the first chat changed focus or selection"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Down), client).await?;

    let active_messages = app
        .state
        .messages
        .iter()
        .map(|message| message.id)
        .collect::<Vec<_>>();
    let active_load_status = app.state.conversation_load_status;
    handle_key_event(app, smoke_key(KeyCode::Char('/')), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Down), client).await?;
    if app.state.selected_chat_index != 1
        || app.state.selected_chat_search_result_index() != Some(2)
        || app
            .state
            .messages
            .iter()
            .map(|message| message.id)
            .ne(active_messages.iter().copied())
        || app.state.conversation_load_status != active_load_status
    {
        return Err(color_eyre::eyre::eyre!(
            "chat search browsing changed the open conversation"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Esc), client).await?;
    if app.state.chat_search_active() || app.state.selected_chat_index != 1 {
        return Err(color_eyre::eyre::eyre!(
            "cancelling chat search changed the open conversation"
        ));
    }

    handle_key_event(app, smoke_key(KeyCode::Char('/')), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Down), client).await?;
    handle_key_event(app, smoke_key(KeyCode::Enter), client).await?;
    if app.state.chat_search_active() || app.state.selected_chat_index != 2 {
        return Err(color_eyre::eyre::eyre!(
            "committing chat search did not open exactly the browsed chat"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Char('/')), client).await?;
    for character in "bob".chars() {
        handle_key_event(app, smoke_key(KeyCode::Char(character)), client).await?;
    }
    handle_key_event(app, smoke_key(KeyCode::Enter), client).await?;
    if app.state.chat_search_active() || app.state.selected_chat_index != 1 {
        return Err(color_eyre::eyre::eyre!(
            "search commit did not return to the second mock chat"
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
    assert_smoke_render(app, theme)?;

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
    if app.state.delete_confirmation().is_none_or(|confirmation| {
        confirmation.message_id != 2 || confirmation.chat_id != app.state.chats[1].id
    }) {
        return Err(color_eyre::eyre::eyre!(
            "delete shortcut did not request confirmation"
        ));
    }
    assert_smoke_render(app, theme)?;
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
    if !app.state.selected_message_is_last() {
        return Err(color_eyre::eyre::eyre!(
            "End did not jump to the last message"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Down), client).await?;
    if app.state.focused_panel != state::FocusedPanel::Messages
        || !app.state.selected_message_is_last()
    {
        return Err(color_eyre::eyre::eyre!(
            "Down at the last message changed focus or selection"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Home), client).await?;
    if app.state.selected_message_index != 0 || app.state.message_scroll_offset != 0 {
        return Err(color_eyre::eyre::eyre!(
            "Home did not jump to the first message and reset message scroll"
        ));
    }

    if app.state.selected_chat_id() != Some(3) || app.state.thread_topics.is_empty() {
        return Err(color_eyre::eyre::eyre!(
            "threaded mock chat did not expose topic tabs before topic-open smoke"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Right), client).await?;
    if app.state.selected_thread_topic_index != 1 {
        return Err(color_eyre::eyre::eyre!(
            "Right in messages did not open the next mock topic"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Left), client).await?;
    if app.state.selected_thread_topic_index != 0
        || app.state.messages.is_empty()
        || !app
            .state
            .messages
            .iter()
            .all(|message| message.chat_id == 3 && message.thread_topic_id == Some(101))
    {
        return Err(color_eyre::eyre::eyre!(
            "Left in messages did not return to the first mock topic"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Enter), client).await?;
    if app.state.focused_panel != state::FocusedPanel::Input {
        return Err(color_eyre::eyre::eyre!(
            "Enter in messages did not focus input"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Input;
    app.state.input_buffer = "topic smoke send".to_string();
    app.state.move_input_cursor_to_end();
    handle_key_event(app, smoke_key(KeyCode::Enter), client).await?;
    let selected_sent_topic_message = app
        .state
        .messages
        .get(app.state.selected_message_index)
        .ok_or_else(|| color_eyre::eyre::eyre!("topic send did not select a message"))?;
    if selected_sent_topic_message.content != "topic smoke send"
        || selected_sent_topic_message.chat_id != 3
        || selected_sent_topic_message.thread_topic_id != Some(101)
        || selected_sent_topic_message.status != telegram::types::MessageStatus::Sent
    {
        return Err(color_eyre::eyre::eyre!(
            "topic send interaction did not append/select a sent topic-scoped mock message"
        ));
    }

    app.state.focused_panel = state::FocusedPanel::Messages;
    handle_key_event(app, smoke_key(KeyCode::Char('r')), client).await?;
    if app.state.focused_panel != state::FocusedPanel::Input
        || app.state.replying_to_message_id.is_none()
    {
        return Err(color_eyre::eyre::eyre!(
            "topic reply shortcut did not enter input reply mode"
        ));
    }
    app.state.input_buffer = "topic smoke reply".to_string();
    app.state.move_input_cursor_to_end();
    handle_key_event(app, smoke_key(KeyCode::Enter), client).await?;
    let selected_topic_reply = app
        .state
        .messages
        .get(app.state.selected_message_index)
        .ok_or_else(|| color_eyre::eyre::eyre!("topic reply did not select a message"))?;
    if selected_topic_reply.content != "topic smoke reply"
        || selected_topic_reply.chat_id != 3
        || selected_topic_reply.thread_topic_id != Some(101)
        || selected_topic_reply.reply_to_content.as_deref() != Some("topic 101 reply 1101")
        || selected_topic_reply.status != telegram::types::MessageStatus::Sent
        || app.state.replying_to_message_id.is_some()
    {
        return Err(color_eyre::eyre::eyre!(
            "topic reply interaction did not append/select a sent topic-scoped reply"
        ));
    }

    Ok(())
}

async fn run_mouse_smoke<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    client: &mut C,
) -> Result<()> {
    if app.state.chats_area.width < 4 || app.state.chats_area.height < 4 {
        return Err(color_eyre::eyre::eyre!(
            "smoke layout did not expose a clickable chat list"
        ));
    }

    if app.state.selected_chat_id() != Some(3) || app.state.thread_topics.len() < 2 {
        return Err(color_eyre::eyre::eyre!(
            "mouse smoke did not start with mock topic tabs available"
        ));
    }
    let topic_click_column = (0..app.state.thread_topics_area.width.saturating_sub(2) as usize)
        .find(|column| app.state.thread_topic_index_at_visible_column(*column) == Some(1))
        .ok_or_else(|| color_eyre::eyre::eyre!("second topic tab was not clickable"))?;
    let topic_click = smoke_click(
        app.state.thread_topics_area.x + 1 + topic_click_column as u16,
        app.state.thread_topics_area.y + 1,
    );
    handle_mouse_event(app, topic_click, client).await?;
    if app.state.selected_thread_topic_index != 1
        || app.state.messages.is_empty()
        || !app
            .state
            .messages
            .iter()
            .all(|message| message.chat_id == 3 && message.thread_topic_id == Some(102))
    {
        return Err(color_eyre::eyre::eyre!(
            "mouse topic tab click did not load the clicked mock topic history"
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

    app.state.chats_area.height = 6;
    let chat_scroll_down = smoke_mouse(
        MouseEventKind::ScrollDown,
        app.state.chats_area.x + 2,
        app.state.chats_area.y + 1,
    );
    handle_mouse_event(app, chat_scroll_down, client).await?;
    if app.state.focused_panel != state::FocusedPanel::Chats
        || app.state.selected_chat_index != 0
        || app.state.chat_scroll_offset != 1
        || app.state.messages.len() != 3
    {
        return Err(color_eyre::eyre::eyre!(
            "mouse wheel over chats should scroll without loading another chat"
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

    let message_menu = smoke_mouse(
        MouseEventKind::Down(MouseButton::Right),
        app.state.messages_area.x + 2,
        app.state.messages_area.y + 2,
    );
    handle_mouse_event(app, message_menu, client).await?;
    if app.state.context_menu().is_none() {
        return Err(color_eyre::eyre::eyre!(
            "right-click did not open a message context menu"
        ));
    }
    let blocked_offset = app.state.chat_scroll_offset;
    handle_mouse_event(app, chat_scroll_down, client).await?;
    if app.state.chat_scroll_offset != blocked_offset {
        return Err(color_eyre::eyre::eyre!(
            "context menu leaked an underlying chat wheel event"
        ));
    }
    handle_key_event(app, smoke_key(KeyCode::Esc), client).await?;
    if app.state.context_menu().is_some() {
        return Err(color_eyre::eyre::eyre!(
            "Escape did not close the context menu"
        ));
    }

    let original_split_ratio = app.state.split_ratio;
    let divider_column = app.state.chats_area.x + app.state.chats_area.width - 1;
    let divider_row = app.state.chats_area.y + 1;
    let drag_column = if original_split_ratio < 0.5 {
        app.state.screen_area.x + app.state.screen_area.width * 3 / 4
    } else {
        app.state.screen_area.x + app.state.screen_area.width / 4
    };
    handle_mouse_event(
        app,
        smoke_mouse(
            MouseEventKind::Down(MouseButton::Left),
            divider_column,
            divider_row,
        ),
        client,
    )
    .await?;
    handle_mouse_event(
        app,
        smoke_mouse(MouseEventKind::Drag(MouseButton::Left), drag_column, 10),
        client,
    )
    .await?;
    handle_mouse_event(
        app,
        smoke_mouse(MouseEventKind::Up(MouseButton::Left), drag_column, 10),
        client,
    )
    .await?;
    if app.state.split_drag_active || app.state.split_ratio == original_split_ratio {
        return Err(color_eyre::eyre::eyre!(
            "mouse divider drag did not resize and release: active={} before={} after={} divider=({}, {}) drag_column={}",
            app.state.split_drag_active,
            original_split_ratio,
            app.state.split_ratio,
            divider_column,
            divider_row,
            drag_column,
        ));
    }
    app.state.split_ratio = original_split_ratio;

    if let Some(confirmation) =
        app.state
            .selected_message()
            .map(|message| state::DeleteConfirmation {
                chat_id: message.chat_id,
                message_id: message.id,
            })
    {
        app.state.set_delete_confirmation(confirmation);
    }
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
    app.state.cancel_delete_confirmation();
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

async fn run_with_client<C: TelegramClient + Clone + Send + Sync + 'static>(
    mut client: C,
    theme: &config::Theme,
    preferences_path: Option<PathBuf>,
) -> Result<()> {
    diagnostics::event("client_connect_start", "client=telegram");
    let started = Instant::now();
    client.connect().await?;
    diagnostics::event(
        "client_connect_finish",
        format!("elapsed_ms={}", started.elapsed().as_millis()),
    );

    let mut app = App::new();
    app.preferences_path = preferences_path;
    load_app_preferences(&mut app);
    let mut terminal = setup_terminal()?;

    app.state.set_status(LOADING_TELEGRAM_STATUS);
    let result = run_app(&mut terminal, &mut app, theme, &mut client).await;

    diagnostics::event("terminal_restore_start", "event_stream_stopped=true");
    let restore_result = restore_terminal(&mut terminal);
    diagnostics::event(
        "terminal_restore_finish",
        format!("success={}", restore_result.is_ok()),
    );
    match (result, restore_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn trim_prompt_input_line(line: &str) -> String {
    strip_prompt_line_ending(line).trim().to_string()
}

fn preserve_prompt_input_line_spaces(line: &str) -> String {
    strip_prompt_line_ending(line).to_string()
}

fn strip_prompt_line_ending(line: &str) -> &str {
    line.strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .or_else(|| line.strip_suffix('\r'))
        .unwrap_or(line)
}

fn require_prompt_line(bytes_read: usize, line: String) -> Result<String> {
    if bytes_read == 0 {
        Err(color_eyre::eyre::eyre!(PROMPT_EOF_ERROR))
    } else {
        Ok(line)
    }
}

fn require_prompt_response(response: String) -> Result<String> {
    if response.is_empty() {
        Err(color_eyre::eyre::eyre!(PROMPT_EMPTY_ERROR))
    } else {
        Ok(response)
    }
}

fn read_prompt_line_raw(msg: &str) -> Result<(usize, String)> {
    use std::io::{self, Write};
    print!("{}", msg);
    io::stdout().flush()?;
    let mut line = String::new();
    let bytes_read = io::stdin().read_line(&mut line)?;
    Ok((bytes_read, line))
}

fn read_prompt_line(msg: &str) -> Result<String> {
    let (bytes_read, line) = read_prompt_line_raw(msg)?;
    require_prompt_line(bytes_read, line)
}

fn prompt_input(msg: &str) -> Result<String> {
    let response = read_prompt_line(msg).map(|line| trim_prompt_input_line(&line))?;
    require_prompt_response(response)
}

fn prompt_input_preserving_spaces(msg: &str) -> Result<String> {
    let response = read_prompt_line(msg).map(|line| preserve_prompt_input_line_spaces(&line))?;
    require_prompt_response(response)
}

fn wait_for_enter_to_start() -> Result<()> {
    read_prompt_line_raw(LOGIN_START_PROMPT).map(|_| ())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let clear_images_result = terminal_images::clear_terminal_images(terminal.backend_mut());
    let raw_mode_result = disable_raw_mode();
    let leave_screen_result = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let cursor_result = terminal.show_cursor();

    clear_images_result?;
    raw_mode_result?;
    leave_screen_result?;
    cursor_result?;
    Ok(())
}

fn load_app_preferences(app: &mut App) {
    let Some(path) = app.preferences_path.as_deref() else {
        return;
    };

    match preferences::AppPreferences::load(path) {
        Ok(preferences) => {
            preferences.apply_to_state(&mut app.state);
            diagnostics::event("preferences_load", format!("path={}", path.display()));
        }
        Err(error) => diagnostics::event(
            "preferences_load_error",
            format!("path={} error={error}", path.display()),
        ),
    }
}

fn save_app_preferences(app: &mut App) {
    let Some(path) = app.preferences_path.as_deref() else {
        return;
    };

    match preferences::AppPreferences::from_state(&app.state).save(path) {
        Ok(()) => diagnostics::event("preferences_save", format!("path={}", path.display())),
        Err(error) => {
            diagnostics::event(
                "preferences_save_error",
                format!("path={} error={error}", path.display()),
            );
            app.state
                .set_error(format!("Save preferences failed: {error}"));
        }
    }
}

fn save_app_preferences_if_changed(app: &mut App, before: preferences::AppPreferences) {
    if before != preferences::AppPreferences::from_state(&app.state) {
        save_app_preferences(app);
    }
}

async fn run_app<C: TelegramClient + Clone + Send + Sync + 'static>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    theme: &config::Theme,
    client: &mut C,
) -> Result<()> {
    diagnostics::event("run_loop_start", "event_driven=true");
    let (mut loop_state, senders) = EventLoopState::new();
    let mut subscribe_updates_loader =
        SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
    subscribe_updates_loader.spawn_subscribe_updates();
    let initial_state_loader = InitialStateLoader::new(client.clone(), senders.initial_state);
    initial_state_loader.spawn_initial_state();
    let mut chat_message_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
    let mut older_message_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
    let mut folder_chat_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
    let mark_read_loader = MarkChatReadLoader::new(client.clone());
    let send_message_loader = SendMessageLoader::new(client.clone(), senders.send_message);
    let delete_message_loader = DeleteMessageLoader::new(client.clone(), senders.delete_message);
    let edit_message_loader = EditMessageLoader::new(client.clone(), senders.edit_message);
    let reply_message_loader = ReplyMessageLoader::new(client.clone(), senders.reply_message);
    let download_media_loader = DownloadMediaLoader::new(client.clone(), senders.download_media);
    let mut media_preview_loader = MediaPreviewLoader::new(client.clone(), senders.media_preview);
    let mut reconciliation_loader =
        ReconciliationLoader::new(client.clone(), senders.reconciliation);
    let mut events = EventStream::new();

    let result = run_event_loop(
        terminal,
        app,
        theme,
        client,
        &mut events,
        &mut loop_state,
        &mut subscribe_updates_loader,
        &mut reconciliation_loader,
        &mut chat_message_loader,
        &mut older_message_loader,
        &mut folder_chat_loader,
        &mark_read_loader,
        &send_message_loader,
        &delete_message_loader,
        &edit_message_loader,
        &reply_message_loader,
        &download_media_loader,
        &mut media_preview_loader,
    )
    .await;

    drop(events);
    diagnostics::event("terminal_event_stream_stopped", "before_restore=true");
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_event_loop<C: TelegramClient + Clone + Send + Sync + 'static>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    theme: &config::Theme,
    client: &mut C,
    events: &mut EventStream,
    loop_state: &mut EventLoopState,
    subscribe_updates_loader: &mut SubscribeUpdatesLoader<C>,
    reconciliation_loader: &mut ReconciliationLoader<C>,
    chat_message_loader: &mut ChatMessageLoader<C>,
    older_message_loader: &mut OlderMessageLoader<C>,
    folder_chat_loader: &mut FolderChatLoader<C>,
    mark_read_loader: &MarkChatReadLoader<C>,
    send_message_loader: &SendMessageLoader<C>,
    delete_message_loader: &DeleteMessageLoader<C>,
    edit_message_loader: &EditMessageLoader<C>,
    reply_message_loader: &ReplyMessageLoader<C>,
    download_media_loader: &DownloadMediaLoader<C>,
    media_preview_loader: &mut MediaPreviewLoader<C>,
) -> Result<()> {
    let mut frames = FrameScheduler::new(true);
    draw_due_frame(terminal, app, theme, &mut frames)?;
    loop {
        let now = TokioInstant::now();
        if loop_state
            .next_subscription_at
            .is_some_and(|deadline| deadline <= now)
            && !loop_state.subscription_pending
        {
            loop_state.next_subscription_at = None;
            loop_state.subscription_pending = true;
            subscribe_updates_loader.spawn_subscribe_updates();
        }
        if loop_state
            .next_reconciliation_at
            .is_some_and(|deadline| deadline <= now)
            && !loop_state.reconciliation_pending
            && !loop_state.initial_state_pending
        {
            loop_state.next_reconciliation_at = None;
            loop_state.reconciliation_pending = true;
            reconciliation_loader.spawn_reconciliation(app.state.reconciliation_context());
        }

        let step = prepare_loop_step(
            loop_state,
            app,
            subscribe_updates_loader,
            reconciliation_loader,
            chat_message_loader,
            older_message_loader,
            folder_chat_loader,
            mark_read_loader,
            media_preview_loader,
        );
        if step.dirty {
            frames.mark_dirty(TokioInstant::now());
        }
        if let Some(event) = step.terminal_event
            && dispatch_terminal_event(
                terminal,
                app,
                client,
                event,
                loop_state,
                chat_message_loader,
                older_message_loader,
                folder_chat_loader,
                mark_read_loader,
                send_message_loader,
                delete_message_loader,
                edit_message_loader,
                reply_message_loader,
                download_media_loader,
            )
            .await?
        {
            frames.mark_dirty(TokioInstant::now());
        }

        media_preview_loader.request(app.state.selected_media_preview_request());

        if app.should_quit {
            diagnostics::event("run_loop_quit", "should_quit=true");
            return Ok(());
        }

        draw_due_frame(terminal, app, theme, &mut frames)?;

        let service_deadline = loop_state.service_deadline();
        match wait_for_loop_wake(
            events,
            &loop_state.wake,
            &mut loop_state.update_rx,
            frames.frame_deadline(),
            app.state.notification_deadline(),
            service_deadline,
        )
        .await?
        {
            LoopWake::Notify | LoopWake::Deadline => {}
            LoopWake::Terminal(event) => loop_state.staged_terminal_event = Some(event),
            LoopWake::Update(update) => loop_state.staged_update = Some(update),
            LoopWake::UpdateStreamClosed => {
                loop_state.update_rx = None;
                loop_state.subscription_pending = false;
                loop_state.announce_reconciliation_success = true;
                loop_state.schedule_subscription_retry();
                loop_state.schedule_reconciliation_now();
                app.state
                    .set_error(TELEGRAM_UPDATES_DISCONNECTED_ERROR.to_string());
                frames.mark_dirty(TokioInstant::now());
            }
        }
    }
}

struct PreparedLoopStep {
    dirty: bool,
    terminal_event: Option<Event>,
}

#[allow(clippy::too_many_arguments)]
fn prepare_loop_step<C: TelegramClient + Clone + Send + Sync + 'static>(
    loop_state: &mut EventLoopState,
    app: &mut App,
    subscribe_updates_loader: &SubscribeUpdatesLoader<C>,
    reconciliation_loader: &ReconciliationLoader<C>,
    chat_message_loader: &ChatMessageLoader<C>,
    older_message_loader: &OlderMessageLoader<C>,
    folder_chat_loader: &FolderChatLoader<C>,
    mark_read_loader: &MarkChatReadLoader<C>,
    media_preview_loader: &MediaPreviewLoader<C>,
) -> PreparedLoopStep {
    let results_dirty = drain_ready_results(
        loop_state,
        app,
        subscribe_updates_loader,
        reconciliation_loader,
        chat_message_loader,
        older_message_loader,
        folder_chat_loader,
        mark_read_loader,
        media_preview_loader,
    );
    PreparedLoopStep {
        dirty: app.state.check_notification_timeout() || results_dirty,
        terminal_event: loop_state.staged_terminal_event.take(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_terminal_event<C: TelegramClient + Clone + Send + Sync + 'static>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    client: &mut C,
    event: Event,
    loop_state: &mut EventLoopState,
    chat_message_loader: &mut ChatMessageLoader<C>,
    older_message_loader: &mut OlderMessageLoader<C>,
    folder_chat_loader: &mut FolderChatLoader<C>,
    mark_read_loader: &MarkChatReadLoader<C>,
    send_message_loader: &SendMessageLoader<C>,
    delete_message_loader: &DeleteMessageLoader<C>,
    edit_message_loader: &EditMessageLoader<C>,
    reply_message_loader: &ReplyMessageLoader<C>,
    download_media_loader: &DownloadMediaLoader<C>,
) -> Result<bool> {
    match classify_terminal_event(event) {
        TerminalAction::Key(key) => {
            let mut progress = UiProgress::Live { terminal };
            handle_key_event_with_progress(
                app,
                key,
                client,
                &mut progress,
                HandlerLoaders {
                    chat_message: Some(chat_message_loader),
                    older_message: Some(older_message_loader),
                    folder_chat: Some(folder_chat_loader),
                    mark_read: Some(mark_read_loader),
                    send_message: Some(send_message_loader),
                    delete_message: Some(delete_message_loader),
                    edit_message: Some(edit_message_loader),
                    reply_message: Some(reply_message_loader),
                    download_media: Some(download_media_loader),
                },
            )
            .await?;
            Ok(true)
        }
        TerminalAction::Mouse(mouse_event) => {
            let split_drag_was_active = app.state.split_drag_active;
            let mut progress = UiProgress::Live { terminal };
            handle_mouse_event_with_progress(
                app,
                mouse_event,
                client,
                &mut progress,
                HandlerLoaders {
                    chat_message: Some(chat_message_loader),
                    older_message: Some(older_message_loader),
                    folder_chat: Some(folder_chat_loader),
                    mark_read: Some(mark_read_loader),
                    send_message: Some(send_message_loader),
                    delete_message: Some(delete_message_loader),
                    edit_message: Some(edit_message_loader),
                    reply_message: Some(reply_message_loader),
                    download_media: Some(download_media_loader),
                },
            )
            .await?;
            if split_drag_was_active
                && matches!(mouse_event.kind, MouseEventKind::Up(MouseButton::Left))
            {
                save_app_preferences(app);
            }
            Ok(true)
        }
        TerminalAction::Resize => Ok(true),
        TerminalAction::FocusLost => {
            if app.state.split_drag_active {
                app.state.end_split_drag();
                save_app_preferences(app);
            }
            Ok(true)
        }
        TerminalAction::FocusGained => {
            loop_state.schedule_focus_reconciliation();
            Ok(false)
        }
        TerminalAction::Ignore => Ok(false),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalAction {
    Key(KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Resize,
    FocusLost,
    FocusGained,
    Ignore,
}

fn classify_terminal_event(event: Event) -> TerminalAction {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => TerminalAction::Key(key),
        Event::Mouse(mouse) => TerminalAction::Mouse(mouse),
        Event::Resize(_, _) => TerminalAction::Resize,
        Event::FocusLost => TerminalAction::FocusLost,
        Event::FocusGained => TerminalAction::FocusGained,
        Event::Key(_) | Event::Paste(_) => TerminalAction::Ignore,
    }
}

enum LoopWake {
    Notify,
    Terminal(Event),
    Update(Update),
    UpdateStreamClosed,
    Deadline,
}

async fn wait_for_loop_wake(
    events: &mut EventStream,
    wake: &tokio::sync::Notify,
    update_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<Update>>,
    frame_deadline: Option<TokioInstant>,
    notification_deadline: Option<TokioInstant>,
    service_deadline: Option<TokioInstant>,
) -> Result<LoopWake> {
    tokio::select! {
        _ = wake.notified() => Ok(LoopWake::Notify),
        event = next_terminal_event(events) => match event {
            Some(Ok(event)) => Ok(LoopWake::Terminal(event)),
            Some(Err(error)) => Err(error.into()),
            None => Err(color_eyre::eyre::eyre!("terminal event stream ended")),
        },
        update = receive_update(update_rx) => Ok(match update {
            Some(update) => LoopWake::Update(update),
            None => LoopWake::UpdateStreamClosed,
        }),
        _ = sleep_until_optional(frame_deadline) => Ok(LoopWake::Deadline),
        _ = sleep_until_optional(notification_deadline) => Ok(LoopWake::Deadline),
        _ = sleep_until_optional(service_deadline) => Ok(LoopWake::Deadline),
    }
}

async fn next_terminal_event(
    events: &mut EventStream,
) -> Option<std::result::Result<Event, io::Error>> {
    poll_fn(|context| Pin::new(&mut *events).poll_next(context)).await
}

async fn receive_update(
    update_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<Update>>,
) -> Option<Update> {
    match update_rx {
        Some(rx) => rx.recv().await,
        None => pending().await,
    }
}

async fn sleep_until_optional(deadline: Option<TokioInstant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => pending().await,
    }
}

fn draw_due_frame(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    theme: &config::Theme,
    frames: &mut FrameScheduler,
) -> Result<bool> {
    let Some(scheduling_delay) = frames.take_due_frame(TokioInstant::now()) else {
        return Ok(false);
    };

    let draw_started = Instant::now();
    terminal.draw(|frame| ui::render_layout(frame, app, theme))?;
    terminal_images::render_selected_image(terminal.backend_mut(), app)?;
    let draw_duration = draw_started.elapsed();
    diagnostics::event(
        "frame_draw",
        format!(
            "schedule_ms={} draw_ms={}",
            scheduling_delay.as_millis(),
            draw_duration.as_millis()
        ),
    );
    log_draw_duration("main_loop", draw_started);
    Ok(true)
}

struct FrameScheduler {
    dirty: bool,
    dirty_since: Option<TokioInstant>,
    last_draw_at: Option<TokioInstant>,
}

impl FrameScheduler {
    fn new(dirty: bool) -> Self {
        Self {
            dirty,
            dirty_since: dirty.then(TokioInstant::now),
            last_draw_at: None,
        }
    }

    fn mark_dirty(&mut self, now: TokioInstant) {
        if !self.dirty {
            self.dirty_since = Some(now);
        }
        self.dirty = true;
    }

    fn frame_deadline(&self) -> Option<TokioInstant> {
        self.dirty.then(|| {
            self.last_draw_at
                .map_or_else(TokioInstant::now, |last| last + MIN_FRAME_INTERVAL)
        })
    }

    fn take_due_frame(&mut self, now: TokioInstant) -> Option<Duration> {
        if !self.dirty
            || self
                .last_draw_at
                .is_some_and(|last| now < last + MIN_FRAME_INTERVAL)
        {
            return None;
        }

        self.dirty = false;
        self.last_draw_at = Some(now);
        Some(
            self.dirty_since
                .take()
                .map_or(Duration::ZERO, |dirty_since| {
                    now.saturating_duration_since(dirty_since)
                }),
        )
    }
}

fn ui_channel<T>(
    wake: &Arc<tokio::sync::Notify>,
) -> (UiSender<T>, tokio::sync::mpsc::UnboundedReceiver<T>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (
        UiSender {
            tx,
            wake: Arc::clone(wake),
        },
        rx,
    )
}

struct UiSender<T> {
    tx: tokio::sync::mpsc::UnboundedSender<T>,
    wake: Arc<tokio::sync::Notify>,
}

impl<T> Clone for UiSender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            wake: Arc::clone(&self.wake),
        }
    }
}

impl<T> UiSender<T> {
    fn send(&self, value: T) -> std::result::Result<(), tokio::sync::mpsc::error::SendError<T>> {
        self.tx.send(value)?;
        self.wake.notify_one();
        Ok(())
    }
}

impl<T> From<tokio::sync::mpsc::UnboundedSender<T>> for UiSender<T> {
    fn from(tx: tokio::sync::mpsc::UnboundedSender<T>) -> Self {
        Self {
            tx,
            wake: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

struct EventLoopSenders {
    subscribe_updates: UiSender<SubscribeUpdatesResult>,
    initial_state: UiSender<InitialStateLoadResult>,
    chat_message: UiSender<ChatMessageLoadResult>,
    older_message: UiSender<OlderMessageLoadResult>,
    folder_chat: UiSender<FolderChatLoadResult>,
    send_message: UiSender<SendMessageResult>,
    delete_message: UiSender<DeleteMessageResult>,
    edit_message: UiSender<EditMessageResult>,
    reply_message: UiSender<ReplyMessageResult>,
    download_media: UiSender<DownloadMediaResult>,
    media_preview: UiSender<MediaPreviewResult>,
    reconciliation: UiSender<ReconciliationResult>,
}

struct EventLoopState {
    wake: Arc<tokio::sync::Notify>,
    subscribe_updates_rx: tokio::sync::mpsc::UnboundedReceiver<SubscribeUpdatesResult>,
    initial_state_rx: tokio::sync::mpsc::UnboundedReceiver<InitialStateLoadResult>,
    chat_message_rx: tokio::sync::mpsc::UnboundedReceiver<ChatMessageLoadResult>,
    older_message_rx: tokio::sync::mpsc::UnboundedReceiver<OlderMessageLoadResult>,
    folder_chat_rx: tokio::sync::mpsc::UnboundedReceiver<FolderChatLoadResult>,
    send_message_rx: tokio::sync::mpsc::UnboundedReceiver<SendMessageResult>,
    delete_message_rx: tokio::sync::mpsc::UnboundedReceiver<DeleteMessageResult>,
    edit_message_rx: tokio::sync::mpsc::UnboundedReceiver<EditMessageResult>,
    reply_message_rx: tokio::sync::mpsc::UnboundedReceiver<ReplyMessageResult>,
    download_media_rx: tokio::sync::mpsc::UnboundedReceiver<DownloadMediaResult>,
    media_preview_rx: tokio::sync::mpsc::UnboundedReceiver<MediaPreviewResult>,
    reconciliation_rx: tokio::sync::mpsc::UnboundedReceiver<ReconciliationResult>,
    update_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Update>>,
    initial_state_pending: bool,
    reconciliation_pending: bool,
    subscription_pending: bool,
    next_reconciliation_at: Option<TokioInstant>,
    last_reconciliation_success_at: Option<TokioInstant>,
    next_subscription_at: Option<TokioInstant>,
    announce_reconciliation_success: bool,
    reconciliation_requested_while_pending: bool,
    reconciliation_high_water_ids: HashMap<i64, i32>,
    deferred_updates: Vec<Update>,
    staged_update: Option<Update>,
    staged_terminal_event: Option<Event>,
    #[cfg(test)]
    drain_trace: Vec<String>,
}

impl EventLoopState {
    fn new() -> (Self, EventLoopSenders) {
        let wake = Arc::new(tokio::sync::Notify::new());
        let (subscribe_updates, subscribe_updates_rx) = ui_channel(&wake);
        let (initial_state, initial_state_rx) = ui_channel(&wake);
        let (chat_message, chat_message_rx) = ui_channel(&wake);
        let (older_message, older_message_rx) = ui_channel(&wake);
        let (folder_chat, folder_chat_rx) = ui_channel(&wake);
        let (send_message, send_message_rx) = ui_channel(&wake);
        let (delete_message, delete_message_rx) = ui_channel(&wake);
        let (edit_message, edit_message_rx) = ui_channel(&wake);
        let (reply_message, reply_message_rx) = ui_channel(&wake);
        let (download_media, download_media_rx) = ui_channel(&wake);
        let (media_preview, media_preview_rx) = ui_channel(&wake);
        let (reconciliation, reconciliation_rx) = ui_channel(&wake);

        (
            Self {
                wake,
                subscribe_updates_rx,
                initial_state_rx,
                chat_message_rx,
                older_message_rx,
                folder_chat_rx,
                send_message_rx,
                delete_message_rx,
                edit_message_rx,
                reply_message_rx,
                download_media_rx,
                media_preview_rx,
                reconciliation_rx,
                update_rx: None,
                initial_state_pending: true,
                reconciliation_pending: false,
                subscription_pending: true,
                next_reconciliation_at: None,
                last_reconciliation_success_at: None,
                next_subscription_at: None,
                announce_reconciliation_success: false,
                reconciliation_requested_while_pending: false,
                reconciliation_high_water_ids: HashMap::new(),
                deferred_updates: Vec::new(),
                staged_update: None,
                staged_terminal_event: None,
                #[cfg(test)]
                drain_trace: Vec::new(),
            },
            EventLoopSenders {
                subscribe_updates,
                initial_state,
                chat_message,
                older_message,
                folder_chat,
                send_message,
                delete_message,
                edit_message,
                reply_message,
                download_media,
                media_preview,
                reconciliation,
            },
        )
    }

    fn schedule_reconciliation_at(&mut self, deadline: TokioInstant) {
        if self.reconciliation_pending || self.initial_state_pending {
            self.reconciliation_requested_while_pending = true;
            return;
        }
        self.next_reconciliation_at = Some(
            self.next_reconciliation_at
                .map_or(deadline, |current| current.min(deadline)),
        );
    }

    fn finish_reconciliation_gate(&mut self, default_deadline: TokioInstant) -> bool {
        let follow_up_requested = self.reconciliation_requested_while_pending;
        self.reconciliation_requested_while_pending = false;
        self.next_reconciliation_at = Some(if follow_up_requested {
            TokioInstant::now()
        } else {
            default_deadline
        });
        follow_up_requested
    }

    fn schedule_reconciliation_now(&mut self) {
        self.schedule_reconciliation_at(TokioInstant::now());
    }

    fn schedule_subscription_retry(&mut self) {
        if self.subscription_pending {
            return;
        }
        let deadline = TokioInstant::now() + UPDATE_SUBSCRIPTION_RETRY_DELAY;
        self.next_subscription_at = Some(
            self.next_subscription_at
                .map_or(deadline, |current| current.min(deadline)),
        );
    }

    fn schedule_focus_reconciliation(&mut self) {
        let now = TokioInstant::now();
        if self.last_reconciliation_success_at.is_none_or(|last| {
            now.saturating_duration_since(last) >= RECONCILIATION_FOCUS_STALE_AFTER
        }) {
            self.announce_reconciliation_success = true;
            self.schedule_reconciliation_at(now);
        }
    }

    fn service_deadline(&self) -> Option<TokioInstant> {
        [self.next_reconciliation_at, self.next_subscription_at]
            .into_iter()
            .flatten()
            .min()
    }
}

#[allow(clippy::too_many_arguments)]
fn drain_ready_results<C: TelegramClient + Clone + Send + Sync + 'static>(
    loop_state: &mut EventLoopState,
    app: &mut App,
    subscribe_updates_loader: &SubscribeUpdatesLoader<C>,
    reconciliation_loader: &ReconciliationLoader<C>,
    chat_message_loader: &ChatMessageLoader<C>,
    older_message_loader: &OlderMessageLoader<C>,
    folder_chat_loader: &FolderChatLoader<C>,
    mark_read_loader: &MarkChatReadLoader<C>,
    media_preview_loader: &MediaPreviewLoader<C>,
) -> bool {
    let mut dirty = false;
    while let Ok(result) = loop_state.subscribe_updates_rx.try_recv() {
        #[cfg(test)]
        loop_state.drain_trace.push("subscription".to_string());
        dirty |= apply_subscribe_updates_result(
            app,
            result,
            subscribe_updates_loader.latest_request_id(),
            loop_state,
        );
    }
    while let Ok(result) = loop_state.initial_state_rx.try_recv() {
        #[cfg(test)]
        loop_state.drain_trace.push("initial".to_string());
        let succeeded = result.result.is_ok();
        apply_initial_state_load_result(app, result, mark_read_loader);
        loop_state.initial_state_pending = false;
        let now = TokioInstant::now();
        if succeeded {
            loop_state.last_reconciliation_success_at = Some(now);
            loop_state.finish_reconciliation_gate(now + RECONCILIATION_INTERVAL);
        } else {
            loop_state.announce_reconciliation_success = true;
            loop_state.finish_reconciliation_gate(now + RECONCILIATION_RETRY_DELAY);
        }
        dirty = true;
        replay_deferred_updates(loop_state, app, mark_read_loader);
    }
    while let Ok(result) = loop_state.reconciliation_rx.try_recv() {
        dirty |= apply_reconciliation_result(
            app,
            result,
            reconciliation_loader.latest_request_id(),
            loop_state,
            mark_read_loader,
        );
        if !loop_state.initial_state_pending && !loop_state.reconciliation_pending {
            replay_deferred_updates(loop_state, app, mark_read_loader);
        }
    }
    if let Some(update) = loop_state.staged_update.take() {
        dirty |= handle_received_update(loop_state, app, update, mark_read_loader);
    }
    let queued_updates = loop_state
        .update_rx
        .as_mut()
        .map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>())
        .unwrap_or_default();
    for update in queued_updates {
        dirty |= handle_received_update(loop_state, app, update, mark_read_loader);
    }
    while let Ok(result) = loop_state.chat_message_rx.try_recv() {
        dirty |= apply_chat_message_load_result(
            app,
            chat_message_loader.latest_request_id(),
            result,
            mark_read_loader,
        );
    }
    while let Ok(result) = loop_state.older_message_rx.try_recv() {
        dirty |=
            apply_older_message_load_result(app, older_message_loader.latest_request_id(), result);
    }
    while let Ok(result) = loop_state.folder_chat_rx.try_recv() {
        dirty |= apply_folder_chat_load_result(
            app,
            folder_chat_loader.latest_request_id(),
            result,
            mark_read_loader,
        );
    }
    while let Ok(result) = loop_state.send_message_rx.try_recv() {
        apply_send_message_result(app, result);
        dirty = true;
    }
    while let Ok(result) = loop_state.delete_message_rx.try_recv() {
        apply_delete_message_result(app, result);
        dirty = true;
    }
    while let Ok(result) = loop_state.edit_message_rx.try_recv() {
        apply_edit_message_result(app, result);
        dirty = true;
    }
    while let Ok(result) = loop_state.reply_message_rx.try_recv() {
        apply_reply_message_result(app, result);
        dirty = true;
    }
    while let Ok(result) = loop_state.download_media_rx.try_recv() {
        apply_download_media_result(app, result);
        dirty = true;
    }
    while let Ok(result) = loop_state.media_preview_rx.try_recv() {
        dirty |= apply_media_preview_result(app, media_preview_loader.latest_request_id(), result);
    }
    dirty
}

fn update_represented_by_reconciliation(
    loop_state: &EventLoopState,
    app: &App,
    update: &Update,
) -> bool {
    let Update::NewMessage(message) = update else {
        return false;
    };
    let belongs_to_selected_conversation = app.state.selected_chat_id() == Some(message.chat_id)
        && app
            .state
            .selected_thread_topic()
            .map(|topic| topic.id)
            .is_none_or(|topic_id| message.thread_topic_id == Some(topic_id));
    !belongs_to_selected_conversation
        && loop_state
            .reconciliation_high_water_ids
            .get(&message.chat_id)
            .is_some_and(|last_id| message.id <= *last_id)
}

fn handle_received_update<C>(
    loop_state: &mut EventLoopState,
    app: &mut App,
    update: Update,
    mark_read_loader: &MarkChatReadLoader<C>,
) -> bool
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    if matches!(update, Update::Error(_)) {
        loop_state.announce_reconciliation_success = true;
        loop_state.schedule_reconciliation_now();
        #[cfg(test)]
        loop_state.drain_trace.push(update_trace_label(&update));
        apply_update_with_read_ack(app, update, mark_read_loader);
        return true;
    }
    if loop_state.initial_state_pending || loop_state.reconciliation_pending {
        loop_state.deferred_updates.push(update);
        return false;
    }
    if update_represented_by_reconciliation(loop_state, app, &update) {
        diagnostics::event("reconciliation_update_deduplicated", "count=1");
        return false;
    }
    #[cfg(test)]
    loop_state.drain_trace.push(update_trace_label(&update));
    apply_update_with_read_ack(app, update, mark_read_loader);
    true
}

fn replay_deferred_updates<C>(
    loop_state: &mut EventLoopState,
    app: &mut App,
    mark_read_loader: &MarkChatReadLoader<C>,
) where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    let deferred_updates = std::mem::take(&mut loop_state.deferred_updates);
    for update in deferred_updates {
        handle_received_update(loop_state, app, update, mark_read_loader);
    }
}

fn apply_reconciliation_result<C>(
    app: &mut App,
    result: ReconciliationResult,
    latest_request_id: u64,
    loop_state: &mut EventLoopState,
    mark_read_loader: &MarkChatReadLoader<C>,
) -> bool
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    if result.request_id != latest_request_id {
        diagnostics::event(
            "reconciliation_result_ignored",
            format!(
                "request_id={} latest_request_id={latest_request_id}",
                result.request_id
            ),
        );
        return false;
    }

    loop_state.reconciliation_pending = false;
    let now = TokioInstant::now();
    match result.result {
        Ok(snapshot) => {
            let high_water_ids = snapshot.chat_last_message_ids.clone();
            let selected_read_ack = snapshot.selected_chat_id.and_then(|chat_id| {
                if let Some(topic_id) = snapshot.selected_topic_id {
                    let has_unread = snapshot
                        .thread_topics
                        .iter()
                        .find(|topic| topic.id == topic_id)
                        .is_some_and(|topic| topic.unread_count > 0);
                    let max_message_id = snapshot.messages.iter().map(|message| message.id).max();
                    (has_unread)
                        .then_some(max_message_id)
                        .flatten()
                        .map(|max_message_id| ReconciliationReadAck::Thread {
                            chat_id,
                            topic_id,
                            max_message_id,
                        })
                } else {
                    snapshot
                        .chats
                        .iter()
                        .find(|chat| chat.id == chat_id)
                        .is_some_and(|chat| chat.unread_count > 0 && !snapshot.messages.is_empty())
                        .then_some(ReconciliationReadAck::Chat { chat_id })
                }
            });
            match app
                .state
                .apply_reconciliation_snapshot(result.context, snapshot)
            {
                state::ReconciliationApply::Applied { .. } => {
                    loop_state.reconciliation_high_water_ids = high_water_ids;
                    loop_state.last_reconciliation_success_at = Some(now);
                    let follow_up_requested =
                        loop_state.finish_reconciliation_gate(now + RECONCILIATION_INTERVAL);
                    if loop_state.announce_reconciliation_success && !follow_up_requested {
                        app.state.set_status(TELEGRAM_STATE_REFRESHED_STATUS);
                        loop_state.announce_reconciliation_success = false;
                    }
                    match selected_read_ack {
                        Some(ReconciliationReadAck::Chat { chat_id })
                            if app.state.selected_chat_id() == Some(chat_id) =>
                        {
                            mark_read_loader.spawn_mark_chat_read(chat_id);
                        }
                        Some(ReconciliationReadAck::Thread {
                            chat_id,
                            topic_id,
                            max_message_id,
                        }) if app.state.selected_chat_id() == Some(chat_id)
                            && app.state.selected_thread_topic().map(|topic| topic.id)
                                == Some(topic_id) =>
                        {
                            mark_read_loader.spawn_mark_thread_read(
                                chat_id,
                                topic_id,
                                max_message_id,
                            );
                        }
                        _ => {}
                    }
                }
                state::ReconciliationApply::Stale => {
                    diagnostics::event("reconciliation_result_ignored", "reason=stale_context");
                    loop_state.finish_reconciliation_gate(now);
                }
            }
        }
        Err(error) => {
            diagnostics::event("reconciliation_result_error", format!("error={error}"));
            app.state.set_error(error);
            loop_state.announce_reconciliation_success = true;
            loop_state.finish_reconciliation_gate(now + RECONCILIATION_RETRY_DELAY);
        }
    }
    true
}

#[cfg(test)]
fn update_trace_label(update: &Update) -> String {
    match update {
        Update::Error(error) => format!("update:{error}"),
        _ => "update".to_string(),
    }
}

enum ReconciliationReadAck {
    Chat {
        chat_id: i64,
    },
    Thread {
        chat_id: i64,
        topic_id: i32,
        max_message_id: i32,
    },
}

struct SubscribeUpdatesResult {
    request_id: u64,
    result: std::result::Result<tokio::sync::mpsc::UnboundedReceiver<Update>, String>,
}

struct InitialStateLoadResult {
    result: std::result::Result<actions::InitialStateLoad, String>,
}

struct ReconciliationResult {
    request_id: u64,
    context: state::ReconciliationContext,
    result: std::result::Result<state::ReconciliationSnapshot, String>,
}

struct ChatMessageLoad {
    messages: Vec<Message>,
    thread_topics: Option<Vec<ThreadTopic>>,
}

struct ChatMessageLoadResult {
    request_id: u64,
    chat_id: i64,
    topic_id: Option<i32>,
    result: std::result::Result<ChatMessageLoad, String>,
}

struct OlderMessageLoadResult {
    request_id: u64,
    chat_id: i64,
    topic_id: Option<i32>,
    before_message_id: i32,
    navigation: OlderMessageNavigation,
    result: std::result::Result<Vec<Message>, String>,
}

struct FolderChatLoadResult {
    request_id: u64,
    folder_index: usize,
    folder_id: Option<i32>,
    result: std::result::Result<actions::FolderChatLoad, String>,
}

struct SendMessageResult {
    temp_id: i32,
    chat_id: i64,
    result: std::result::Result<Message, String>,
}

struct DeleteMessageResult {
    confirmation: state::DeleteConfirmation,
    result: std::result::Result<(), String>,
}

struct EditMessageResult {
    chat_id: i64,
    message_id: i32,
    content: String,
    result: std::result::Result<(), String>,
}

struct ReplyMessageResult {
    chat_id: i64,
    message_id: i32,
    result: std::result::Result<Message, String>,
}

struct DownloadMediaResult {
    chat_id: i64,
    message_id: i32,
    result: std::result::Result<telegram::DownloadedMedia, String>,
}

struct MediaPreviewResult {
    request_id: u64,
    chat_id: i64,
    message_id: i32,
    result: std::result::Result<Option<PathBuf>, String>,
}

struct HandlerLoaders<'a, C> {
    chat_message: Option<&'a mut ChatMessageLoader<C>>,
    older_message: Option<&'a mut OlderMessageLoader<C>>,
    folder_chat: Option<&'a mut FolderChatLoader<C>>,
    mark_read: Option<&'a MarkChatReadLoader<C>>,
    send_message: Option<&'a SendMessageLoader<C>>,
    delete_message: Option<&'a DeleteMessageLoader<C>>,
    edit_message: Option<&'a EditMessageLoader<C>>,
    reply_message: Option<&'a ReplyMessageLoader<C>>,
    download_media: Option<&'a DownloadMediaLoader<C>>,
}

impl<C> HandlerLoaders<'_, C> {
    fn none() -> Self {
        Self {
            chat_message: None,
            older_message: None,
            folder_chat: None,
            mark_read: None,
            send_message: None,
            delete_message: None,
            edit_message: None,
            reply_message: None,
            download_media: None,
        }
    }
}

struct SubscribeUpdatesLoader<C> {
    client: C,
    tx: UiSender<SubscribeUpdatesResult>,
    latest_request_id: u64,
}

impl<C> SubscribeUpdatesLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<SubscribeUpdatesResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
            latest_request_id: 0,
        }
    }

    fn latest_request_id(&self) -> u64 {
        self.latest_request_id
    }

    fn spawn_subscribe_updates(&mut self) {
        self.latest_request_id = self.latest_request_id.saturating_add(1);
        let request_id = self.latest_request_id;
        let mut client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event("subscribe_updates_spawn", "updates=true");
        tokio::spawn(async move {
            let result = client
                .subscribe_updates()
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(SubscribeUpdatesResult { request_id, result });
        });
    }
}

struct InitialStateLoader<C> {
    client: C,
    tx: UiSender<InitialStateLoadResult>,
}

impl<C> InitialStateLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<InitialStateLoadResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
        }
    }

    fn spawn_initial_state(&self) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "initial_load_spawn",
            "folders=true chats=true messages=true",
        );
        tokio::spawn(async move {
            let result = actions::fetch_initial_state(&client).await;
            let _ = tx.send(InitialStateLoadResult { result });
        });
    }
}

struct ReconciliationLoader<C> {
    client: C,
    tx: UiSender<ReconciliationResult>,
    latest_request_id: u64,
    current_handle: Option<tokio::task::JoinHandle<()>>,
}

impl<C> ReconciliationLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: UiSender<ReconciliationResult>) -> Self {
        Self {
            client,
            tx,
            latest_request_id: 0,
            current_handle: None,
        }
    }

    fn latest_request_id(&self) -> u64 {
        self.latest_request_id
    }

    fn spawn_reconciliation(&mut self, context: state::ReconciliationContext) {
        abort_running_task(
            &mut self.current_handle,
            "reconciliation_abort",
            self.latest_request_id,
        );
        self.latest_request_id = self.latest_request_id.saturating_add(1);
        let request_id = self.latest_request_id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "reconciliation_spawn",
            format!(
                "request_id={request_id} folder_id={:?} chat_id={:?} topic_id={:?}",
                context.folder_id, context.chat_id, context.topic_id
            ),
        );
        self.current_handle = Some(tokio::spawn(async move {
            let result = actions::fetch_reconciliation_snapshot(&client, context).await;
            let _ = tx.send(ReconciliationResult {
                request_id,
                context,
                result,
            });
        }));
    }
}

struct ChatMessageLoader<C> {
    client: C,
    tx: UiSender<ChatMessageLoadResult>,
    latest_request_id: u64,
    current_handle: Option<tokio::task::JoinHandle<()>>,
}

fn abort_running_task(
    handle: &mut Option<tokio::task::JoinHandle<()>>,
    event_name: &'static str,
    latest_request_id: u64,
) -> bool {
    let Some(handle) = handle.take() else {
        return false;
    };
    if handle.is_finished() {
        return false;
    }

    diagnostics::event(event_name, format!("latest_request_id={latest_request_id}"));
    handle.abort();
    true
}

struct MarkChatReadLoader<C> {
    client: C,
}

impl<C> MarkChatReadLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C) -> Self {
        Self { client }
    }

    fn spawn_mark_chat_read(&self, chat_id: i64) {
        let client = self.client.clone();
        diagnostics::event("mark_chat_read_spawn", format!("chat_id={chat_id}"));
        tokio::spawn(async move {
            actions::mark_chat_read_best_effort(&client, chat_id).await;
        });
    }

    fn spawn_mark_thread_read(&self, chat_id: i64, topic_id: i32, max_message_id: i32) {
        let client = self.client.clone();
        diagnostics::event(
            "mark_thread_read_spawn",
            format!("chat_id={chat_id} topic_id={topic_id} max_message_id={max_message_id}"),
        );
        tokio::spawn(async move {
            actions::mark_thread_read_best_effort(&client, chat_id, topic_id, max_message_id).await;
        });
    }
}

impl<C> ChatMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<ChatMessageLoadResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
            latest_request_id: 0,
            current_handle: None,
        }
    }

    fn latest_request_id(&self) -> u64 {
        self.latest_request_id
    }

    fn spawn_latest_chat_messages(&mut self, chat_id: i64) {
        abort_running_task(
            &mut self.current_handle,
            "messages_load_abort",
            self.latest_request_id,
        );

        self.latest_request_id = self.latest_request_id.saturating_add(1);
        let request_id = self.latest_request_id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "messages_load_spawn",
            format!("request_id={request_id} chat_id={chat_id}"),
        );
        self.current_handle = Some(tokio::spawn(async move {
            let result = match actions::fetch_latest_chat_messages(&client, chat_id).await {
                Ok(messages) => {
                    let thread_topics = actions::fetch_chat_thread_topics(&client, chat_id)
                        .await
                        .unwrap_or_default();
                    Ok(ChatMessageLoad {
                        messages,
                        thread_topics: Some(thread_topics),
                    })
                }
                Err(error) => Err(error),
            };
            let _ = tx.send(ChatMessageLoadResult {
                request_id,
                chat_id,
                topic_id: None,
                result,
            });
        }));
    }

    fn spawn_thread_topic_messages(&mut self, chat_id: i64, topic_id: i32) {
        abort_running_task(
            &mut self.current_handle,
            "messages_load_abort",
            self.latest_request_id,
        );

        self.latest_request_id = self.latest_request_id.saturating_add(1);
        let request_id = self.latest_request_id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "thread_messages_load_spawn",
            format!("request_id={request_id} chat_id={chat_id} topic_id={topic_id}"),
        );
        self.current_handle = Some(tokio::spawn(async move {
            let result = actions::fetch_thread_topic_messages(&client, chat_id, topic_id)
                .await
                .map(|messages| ChatMessageLoad {
                    messages,
                    thread_topics: None,
                });
            let _ = tx.send(ChatMessageLoadResult {
                request_id,
                chat_id,
                topic_id: Some(topic_id),
                result,
            });
        }));
    }
}

struct OlderMessageLoader<C> {
    client: C,
    tx: UiSender<OlderMessageLoadResult>,
    latest_request_id: u64,
    current_handle: Option<tokio::task::JoinHandle<()>>,
}

impl<C> OlderMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<OlderMessageLoadResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
            latest_request_id: 0,
            current_handle: None,
        }
    }

    fn latest_request_id(&self) -> u64 {
        self.latest_request_id
    }

    fn spawn_older_messages(
        &mut self,
        chat_id: i64,
        topic_id: Option<i32>,
        before_message_id: i32,
        navigation: OlderMessageNavigation,
    ) {
        abort_running_task(
            &mut self.current_handle,
            "older_messages_load_abort",
            self.latest_request_id,
        );

        self.latest_request_id = self.latest_request_id.saturating_add(1);
        let request_id = self.latest_request_id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "older_messages_load_spawn",
            format!(
                "request_id={request_id} chat_id={chat_id} topic_id={topic_id:?} before_message_id={before_message_id}"
            ),
        );
        self.current_handle = Some(tokio::spawn(async move {
            let result = if let Some(topic_id) = topic_id {
                actions::fetch_older_thread_topic_messages(
                    &client,
                    chat_id,
                    topic_id,
                    before_message_id,
                )
                .await
            } else {
                actions::fetch_older_chat_messages(&client, chat_id, before_message_id).await
            };
            let _ = tx.send(OlderMessageLoadResult {
                request_id,
                chat_id,
                topic_id,
                before_message_id,
                navigation,
                result,
            });
        }));
    }
}

struct FolderChatLoader<C> {
    client: C,
    tx: UiSender<FolderChatLoadResult>,
    latest_request_id: u64,
    current_handle: Option<tokio::task::JoinHandle<()>>,
}

struct SendMessageLoader<C> {
    client: C,
    tx: UiSender<SendMessageResult>,
}

struct DeleteMessageLoader<C> {
    client: C,
    tx: UiSender<DeleteMessageResult>,
}

struct EditMessageLoader<C> {
    client: C,
    tx: UiSender<EditMessageResult>,
}

struct ReplyMessageLoader<C> {
    client: C,
    tx: UiSender<ReplyMessageResult>,
}

struct DownloadMediaLoader<C> {
    client: C,
    tx: UiSender<DownloadMediaResult>,
}

struct MediaPreviewLoader<C> {
    client: C,
    tx: UiSender<MediaPreviewResult>,
    latest_request_id: u64,
    // ponytail: failed/empty previews retry only after selection changes; add timed retry if transient failures matter.
    last_requested: Option<(i64, i32)>,
    current_handle: Option<tokio::task::JoinHandle<()>>,
}

impl<C> SendMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<SendMessageResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
        }
    }

    fn spawn_send_message(&self, pending: actions::PendingSend) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "send_message_spawn",
            format!("temp_id={} chat_id={}", pending.temp_id, pending.chat_id),
        );
        tokio::spawn(async move {
            let result = actions::send_message_result(
                &client,
                pending.chat_id,
                pending.thread_top_message_id,
                pending.content,
            )
            .await;
            let _ = tx.send(SendMessageResult {
                temp_id: pending.temp_id,
                chat_id: pending.chat_id,
                result,
            });
        });
    }
}

impl<C> DeleteMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<DeleteMessageResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
        }
    }

    fn spawn_delete_message(&self, confirmation: state::DeleteConfirmation) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "delete_message_spawn",
            format!(
                "chat_id={} message_id={}",
                confirmation.chat_id, confirmation.message_id
            ),
        );
        tokio::spawn(async move {
            let result = actions::delete_message_result(&client, confirmation).await;
            let _ = tx.send(DeleteMessageResult {
                confirmation,
                result,
            });
        });
    }
}

impl<C> EditMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<EditMessageResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
        }
    }

    fn spawn_edit_message(&self, chat_id: i64, message_id: i32, content: String) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "edit_message_spawn",
            format!("chat_id={chat_id} message_id={message_id}"),
        );
        tokio::spawn(async move {
            let result =
                actions::edit_message_result(&client, chat_id, message_id, content.clone()).await;
            let _ = tx.send(EditMessageResult {
                chat_id,
                message_id,
                content,
                result,
            });
        });
    }
}

impl<C> ReplyMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<ReplyMessageResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
        }
    }

    fn spawn_reply_message(
        &self,
        chat_id: i64,
        thread_top_message_id: Option<i32>,
        message_id: i32,
        content: String,
    ) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "reply_message_spawn",
            format!("chat_id={chat_id} topic_id={thread_top_message_id:?} message_id={message_id}"),
        );
        tokio::spawn(async move {
            let result = actions::reply_message_result(
                &client,
                chat_id,
                thread_top_message_id,
                message_id,
                content,
            )
            .await;
            let _ = tx.send(ReplyMessageResult {
                chat_id,
                message_id,
                result,
            });
        });
    }
}

impl<C> MediaPreviewLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<MediaPreviewResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
            latest_request_id: 0,
            last_requested: None,
            current_handle: None,
        }
    }

    fn latest_request_id(&self) -> u64 {
        self.latest_request_id
    }

    fn request(&mut self, request: Option<(i64, i32)>) {
        if self.last_requested == request {
            return;
        }
        abort_running_task(
            &mut self.current_handle,
            "media_preview_abort",
            self.latest_request_id,
        );
        self.last_requested = request;
        let Some((chat_id, message_id)) = request else {
            return;
        };

        self.latest_request_id = self.latest_request_id.saturating_add(1);
        let request_id = self.latest_request_id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "media_preview_spawn",
            format!("request_id={request_id} chat_id={chat_id} message_id={message_id}"),
        );
        self.current_handle = Some(tokio::spawn(async move {
            let result = client
                .load_message_media_preview(chat_id, message_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(MediaPreviewResult {
                request_id,
                chat_id,
                message_id,
                result,
            });
        }));
    }
}

impl<C> DownloadMediaLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<DownloadMediaResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
        }
    }

    fn spawn_download_media(
        &self,
        chat_id: i64,
        message_id: i32,
        media_kind: telegram::types::MessageMediaKind,
    ) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "media_download_spawn",
            format!(
                "chat_id={chat_id} message_id={message_id} kind={} destination=downloads",
                media_kind.diagnostic_label()
            ),
        );
        tokio::spawn(async move {
            let result = actions::download_message_media_result(
                &client,
                chat_id,
                message_id,
                media_kind,
                actions::default_download_dir(),
            )
            .await;
            let _ = tx.send(DownloadMediaResult {
                chat_id,
                message_id,
                result,
            });
        });
    }
}

impl<C> FolderChatLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: impl Into<UiSender<FolderChatLoadResult>>) -> Self {
        Self {
            client,
            tx: tx.into(),
            latest_request_id: 0,
            current_handle: None,
        }
    }

    fn latest_request_id(&self) -> u64 {
        self.latest_request_id
    }

    fn spawn_folder_chats(&mut self, folder_index: usize, folder_id: Option<i32>) {
        abort_running_task(
            &mut self.current_handle,
            "folder_chats_load_abort",
            self.latest_request_id,
        );

        self.latest_request_id = self.latest_request_id.saturating_add(1);
        let request_id = self.latest_request_id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "folder_chats_load_spawn",
            format!("request_id={request_id} folder_index={folder_index} folder_id={folder_id:?}"),
        );
        self.current_handle = Some(tokio::spawn(async move {
            let result =
                actions::fetch_folder_chats_and_selected_messages(&client, folder_id).await;
            let _ = tx.send(FolderChatLoadResult {
                request_id,
                folder_index,
                folder_id,
                result,
            });
        }));
    }
}

fn apply_subscribe_updates_result(
    app: &mut App,
    load: SubscribeUpdatesResult,
    latest_request_id: u64,
    loop_state: &mut EventLoopState,
) -> bool {
    if load.request_id != latest_request_id {
        diagnostics::event(
            "subscribe_updates_result_ignored",
            format!(
                "request_id={} latest_request_id={latest_request_id}",
                load.request_id
            ),
        );
        return false;
    }

    loop_state.subscription_pending = false;
    match load.result {
        Ok(rx) => {
            diagnostics::event("subscribe_updates_result", "updates=true");
            loop_state.update_rx = Some(rx);
            loop_state.next_subscription_at = None;
            false
        }
        Err(error) => {
            diagnostics::event("subscribe_updates_result", "updates=false");
            app.state.set_error(error);
            loop_state.announce_reconciliation_success = true;
            loop_state.schedule_reconciliation_now();
            loop_state.schedule_subscription_retry();
            true
        }
    }
}

fn initial_load_read_ack_chat_id(load: &InitialStateLoadResult) -> Option<i64> {
    let load = load.result.as_ref().ok()?;
    if load.messages.as_ref().ok()?.is_empty() {
        return None;
    }
    let chat = load.chats.first()?;
    (chat.unread_count > 0).then_some(chat.id)
}

fn folder_chat_load_read_ack_chat_id(load: &FolderChatLoadResult) -> Option<i64> {
    let load = load.result.as_ref().ok()?;
    if load.messages.as_ref().ok()?.is_empty() {
        return None;
    }
    let chat = load.chats.first()?;
    (chat.unread_count > 0).then_some(chat.id)
}

fn selected_chat_needs_read_ack(app: &App, chat_id: i64, messages: &[Message]) -> bool {
    !messages.is_empty()
        && app.state.selected_chat_id() == Some(chat_id)
        && app
            .state
            .chats
            .get(app.state.selected_chat_index)
            .is_some_and(|chat| chat.unread_count > 0)
}

fn apply_initial_state_load_result<C>(
    app: &mut App,
    load: InitialStateLoadResult,
    mark_read_loader: &MarkChatReadLoader<C>,
) where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    diagnostics::event("initial_load_result", "received=true");
    let read_ack_chat_id = initial_load_read_ack_chat_id(&load);
    actions::apply_initial_state_load_result(&mut app.state, load.result);
    if let Some(chat_id) =
        read_ack_chat_id.filter(|chat_id| app.state.selected_chat_id() == Some(*chat_id))
    {
        mark_read_loader.spawn_mark_chat_read(chat_id);
    }
    diagnostics::event(
        "state_after_initial_load",
        format!(
            "folders={} chats={} messages={} selected_chat_id={:?}",
            app.state.folders.len(),
            app.state.chats.len(),
            app.state.messages.len(),
            app.state.selected_chat_id()
        ),
    );
    app.state.clear_status();
}

fn apply_send_message_result(app: &mut App, load: SendMessageResult) {
    diagnostics::event(
        "send_message_result",
        format!("temp_id={} chat_id={}", load.temp_id, load.chat_id),
    );
    actions::apply_send_message_result(&mut app.state, load.temp_id, load.result);
}

fn apply_delete_message_result(app: &mut App, load: DeleteMessageResult) {
    diagnostics::event(
        "delete_message_result",
        format!(
            "chat_id={} message_id={}",
            load.confirmation.chat_id, load.confirmation.message_id
        ),
    );
    actions::apply_delete_message_result(&mut app.state, load.confirmation, load.result);
}

fn apply_edit_message_result(app: &mut App, load: EditMessageResult) {
    diagnostics::event(
        "edit_message_result",
        format!("chat_id={} message_id={}", load.chat_id, load.message_id),
    );
    actions::apply_edit_message_result(&mut app.state, load.message_id, load.content, load.result);
}

fn apply_reply_message_result(app: &mut App, load: ReplyMessageResult) {
    diagnostics::event(
        "reply_message_result",
        format!("chat_id={} message_id={}", load.chat_id, load.message_id),
    );
    actions::apply_reply_message_result(&mut app.state, load.result);
}

fn incoming_update_thread_read_ack(app: &App, update: &Update) -> Option<(i64, i32, i32)> {
    let Update::NewMessage(message) = update else {
        return None;
    };
    if message.is_own {
        return None;
    }

    let chat_id = app.state.selected_chat_id()?;
    let topic_id = app.state.selected_thread_topic()?.id;
    (message.chat_id == chat_id && message.thread_topic_id == Some(topic_id))
        .then_some((chat_id, topic_id, message.id))
}

fn apply_update_with_read_ack<C>(
    app: &mut App,
    update: Update,
    mark_read_loader: &MarkChatReadLoader<C>,
) where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    let thread_read_ack = incoming_update_thread_read_ack(app, &update);
    app.state.apply_update(update);
    if let Some((chat_id, topic_id, max_message_id)) = thread_read_ack {
        mark_read_loader.spawn_mark_thread_read(chat_id, topic_id, max_message_id);
    }
}

fn apply_chat_message_load_result<C>(
    app: &mut App,
    latest_request_id: u64,
    load: ChatMessageLoadResult,
    mark_read_loader: &MarkChatReadLoader<C>,
) -> bool
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    if load.request_id != latest_request_id {
        diagnostics::event(
            "messages_load_ignored",
            format!(
                "reason=stale_request request_id={} latest_request_id={} chat_id={}",
                load.request_id, latest_request_id, load.chat_id
            ),
        );
        return false;
    }

    if app.state.selected_chat_id() != Some(load.chat_id) {
        diagnostics::event(
            "messages_load_ignored",
            format!(
                "reason=stale_chat request_id={} chat_id={} selected_chat_id={:?}",
                load.request_id,
                load.chat_id,
                app.state.selected_chat_id()
            ),
        );
        return false;
    }

    let selected_topic_id = app.state.selected_thread_topic().map(|topic| topic.id);
    if selected_topic_id != load.topic_id {
        diagnostics::event(
            "messages_load_ignored",
            format!(
                "reason=stale_topic request_id={} chat_id={} topic_id={:?} selected_topic_id={:?}",
                load.request_id, load.chat_id, load.topic_id, selected_topic_id
            ),
        );
        return false;
    }

    let chat_id = load.chat_id;
    let topic_id = load.topic_id;
    match load.result {
        Ok(load) => {
            let thread_read_ack = topic_id.and_then(|topic_id| {
                load.messages
                    .iter()
                    .map(|message| message.id)
                    .max()
                    .map(|max_message_id| (topic_id, max_message_id))
            });
            let should_mark_chat_read =
                topic_id.is_none() && selected_chat_needs_read_ack(app, chat_id, &load.messages);
            app.state.apply_loaded_selected_chat_messages(load.messages);
            if let Some(thread_topics) = load.thread_topics {
                app.state
                    .apply_loaded_selected_chat_thread_topics(thread_topics);
            }
            if let Some((topic_id, max_message_id)) = thread_read_ack {
                mark_read_loader.spawn_mark_thread_read(chat_id, topic_id, max_message_id);
            } else if should_mark_chat_read {
                mark_read_loader.spawn_mark_chat_read(chat_id);
            }
            app.state.clear_status();
        }
        Err(error) => {
            app.state.mark_conversation_load_failed();
            app.state.set_error(error);
        }
    }
    true
}

fn apply_folder_chat_load_result<C>(
    app: &mut App,
    latest_request_id: u64,
    load: FolderChatLoadResult,
    mark_read_loader: &MarkChatReadLoader<C>,
) -> bool
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    if load.request_id != latest_request_id {
        diagnostics::event(
            "folder_chats_load_ignored",
            format!(
                "reason=stale_request request_id={} latest_request_id={} folder_index={} folder_id={:?}",
                load.request_id, latest_request_id, load.folder_index, load.folder_id
            ),
        );
        return false;
    }

    if app.state.selected_folder_index != load.folder_index
        || app.state.selected_folder_filter_id() != load.folder_id
    {
        diagnostics::event(
            "folder_chats_load_ignored",
            format!(
                "reason=stale_folder request_id={} folder_index={} selected_folder_index={} folder_id={:?} selected_folder_id={:?}",
                load.request_id,
                load.folder_index,
                app.state.selected_folder_index,
                load.folder_id,
                app.state.selected_folder_filter_id()
            ),
        );
        return false;
    }

    let read_ack_chat_id = folder_chat_load_read_ack_chat_id(&load);
    actions::apply_folder_chat_load_result(&mut app.state, load.result);
    if let Some(chat_id) =
        read_ack_chat_id.filter(|chat_id| app.state.selected_chat_id() == Some(*chat_id))
    {
        mark_read_loader.spawn_mark_chat_read(chat_id);
    }
    app.state.clear_status();
    true
}

fn apply_older_message_load_result(
    app: &mut App,
    latest_request_id: u64,
    load: OlderMessageLoadResult,
) -> bool {
    if load.request_id != latest_request_id {
        diagnostics::event(
            "older_messages_load_ignored",
            format!(
                "reason=stale_request request_id={} latest_request_id={} chat_id={} before_message_id={}",
                load.request_id, latest_request_id, load.chat_id, load.before_message_id
            ),
        );
        return false;
    }

    if app.state.selected_chat_id() != Some(load.chat_id) {
        diagnostics::event(
            "older_messages_load_ignored",
            format!(
                "reason=stale_chat request_id={} chat_id={} selected_chat_id={:?}",
                load.request_id,
                load.chat_id,
                app.state.selected_chat_id()
            ),
        );
        return false;
    }

    let selected_topic_id = app.state.selected_thread_topic().map(|topic| topic.id);
    if selected_topic_id != load.topic_id {
        diagnostics::event(
            "older_messages_load_ignored",
            format!(
                "reason=stale_topic request_id={} chat_id={} topic_id={:?} selected_topic_id={:?}",
                load.request_id, load.chat_id, load.topic_id, selected_topic_id
            ),
        );
        return false;
    }

    if app.state.messages.first().map(|message| message.id) != Some(load.before_message_id) {
        diagnostics::event(
            "older_messages_load_ignored",
            format!(
                "reason=stale_anchor request_id={} chat_id={} before_message_id={}",
                load.request_id, load.chat_id, load.before_message_id
            ),
        );
        return false;
    }

    let added = actions::apply_older_chat_messages_result(&mut app.state, load.result);
    if added > 0 {
        app.state.clear_status();
        apply_older_message_navigation(&mut app.state, load.navigation);
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OlderMessageNavigation {
    OneLine,
    Page,
}

enum UiProgress<'a> {
    Live {
        terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
    },
    Silent,
}

impl UiProgress<'_> {
    fn copy_text(&mut self, text: &str) -> Result<()> {
        match self {
            Self::Live { terminal, .. } => {
                terminal_clipboard::copy_text(terminal.backend_mut(), text)?;
            }
            Self::Silent => {}
        }
        Ok(())
    }

    fn show(&mut self, app: &mut App, status: impl Into<String>) -> Result<()> {
        let status = status.into();
        let show_status_banner = !is_loading_progress_status(&status);
        diagnostics::event(
            "progress_status",
            format!("status={status} banner={show_status_banner}"),
        );
        if show_status_banner {
            app.state.set_status(status);
        }
        Ok(())
    }
}

fn is_loading_progress_status(status: &str) -> bool {
    status.starts_with("Loading ")
}

fn log_draw_duration(label: &str, started: Instant) {
    let elapsed_ms = started.elapsed().as_millis();
    if elapsed_ms >= SLOW_RENDER_LOG_THRESHOLD_MS {
        diagnostics::event(
            "slow_terminal_draw",
            format!("label={label} elapsed_ms={elapsed_ms}"),
        );
    }
}

fn copy_selected_message_text(app: &mut App, progress: &mut UiProgress<'_>) -> Result<()> {
    let Some(message) = app.state.selected_message() else {
        app.state.set_status(NO_TEXT_IN_SELECTED_MESSAGE_STATUS);
        return Ok(());
    };
    if message.content.trim().is_empty() {
        diagnostics::event(
            "message_copy_skipped",
            format!(
                "reason=no_text chat_id={} message_id={}",
                message.chat_id, message.id
            ),
        );
        app.state.set_status(NO_TEXT_IN_SELECTED_MESSAGE_STATUS);
        return Ok(());
    }

    diagnostics::event(
        "message_copy",
        format!(
            "chat_id={} message_id={} chars={}",
            message.chat_id,
            message.id,
            message.content.chars().count()
        ),
    );
    let text = message.content.clone();
    progress.copy_text(&text)?;
    app.state.set_status(MESSAGE_TEXT_COPIED_STATUS);
    Ok(())
}

fn selected_media_download_request(
    app: &mut App,
) -> Option<(i64, i32, telegram::types::MessageMediaKind)> {
    let Some(message) = app.state.selected_message() else {
        app.state.set_status(NO_MEDIA_IN_SELECTED_MESSAGE_STATUS);
        return None;
    };
    let Some(media) = message
        .media
        .as_ref()
        .filter(|media| media.kind.is_downloadable())
    else {
        diagnostics::event(
            "media_download_skipped",
            format!(
                "reason=no_downloadable_media chat_id={} message_id={}",
                message.chat_id, message.id
            ),
        );
        app.state.set_status(NO_MEDIA_IN_SELECTED_MESSAGE_STATUS);
        return None;
    };
    Some((message.chat_id, message.id, media.kind.clone()))
}

async fn download_selected_media_with_optional_async_loader<
    C: TelegramClient + Clone + Send + Sync + 'static,
>(
    app: &mut App,
    client: &C,
    progress: &mut UiProgress<'_>,
    loader: Option<&DownloadMediaLoader<C>>,
) -> Result<()> {
    let Some((chat_id, message_id, media_kind)) = selected_media_download_request(app) else {
        return Ok(());
    };
    progress.show(app, DOWNLOADING_MEDIA_STATUS)?;
    if let Some(loader) = loader {
        loader.spawn_download_media(chat_id, message_id, media_kind);
    } else {
        let result = actions::download_message_media_result(
            client,
            chat_id,
            message_id,
            media_kind,
            actions::default_download_dir(),
        )
        .await;
        apply_download_media_result(
            app,
            DownloadMediaResult {
                chat_id,
                message_id,
                result,
            },
        );
    }
    Ok(())
}

fn apply_media_preview_result(
    app: &mut App,
    latest_request_id: u64,
    preview: MediaPreviewResult,
) -> bool {
    if preview.request_id != latest_request_id
        || app.state.selected_media_preview_request() != Some((preview.chat_id, preview.message_id))
    {
        diagnostics::event(
            "media_preview_stale",
            format!(
                "request_id={} latest_request_id={} chat_id={} message_id={}",
                preview.request_id, latest_request_id, preview.chat_id, preview.message_id
            ),
        );
        return false;
    }

    match preview.result {
        Ok(Some(path)) => {
            let applied =
                app.state
                    .apply_selected_media_preview(preview.chat_id, preview.message_id, path);
            if applied {
                diagnostics::event(
                    "media_preview_apply",
                    format!(
                        "request_id={} chat_id={} message_id={}",
                        preview.request_id, preview.chat_id, preview.message_id
                    ),
                );
            }
            applied
        }
        Ok(None) => {
            diagnostics::event(
                "media_preview_unavailable",
                format!(
                    "request_id={} chat_id={} message_id={}",
                    preview.request_id, preview.chat_id, preview.message_id
                ),
            );
            false
        }
        Err(_) => {
            diagnostics::event(
                "media_preview_download_error",
                format!(
                    "request_id={} chat_id={} message_id={} error=true",
                    preview.request_id, preview.chat_id, preview.message_id
                ),
            );
            false
        }
    }
}

fn apply_download_media_result(app: &mut App, download: DownloadMediaResult) {
    match download.result {
        Ok(downloaded) => {
            diagnostics::event(
                "media_download_apply",
                format!(
                    "chat_id={} message_id={} bytes={} destination=downloads",
                    download.chat_id, download.message_id, downloaded.bytes
                ),
            );
            app.state.record_downloaded_media(
                download.chat_id,
                download.message_id,
                downloaded.path,
            );
            app.state.set_status(MEDIA_DOWNLOADED_STATUS);
        }
        Err(error) => app.state.set_error(error),
    }
}

fn open_selected_downloaded_media(app: &mut App) {
    let Some(path) = app.state.selected_message_download_path() else {
        app.state.set_status(NO_DOWNLOADED_MEDIA_STATUS);
        return;
    };

    diagnostics::event("downloaded_media_open", "target=file_opener");
    match file_opener::open_path(path) {
        Ok(()) => app.state.set_status(DOWNLOADED_MEDIA_OPENED_STATUS),
        Err(error) => {
            diagnostics::event("downloaded_media_open_error", "error=true");
            app.state
                .set_error(format!("{OPEN_DOWNLOADED_MEDIA_FAILED_PREFIX}: {error}"));
        }
    }
}

fn open_selected_message_link(app: &mut App) {
    if let Some(url) = app
        .state
        .selected_message()
        .and_then(|message| links::first_url(&message.content))
    {
        open_message_link(app, &url);
    } else {
        app.state.set_status(NO_LINK_IN_SELECTED_MESSAGE_STATUS);
    }
}

fn open_message_link(app: &mut App, url: &str) {
    diagnostics::event("link_open", "target=browser");
    match links::open_url(url) {
        Ok(()) => app.state.set_status(LINK_OPENED_STATUS),
        Err(error) => {
            diagnostics::event("link_open_error", format!("error={error}"));
            app.state
                .set_error(format!("{OPEN_LINK_FAILED_PREFIX}: {error}"));
        }
    }
}

fn key_event_label(key: KeyEvent, focus: state::FocusedPanel) -> String {
    match key.code {
        KeyCode::Char(_) if focus == state::FocusedPanel::Input => "Char(redacted)".to_string(),
        KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            "Ctrl+Char(redacted)".to_string()
        }
        KeyCode::Char(_) => "Char(command)".to_string(),
        other => format!("{other:?}"),
    }
}

async fn handle_key_event<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    key: KeyEvent,
    client: &mut C,
) -> Result<()> {
    let mut progress = UiProgress::Silent;
    handle_key_event_with_progress(app, key, client, &mut progress, HandlerLoaders::none()).await
}

async fn handle_key_event_with_progress<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    key: KeyEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    mut loaders: HandlerLoaders<'_, C>,
) -> Result<()> {
    diagnostics::event(
        "key_event",
        format!(
            "focus={} key={} modifiers={:?}",
            app.state.focused_panel.label(),
            key_event_label(key, app.state.focused_panel),
            key.modifiers
        ),
    );
    if app.state.context_menu().is_some() {
        let action = match key.code {
            KeyCode::Esc => {
                app.state.close_context_menu();
                None
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.state.close_context_menu();
                None
            }
            KeyCode::Up => {
                app.state.move_context_menu_highlight(-1);
                None
            }
            KeyCode::Down => {
                app.state.move_context_menu_highlight(1);
                None
            }
            KeyCode::Enter => app.state.take_highlighted_context_menu_action(),
            _ => None,
        };
        if let Some((target, action)) = action {
            execute_context_menu_action(app, client, progress, &mut loaders, target, action)
                .await?;
        }
        return Ok(());
    }

    if global_keys::handle_global_key(&mut app.state, key) == global_keys::GlobalKeyOutcome::Handled
    {
        diagnostics::event("key_handled", "handler=global");
        return Ok(());
    }

    if app.state.focused_panel == state::FocusedPanel::Input {
        handle_input_focused(
            app,
            key,
            client,
            progress,
            loaders.send_message,
            loaders.edit_message,
            loaders.reply_message,
        )
        .await?;
    } else {
        handle_normal_navigation(app, key, client, progress, loaders).await?;
    }
    Ok(())
}

async fn handle_normal_navigation<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    key: KeyEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    mut loaders: HandlerLoaders<'_, C>,
) -> Result<()> {
    if app.state.delete_confirmation().is_some() {
        match confirm_keys::handle_confirm_key(key) {
            confirm_keys::ConfirmKeyOutcome::Confirm => {
                progress.show(app, DELETING_MESSAGE_STATUS)?;
                if let Some(loader) = loaders.delete_message {
                    if let Some(confirmation) = actions::begin_confirm_delete(&mut app.state) {
                        loader.spawn_delete_message(confirmation);
                    }
                } else {
                    actions::confirm_delete(&mut app.state, client).await?;
                }
            }
            confirm_keys::ConfirmKeyOutcome::Cancel => {
                app.state.cancel_delete_confirmation();
            }
            confirm_keys::ConfirmKeyOutcome::Ignored => {}
        }
        return Ok(());
    }

    if let Some(navigation) = older_message_key_navigation(app, key) {
        progress.show(app, LOADING_OLDER_MESSAGES_STATUS)?;
        if let Some(loader) = loaders.older_message {
            if let Some((chat_id, topic_id, before_message_id)) =
                actions::selected_older_messages_request(&mut app.state)
            {
                loader.spawn_older_messages(chat_id, topic_id, before_message_id, navigation);
            }
        } else {
            let loaded = actions::load_older_selected_chat_messages(&mut app.state, client).await?;
            if loaded > 0 {
                app.state.clear_status();
                apply_older_message_navigation(&mut app.state, navigation);
            }
        }
        return Ok(());
    }

    match message_keys::handle_message_key(&mut app.state, key) {
        message_keys::MessageKeyOutcome::Handled => return Ok(()),
        message_keys::MessageKeyOutcome::OpenSelectedThreadTopic => {
            open_selected_thread_topic_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.chat_message,
            )
            .await?;
            return Ok(());
        }
        message_keys::MessageKeyOutcome::OpenSelectedLink => {
            open_selected_message_link(app);
            return Ok(());
        }
        message_keys::MessageKeyOutcome::CopySelectedText => {
            copy_selected_message_text(app, progress)?;
            return Ok(());
        }
        message_keys::MessageKeyOutcome::DownloadSelectedMedia => {
            download_selected_media_with_optional_async_loader(
                app,
                client,
                progress,
                loaders.download_media,
            )
            .await?;
            return Ok(());
        }
        message_keys::MessageKeyOutcome::OpenDownloadedMedia => {
            open_selected_downloaded_media(app);
            return Ok(());
        }
        message_keys::MessageKeyOutcome::Ignored => {}
    }

    match chat_keys::handle_chat_key(&mut app.state, key) {
        chat_keys::ChatKeyOutcome::Handled => return Ok(()),
        chat_keys::ChatKeyOutcome::OpenNextChat => {
            let index = app.state.selected_chat_index + 1;
            open_chat_at_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.chat_message,
                index,
            )
            .await?;
            return Ok(());
        }
        chat_keys::ChatKeyOutcome::OpenChatAt(index) => {
            open_chat_at_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.chat_message,
                index,
            )
            .await?;
            return Ok(());
        }
        chat_keys::ChatKeyOutcome::Ignored => {}
    }

    match folder_keys::handle_folder_key(&mut app.state, key) {
        folder_keys::FolderKeyOutcome::Handled => return Ok(()),
        folder_keys::FolderKeyOutcome::OpenPreviousFolder => {
            let Some(index) = previous_folder_index(&app.state) else {
                return Ok(());
            };
            open_folder_at_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.folder_chat,
                index,
            )
            .await?;
            return Ok(());
        }
        folder_keys::FolderKeyOutcome::OpenNextFolder => {
            let Some(index) = next_folder_index(&app.state) else {
                return Ok(());
            };
            open_folder_at_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.folder_chat,
                index,
            )
            .await?;
            return Ok(());
        }
        folder_keys::FolderKeyOutcome::Ignored => {}
    }

    let preferences_before = preferences::AppPreferences::from_state(&app.state);
    match app_keys::handle_app_key(&mut app.state, key) {
        app_keys::AppKeyOutcome::Handled => {
            save_app_preferences_if_changed(app, preferences_before)
        }
        app_keys::AppKeyOutcome::Ignored => {}
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

fn previous_folder_index(state: &state::AppState) -> Option<usize> {
    if state.folders.is_empty() {
        None
    } else if state.selected_folder_index == 0 {
        Some(state.folders.len() - 1)
    } else {
        Some(state.selected_folder_index - 1)
    }
}

fn next_folder_index(state: &state::AppState) -> Option<usize> {
    if state.folders.is_empty() {
        None
    } else {
        Some((state.selected_folder_index + 1) % state.folders.len())
    }
}

async fn open_folder_at_with_optional_async_loader<
    C: TelegramClient + Clone + Send + Sync + 'static,
>(
    app: &mut App,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    folder_chat_loader: &mut Option<&mut FolderChatLoader<C>>,
    index: usize,
) -> Result<()> {
    let Some((folder_index, folder_id)) = actions::begin_open_folder_at(&mut app.state, index)
    else {
        return Ok(());
    };

    progress.show(app, LOADING_FOLDER_CHATS_STATUS)?;
    if let Some(loader) = folder_chat_loader.as_deref_mut() {
        loader.spawn_folder_chats(folder_index, folder_id);
        return Ok(());
    }

    let result = actions::fetch_folder_chats_and_selected_messages(client, folder_id).await;
    actions::apply_folder_chat_load_result(&mut app.state, result);
    app.state.clear_status();
    Ok(())
}

async fn open_selected_thread_topic_with_optional_async_loader<
    C: TelegramClient + Clone + Send + Sync + 'static,
>(
    app: &mut App,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    chat_message_loader: &mut Option<&mut ChatMessageLoader<C>>,
) -> Result<()> {
    let Some((chat_id, topic_id)) = app
        .state
        .selected_chat_id()
        .zip(app.state.selected_thread_topic().map(|topic| topic.id))
    else {
        return Ok(());
    };

    app.state.begin_conversation_load();
    progress.show(app, LOADING_CHAT_MESSAGES_STATUS)?;
    if let Some(loader) = chat_message_loader.as_deref_mut() {
        loader.spawn_thread_topic_messages(chat_id, topic_id);
        return Ok(());
    }

    let result = actions::load_selected_thread_topic_messages(&mut app.state, client).await;
    if result.is_ok() {
        app.state.clear_status();
    }
    result
}

async fn open_chat_at_with_optional_async_loader<
    C: TelegramClient + Clone + Send + Sync + 'static,
>(
    app: &mut App,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    chat_message_loader: &mut Option<&mut ChatMessageLoader<C>>,
    index: usize,
) -> Result<()> {
    app.state.clear_chat_search();
    let Some(chat_id) = actions::begin_open_chat_at(&mut app.state, index) else {
        return Ok(());
    };

    progress.show(app, LOADING_CHAT_MESSAGES_STATUS)?;
    if let Some(loader) = chat_message_loader.as_deref_mut() {
        loader.spawn_latest_chat_messages(chat_id);
        return Ok(());
    }

    actions::load_selected_chat_messages(&mut app.state, client).await?;
    app.state.clear_status();
    Ok(())
}

async fn handle_input_focused<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    key: KeyEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    send_message_loader: Option<&SendMessageLoader<C>>,
    edit_message_loader: Option<&EditMessageLoader<C>>,
    reply_message_loader: Option<&ReplyMessageLoader<C>>,
) -> Result<()> {
    let input_before = app.state.input_buffer.clone();
    let outcome = input_keys::handle_input_key(&mut app.state, key);
    if outcome != input_keys::InputKeyOutcome::Submit && input_before != app.state.input_buffer {
        if app.state.input_has_submit_text() {
            if let Some(chat_id) = app.state.selected_chat_id() {
                let topic_id = app.state.selected_thread_topic().map(|topic| topic.id);
                if app.state.typing_action_due(chat_id, topic_id) {
                    let client = client.clone();
                    tokio::spawn(async move {
                        actions::send_typing_action_best_effort(&client, chat_id, topic_id).await;
                    });
                }
            }
        } else {
            app.state.reset_typing_action_cooldown();
        }
    }

    if outcome == input_keys::InputKeyOutcome::Submit {
        let Some(action) = app.state.prepare_message_submit() else {
            return Ok(());
        };

        match action {
            state::MessageSubmitAction::Send {
                chat_id,
                thread_top_message_id,
                content,
            } => {
                let pending = actions::begin_send_message(
                    &mut app.state,
                    chat_id,
                    thread_top_message_id,
                    content,
                );
                progress.show(app, SENDING_MESSAGE_STATUS)?;
                if let Some(loader) = send_message_loader {
                    loader.spawn_send_message(pending);
                } else {
                    actions::finish_send_message(&mut app.state, client, pending).await?;
                }
            }
            state::MessageSubmitAction::Edit {
                chat_id,
                message_id,
                content,
            } => {
                progress.show(app, SAVING_EDIT_STATUS)?;
                if let Some(loader) = edit_message_loader {
                    loader.spawn_edit_message(chat_id, message_id, content);
                } else {
                    actions::execute_message_submit_action(
                        &mut app.state,
                        client,
                        state::MessageSubmitAction::Edit {
                            chat_id,
                            message_id,
                            content,
                        },
                    )
                    .await?;
                }
            }
            state::MessageSubmitAction::Reply {
                chat_id,
                thread_top_message_id,
                message_id,
                content,
            } => {
                progress.show(app, SENDING_REPLY_STATUS)?;
                if let Some(loader) = reply_message_loader {
                    loader.spawn_reply_message(chat_id, thread_top_message_id, message_id, content);
                } else {
                    actions::execute_message_submit_action(
                        &mut app.state,
                        client,
                        state::MessageSubmitAction::Reply {
                            chat_id,
                            thread_top_message_id,
                            message_id,
                            content,
                        },
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn message_submit_action_status(action: &state::MessageSubmitAction) -> &'static str {
    match action {
        state::MessageSubmitAction::Edit { .. } => SAVING_EDIT_STATUS,
        state::MessageSubmitAction::Reply { .. } => SENDING_REPLY_STATUS,
        state::MessageSubmitAction::Send { .. } => SENDING_MESSAGE_STATUS,
    }
}

async fn execute_context_menu_action<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    loaders: &mut HandlerLoaders<'_, C>,
    target: state::ContextMenuTarget,
    action: state::ContextMenuAction,
) -> Result<()> {
    if !app
        .state
        .context_actions_for_target(target)
        .contains(&action)
    {
        return Ok(());
    }

    match (target, action) {
        (state::ContextMenuTarget::Chat { chat_id }, state::ContextMenuAction::OpenChat) => {
            let Some(index) = app.state.chats.iter().position(|chat| chat.id == chat_id) else {
                return Ok(());
            };
            open_chat_at_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.chat_message,
                index,
            )
            .await?;
        }
        (state::ContextMenuTarget::Chat { chat_id }, state::ContextMenuAction::MarkChatRead) => {
            if app.state.mark_chat_read_locally(chat_id) {
                if let Some(loader) = loaders.mark_read {
                    loader.spawn_mark_chat_read(chat_id);
                } else {
                    actions::mark_chat_read_best_effort(client, chat_id).await;
                }
            }
        }
        (state::ContextMenuTarget::Chat { chat_id }, state::ContextMenuAction::CopyChatName) => {
            let Some(name) = app
                .state
                .chats
                .iter()
                .find(|chat| chat.id == chat_id)
                .map(|chat| chat.name.clone())
            else {
                return Ok(());
            };
            progress.copy_text(&name)?;
            app.state.set_status(CHAT_NAME_COPIED_STATUS);
        }
        (
            state::ContextMenuTarget::Message {
                chat_id,
                message_id,
            },
            message_action,
        ) => {
            if !app.state.select_message_by_identity(chat_id, message_id) {
                return Ok(());
            }
            match message_action {
                state::ContextMenuAction::ReplyMessage => {
                    app.state.request_reply_to_selected_message()
                }
                state::ContextMenuAction::EditMessage => app.state.request_edit_selected_message(),
                state::ContextMenuAction::CopyMessageText => {
                    copy_selected_message_text(app, progress)?
                }
                state::ContextMenuAction::OpenMessageLink => open_selected_message_link(app),
                state::ContextMenuAction::SaveMessageMedia => {
                    download_selected_media_with_optional_async_loader(
                        app,
                        client,
                        progress,
                        loaders.download_media,
                    )
                    .await?;
                }
                state::ContextMenuAction::OpenDownloadedMedia => {
                    open_selected_downloaded_media(app)
                }
                state::ContextMenuAction::DeleteMessage
                | state::ContextMenuAction::DismissFailedSend => {
                    app.state.request_delete_selected_message()
                }
                state::ContextMenuAction::OpenChat
                | state::ContextMenuAction::MarkChatRead
                | state::ContextMenuAction::CopyChatName => {}
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_mouse_event<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    mouse_event: crossterm::event::MouseEvent,
    client: &mut C,
) -> Result<()> {
    let mut progress = UiProgress::Silent;
    handle_mouse_event_with_progress(
        app,
        mouse_event,
        client,
        &mut progress,
        HandlerLoaders::none(),
    )
    .await
}

async fn handle_mouse_event_with_progress<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    mouse_event: crossterm::event::MouseEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    mut loaders: HandlerLoaders<'_, C>,
) -> Result<()> {
    if app.state.delete_confirmation().is_some() {
        diagnostics::event("mouse_ignored", "reason=delete_confirmation");
        return Ok(());
    }

    match mouse_events::handle_mouse_scroll(&mut app.state, mouse_event) {
        mouse_events::MouseScrollOutcome::Handled => return Ok(()),
        mouse_events::MouseScrollOutcome::Ignored => {}
    }

    match mouse_events::handle_mouse_click(&mut app.state, mouse_event) {
        mouse_events::MouseClickOutcome::Handled | mouse_events::MouseClickOutcome::Ignored => {}
        mouse_events::MouseClickOutcome::OpenLink(url) => open_message_link(app, &url),
        mouse_events::MouseClickOutcome::ContextMenuAction(target, action) => {
            execute_context_menu_action(app, client, progress, &mut loaders, target, action).await?
        }
        mouse_events::MouseClickOutcome::OpenFolderAt(index) => {
            diagnostics::event(
                "mouse_action",
                format!(
                    "action=open_folder_at index={index} column={} row={}",
                    mouse_event.column, mouse_event.row
                ),
            );
            open_folder_at_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.folder_chat,
                index,
            )
            .await?;
        }
        mouse_events::MouseClickOutcome::OpenChatAt(index) => {
            diagnostics::event(
                "mouse_action",
                format!(
                    "action=open_chat_at index={index} column={} row={}",
                    mouse_event.column, mouse_event.row
                ),
            );
            open_chat_at_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.chat_message,
                index,
            )
            .await?;
        }
        mouse_events::MouseClickOutcome::OpenThreadTopicAt(index) => {
            diagnostics::event(
                "mouse_action",
                format!(
                    "action=open_thread_topic_at index={index} column={} row={}",
                    mouse_event.column, mouse_event.row
                ),
            );
            open_selected_thread_topic_with_optional_async_loader(
                app,
                client,
                progress,
                &mut loaders.chat_message,
            )
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        APP_COMMAND, CHECK_AUTH_OK_PREFIX, CHECK_CONFIG_AUTH_CONFLICT,
        CHECK_CONFIG_SESSION_EXISTS_STATUS, CHECK_CONFIG_SESSION_WILL_CREATE_STATUS,
        CLI_USAGE_EXIT_CODE, CONFIG_LOAD_HELP, CONFIG_PATH_ARGUMENT_REQUIRED, ChatMessageLoad,
        ChatMessageLoadResult, ChatMessageLoader, DeleteMessageLoader, DeleteMessageResult,
        EditMessageLoader, EditMessageResult, EventLoopState, FolderChatLoadResult,
        FolderChatLoader, FrameScheduler, HandlerLoaders, InitialStateLoadResult,
        InitialStateLoader, LOADING_CHAT_MESSAGES_STATUS, LOADING_TELEGRAM_STATUS,
        LOG_PATH_ARGUMENT_REQUIRED, LOGIN_2FA_ENABLED_STATUS, LOGIN_2FA_HINT_PREFIX,
        LOGIN_2FA_PROMPT, LOGIN_2FA_SIGNED_IN_PREFIX, LOGIN_CODE_PROMPT, LOGIN_CODE_SENT_PREFIX,
        LOGIN_FAILED_PREFIX, LOGIN_HEADER, LOGIN_PHONE_PROMPT, LOGIN_REQUESTING_CODE_STATUS,
        LOGIN_SESSION_SAVED_STATUS, LOGIN_SIGNED_IN_PREFIX, LOGIN_SIGNING_IN_STATUS,
        LOGIN_START_PROMPT, MIN_FRAME_INTERVAL, MarkChatReadLoader, MediaPreviewLoader,
        MediaPreviewResult, OlderMessageLoadResult, OlderMessageLoader, OlderMessageNavigation,
        PROMPT_EMPTY_ERROR, PROMPT_EOF_ERROR, RECONCILIATION_FOCUS_STALE_AFTER,
        RECONCILIATION_INTERVAL, ReconciliationLoader, ReconciliationResult, ReplyMessageLoader,
        ReplyMessageResult, RunMode, SAVING_EDIT_STATUS, SENDING_MESSAGE_STATUS,
        SENDING_REPLY_STATUS, SETUP_ERROR_EXIT_CODE, SMOKE_CHECK_AUTH_CONFLICT,
        SMOKE_CHECK_CONFIG_CONFLICT, SMOKE_OK_PREFIX, SendMessageLoader, SendMessageResult,
        SubscribeUpdatesLoader, SubscribeUpdatesResult, TerminalAction, TokioInstant,
        UPDATE_SUBSCRIPTION_RETRY_DELAY, UiProgress, abort_running_task,
        apply_chat_message_load_result, apply_delete_message_result, apply_edit_message_result,
        apply_folder_chat_load_result, apply_initial_state_load_result, apply_media_preview_result,
        apply_older_message_load_result, apply_reconciliation_result, apply_reply_message_result,
        apply_send_message_result, apply_subscribe_updates_result, apply_update_with_read_ack,
        check_auth_ok_message, check_auth_unauthorized_message, check_config_message,
        check_config_session_status, classify_terminal_event, default_config_path_string,
        drain_ready_results, ensure_session_parent_dir, handle_input_focused, handle_key_event,
        handle_key_event_with_progress, handle_mouse_event, handle_received_update,
        load_checked_config, load_checked_config_with_session_parent, login_2fa_hint_message,
        login_2fa_signed_in_message, login_code_sent_message, login_failed_message,
        login_signed_in_message, message_submit_action_status, older_message_key_navigation,
        open_chat_at_with_optional_async_loader, parse_args_from, prepare_loop_step,
        preserve_prompt_input_line_spaces, require_prompt_line, require_prompt_response,
        save_app_preferences, save_app_preferences_if_changed, sleep_until_optional,
        smoke_ok_message, trim_prompt_input_line, validate_config,
    };
    use crate::app::App;
    use crate::config::telegram::{Config, TelegramConfig};
    use crate::state::{
        ContextMenuTarget, ConversationLoadStatus, DeleteConfirmation, FocusedPanel,
        ReconciliationSnapshot,
    };
    use crate::telegram::{
        MockTelegramClient, TelegramClient,
        types::{
            Chat, Folder, Message, MessageMedia, MessageStatus, ThreadTopic, Update, all_folder,
        },
    };
    use chrono::Utc;
    use color_eyre::Result;
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use ratatui::layout::Rect;
    use std::{
        collections::HashMap,
        path::Path,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::mpsc;

    static TEST_TEMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

    const TEST_OLDER_SCROLL_MESSAGE_AREA_WIDTH: u16 = 40;
    const TEST_OLDER_SCROLL_MESSAGE_AREA_HEIGHT: u16 = 10;

    fn older_scroll_message_area() -> Rect {
        Rect::new(
            0,
            0,
            TEST_OLDER_SCROLL_MESSAGE_AREA_WIDTH,
            TEST_OLDER_SCROLL_MESSAGE_AREA_HEIGHT,
        )
    }

    fn unique_temp_session_path() -> std::path::PathBuf {
        let clock_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let counter = TEST_TEMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);

        std::env::temp_dir().join(format!(
            "dumbgram-tui-test-{}-{clock_nanos}-{counter}",
            std::process::id()
        ))
    }

    fn folder(id: i32, name: &str) -> Folder {
        Folder {
            id,
            name: name.to_string(),
            unread_count: 0,
        }
    }

    fn chat(id: i64) -> Chat {
        Chat {
            id,
            name: format!("Chat {id}"),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }
    }

    fn message(id: i32) -> Message {
        Message {
            id,
            chat_id: 10,
            thread_topic_id: None,
            sender_name: "Alice".to_string(),
            content: format!("message {id}"),
            timestamp: Utc::now(),
            is_own: false,
            is_edited: false,
            reply_to_content: None,
            media: None,
            status: MessageStatus::Delivered,
            can_edit: false,
            can_delete: false,
            error: None,
        }
    }

    fn thread_topic(id: i32, unread_count: usize) -> ThreadTopic {
        ThreadTopic {
            id,
            title: format!("Topic {id}"),
            top_message_id: id,
            unread_count,
            is_closed: false,
            is_pinned: false,
        }
    }

    #[test]
    fn terminal_event_classification_preserves_input_semantics() {
        let press =
            KeyEvent::new_with_kind(KeyCode::Char('j'), KeyModifiers::NONE, KeyEventKind::Press);
        assert_eq!(
            classify_terminal_event(Event::Key(press)),
            TerminalAction::Key(press)
        );
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            assert_eq!(
                classify_terminal_event(Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                    kind,
                ))),
                TerminalAction::Ignore
            );
        }
        assert_eq!(
            classify_terminal_event(Event::Resize(100, 40)),
            TerminalAction::Resize
        );
        assert_eq!(
            classify_terminal_event(Event::Paste("ignored".to_string())),
            TerminalAction::Ignore
        );
        assert_eq!(
            classify_terminal_event(Event::FocusGained),
            TerminalAction::FocusGained
        );
        assert_eq!(
            classify_terminal_event(Event::FocusLost),
            TerminalAction::FocusLost
        );
        assert!(matches!(
            classify_terminal_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 1,
                row: 2,
                modifiers: KeyModifiers::NONE,
            })),
            TerminalAction::Mouse(_)
        ));
    }

    #[test]
    fn frame_scheduler_skips_idle_and_coalesces_result_input_bursts_at_frame_cap() {
        let now = tokio::time::Instant::now();
        let mut frames = FrameScheduler::new(false);
        assert!(frames.frame_deadline().is_none());
        assert!(frames.take_due_frame(now).is_none());

        frames.mark_dirty(now);
        frames.mark_dirty(now);
        assert_eq!(frames.take_due_frame(now), Some(Duration::ZERO));
        assert!(frames.take_due_frame(now).is_none());

        frames.mark_dirty(now);
        assert!(
            frames
                .take_due_frame(now + MIN_FRAME_INTERVAL - Duration::from_millis(1))
                .is_none()
        );
        assert_eq!(
            frames.take_due_frame(now + MIN_FRAME_INTERVAL),
            Some(MIN_FRAME_INTERVAL)
        );
        assert!(frames.frame_deadline().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn reconciliation_and_subscription_scheduling_are_single_flight() {
        let (mut loop_state, _) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.subscription_pending = false;
        loop_state.last_reconciliation_success_at = Some(TokioInstant::now());

        loop_state.schedule_focus_reconciliation();
        assert!(loop_state.next_reconciliation_at.is_none());
        tokio::time::advance(RECONCILIATION_FOCUS_STALE_AFTER + Duration::from_millis(1)).await;
        loop_state.schedule_focus_reconciliation();
        let focus_deadline = loop_state
            .next_reconciliation_at
            .expect("stale focus should schedule reconciliation");
        loop_state.schedule_focus_reconciliation();
        assert_eq!(loop_state.next_reconciliation_at, Some(focus_deadline));
        assert!(loop_state.announce_reconciliation_success);

        loop_state.reconciliation_pending = true;
        loop_state.next_reconciliation_at = None;
        loop_state.schedule_reconciliation_now();
        assert!(loop_state.reconciliation_requested_while_pending);
        let now = TokioInstant::now();
        loop_state.finish_reconciliation_gate(now + RECONCILIATION_INTERVAL);
        assert_eq!(loop_state.next_reconciliation_at, Some(now));
        assert!(!loop_state.reconciliation_requested_while_pending);

        loop_state.schedule_subscription_retry();
        let subscription_deadline = loop_state
            .next_subscription_at
            .expect("closed stream should schedule subscription retry");
        loop_state.schedule_subscription_retry();
        assert_eq!(loop_state.next_subscription_at, Some(subscription_deadline));
        assert_eq!(
            subscription_deadline.saturating_duration_since(TokioInstant::now()),
            UPDATE_SUBSCRIPTION_RETRY_DELAY
        );
    }

    #[tokio::test]
    async fn absent_notification_deadline_creates_no_wake_timer() {
        assert!(
            tokio::time::timeout(Duration::from_millis(10), sleep_until_optional(None))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn ui_sender_wake_permit_is_not_lost_before_wait() {
        let wake = Arc::new(tokio::sync::Notify::new());
        let (sender, mut rx) = super::ui_channel(&wake);

        sender.send(42).expect("receiver should be open");
        tokio::time::timeout(Duration::from_millis(20), wake.notified())
            .await
            .expect("send-before-wait should retain a wake permit");
        assert_eq!(rx.try_recv(), Ok(42));
    }

    #[tokio::test]
    async fn canonical_drain_orders_initial_replay_before_new_update() {
        let (mut loop_state, senders) = EventLoopState::new();
        let (update_tx, update_rx) = mpsc::unbounded_channel();
        update_tx
            .send(Update::Error("queued".to_string()))
            .expect("update receiver should be open");
        senders
            .subscribe_updates
            .send(SubscribeUpdatesResult {
                request_id: 0,
                result: Ok(update_rx),
            })
            .expect("subscription receiver should be open");
        senders
            .initial_state
            .send(InitialStateLoadResult {
                result: Err("initial".to_string()),
            })
            .expect("initial receiver should be open");
        loop_state.deferred_updates = vec![
            Update::Error("deferred-1".to_string()),
            Update::Error("deferred-2".to_string()),
        ];
        loop_state.staged_update = Some(Update::Error("staged".to_string()));

        let client = MockTelegramClient::new();
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let reconciliation_loader =
            ReconciliationLoader::new(client.clone(), senders.reconciliation);
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);
        let mut app = App::new();

        assert!(drain_ready_results(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        ));
        assert_eq!(
            loop_state.drain_trace,
            [
                "subscription",
                "initial",
                "update:deferred-1",
                "update:deferred-2",
                "update:staged",
                "update:queued",
            ]
        );
        assert_eq!(app.state.error_message.as_deref(), Some("queued"));
    }

    #[test]
    fn reconciliation_applies_snapshot_before_deferred_updates() {
        let (mut loop_state, senders) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.reconciliation_pending = true;
        let mut app = App::new();
        app.state.folders = vec![all_folder(0)];
        app.state.chats = vec![chat(1)];
        let mut first = message(1);
        first.chat_id = 1;
        app.state.messages = vec![first];
        let context = app.state.reconciliation_context();

        let client = MockTelegramClient::new();
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let reconciliation_loader =
            ReconciliationLoader::new(client.clone(), senders.reconciliation.clone());
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);

        let mut deferred = message(3);
        deferred.chat_id = 1;
        assert!(!handle_received_update(
            &mut loop_state,
            &mut app,
            Update::NewMessage(deferred),
            &mark_read_loader,
        ));
        assert_eq!(loop_state.deferred_updates.len(), 1);

        let mut snapshot_message = message(2);
        snapshot_message.chat_id = 1;
        senders
            .reconciliation
            .send(ReconciliationResult {
                request_id: 0,
                context,
                result: Ok(ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(1)],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(1),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: vec![snapshot_message],
                }),
            })
            .expect("reconciliation receiver should be open");

        assert!(drain_ready_results(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        ));
        assert_eq!(
            app.state
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert!(loop_state.deferred_updates.is_empty());
        assert!(!loop_state.reconciliation_pending);
    }

    #[test]
    fn reconciliation_deduplicates_background_updates_already_in_snapshot() {
        let (mut loop_state, senders) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.reconciliation_pending = true;
        let mut app = App::new();
        app.state.folders = vec![all_folder(0)];
        app.state.chats = vec![chat(1), chat(2)];
        let context = app.state.reconciliation_context();

        let client = MockTelegramClient::new();
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let reconciliation_loader =
            ReconciliationLoader::new(client.clone(), senders.reconciliation.clone());
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);

        let mut background = message(3);
        background.chat_id = 2;
        assert!(!handle_received_update(
            &mut loop_state,
            &mut app,
            Update::NewMessage(background),
            &mark_read_loader,
        ));
        let mut refreshed_background = chat(2);
        refreshed_background.unread_count = 1;
        let mut high_water = std::collections::HashMap::new();
        high_water.insert(2, 3);
        senders
            .reconciliation
            .send(ReconciliationResult {
                request_id: 0,
                context,
                result: Ok(ReconciliationSnapshot {
                    folders: vec![all_folder(1)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(1), refreshed_background],
                    chat_last_message_ids: high_water,
                    selected_chat_id: Some(1),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: Vec::new(),
                }),
            })
            .expect("reconciliation receiver should be open");

        assert!(drain_ready_results(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        ));
        assert_eq!(app.state.chats[1].unread_count, 1);
        assert!(loop_state.deferred_updates.is_empty());
    }

    #[test]
    fn stale_reconciliation_replays_deferred_updates_without_new_high_water_filtering() {
        let (mut loop_state, senders) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.reconciliation_pending = true;
        let mut app = App::new();
        app.state.folders = vec![all_folder(0)];
        app.state.chats = vec![chat(1), chat(2)];
        let context = app.state.reconciliation_context();
        app.state.selected_chat_index = 1;
        app.state.messages.clear();

        let client = MockTelegramClient::new();
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let reconciliation_loader =
            ReconciliationLoader::new(client.clone(), senders.reconciliation.clone());
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);

        let mut deferred = message(3);
        deferred.chat_id = 2;
        assert!(!handle_received_update(
            &mut loop_state,
            &mut app,
            Update::NewMessage(deferred),
            &mark_read_loader,
        ));
        let mut high_water = HashMap::new();
        high_water.insert(2, 3);
        senders
            .reconciliation
            .send(ReconciliationResult {
                request_id: 0,
                context,
                result: Ok(ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(1), chat(2)],
                    chat_last_message_ids: high_water,
                    selected_chat_id: Some(1),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: Vec::new(),
                }),
            })
            .expect("reconciliation receiver should be open");

        assert!(drain_ready_results(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        ));
        assert_eq!(app.state.selected_chat_id(), Some(2));
        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].id, 3);
        assert!(loop_state.reconciliation_high_water_ids.is_empty());
    }

    #[test]
    fn queued_background_update_uses_accepted_snapshot_high_water() {
        let (mut loop_state, senders) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.reconciliation_pending = true;
        let mut app = App::new();
        app.state.folders = vec![all_folder(0)];
        app.state.chats = vec![chat(1), chat(2)];
        let context = app.state.reconciliation_context();
        let (update_tx, update_rx) = mpsc::unbounded_channel();
        loop_state.update_rx = Some(update_rx);
        let mut queued = message(3);
        queued.chat_id = 2;
        update_tx
            .send(Update::NewMessage(queued))
            .expect("update receiver should be open");

        let client = MockTelegramClient::new();
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let reconciliation_loader =
            ReconciliationLoader::new(client.clone(), senders.reconciliation.clone());
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);

        let mut refreshed_background = chat(2);
        refreshed_background.unread_count = 1;
        let mut high_water = HashMap::new();
        high_water.insert(2, 3);
        senders
            .reconciliation
            .send(ReconciliationResult {
                request_id: 0,
                context,
                result: Ok(ReconciliationSnapshot {
                    folders: vec![all_folder(1)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(1), refreshed_background],
                    chat_last_message_ids: high_water,
                    selected_chat_id: Some(1),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: Vec::new(),
                }),
            })
            .expect("reconciliation receiver should be open");

        assert!(drain_ready_results(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        ));
        assert_eq!(app.state.chats[1].unread_count, 1);
    }

    #[test]
    fn represented_other_topic_update_is_not_replayed_into_selected_topic_state() {
        let (mut loop_state, senders) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.reconciliation_pending = true;
        let mut app = App::new();
        app.state.folders = vec![all_folder(0)];
        app.state.chats = vec![chat(1)];
        app.state.thread_topics = vec![thread_topic(10, 0), thread_topic(20, 0)];
        let mut selected_message = message(1);
        selected_message.chat_id = 1;
        selected_message.thread_topic_id = Some(10);
        app.state.messages = vec![selected_message.clone()];
        let context = app.state.reconciliation_context();

        let client = MockTelegramClient::new();
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let reconciliation_loader =
            ReconciliationLoader::new(client.clone(), senders.reconciliation.clone());
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);

        let mut other_topic_message = message(30);
        other_topic_message.chat_id = 1;
        other_topic_message.thread_topic_id = Some(20);
        assert!(!handle_received_update(
            &mut loop_state,
            &mut app,
            Update::NewMessage(other_topic_message),
            &mark_read_loader,
        ));
        let mut refreshed_chat = chat(1);
        refreshed_chat.unread_count = 1;
        let mut high_water = HashMap::new();
        high_water.insert(1, 30);
        senders
            .reconciliation
            .send(ReconciliationResult {
                request_id: 0,
                context,
                result: Ok(ReconciliationSnapshot {
                    folders: vec![all_folder(1)],
                    selected_folder_id: Some(0),
                    chats: vec![refreshed_chat],
                    chat_last_message_ids: high_water,
                    selected_chat_id: Some(1),
                    thread_topics: vec![thread_topic(10, 0), thread_topic(20, 1)],
                    selected_topic_id: Some(10),
                    messages: vec![selected_message],
                }),
            })
            .expect("reconciliation receiver should be open");

        assert!(drain_ready_results(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        ));
        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.chats[0].unread_count, 1);
        assert_eq!(app.state.thread_topics[1].unread_count, 1);
    }

    #[test]
    fn selected_chat_tail_update_is_replayed_when_older_history_is_preserved() {
        let (mut loop_state, senders) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.reconciliation_pending = true;
        let mut app = App::new();
        app.state.folders = vec![all_folder(0)];
        app.state.chats = vec![chat(1)];
        app.state.messages = (1..=3)
            .map(|id| {
                let mut message = message(id);
                message.chat_id = 1;
                message
            })
            .collect();
        app.state.selected_message_index = 0;
        let context = app.state.reconciliation_context();

        let client = MockTelegramClient::new();
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let reconciliation_loader =
            ReconciliationLoader::new(client.clone(), senders.reconciliation.clone());
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);

        let mut tail = message(4);
        tail.chat_id = 1;
        assert!(!handle_received_update(
            &mut loop_state,
            &mut app,
            Update::NewMessage(tail.clone()),
            &mark_read_loader,
        ));
        let mut high_water = HashMap::new();
        high_water.insert(1, 4);
        senders
            .reconciliation
            .send(ReconciliationResult {
                request_id: 0,
                context,
                result: Ok(ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(1)],
                    chat_last_message_ids: high_water,
                    selected_chat_id: Some(1),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: vec![tail],
                }),
            })
            .expect("reconciliation receiver should be open");

        assert!(drain_ready_results(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        ));
        assert_eq!(
            app.state
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            app.state.selected_message().map(|message| message.id),
            Some(1)
        );
    }

    #[tokio::test]
    async fn topic_reconciliation_marks_only_the_selected_thread_read() {
        let (mut loop_state, _) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.reconciliation_pending = true;
        let mut app = App::new();
        let mut local_chat = chat(1);
        local_chat.unread_count = 3;
        app.state.folders = vec![all_folder(3)];
        app.state.chats = vec![local_chat];
        app.state.thread_topics = vec![thread_topic(10, 0), thread_topic(20, 3)];
        app.state.messages = (1..=3)
            .map(|id| {
                let mut message = message(id);
                message.chat_id = 1;
                message.thread_topic_id = Some(10);
                message
            })
            .collect();
        app.state.selected_message_index = 0;
        let context = app.state.reconciliation_context();
        let marked_chat_ids = Arc::new(Mutex::new(Vec::new()));
        let marked_threads = Arc::new(Mutex::new(Vec::new()));
        let mark_read_loader = MarkChatReadLoader::new(RecordingMarkReadClient {
            marked_chat_ids: Arc::clone(&marked_chat_ids),
            marked_threads: Arc::clone(&marked_threads),
        });
        let mut refreshed_message = message(30);
        refreshed_message.chat_id = 1;
        refreshed_message.thread_topic_id = Some(10);
        let mut refreshed_chat = chat(1);
        refreshed_chat.unread_count = 5;

        assert!(apply_reconciliation_result(
            &mut app,
            ReconciliationResult {
                request_id: 0,
                context,
                result: Ok(ReconciliationSnapshot {
                    folders: vec![all_folder(5)],
                    selected_folder_id: Some(0),
                    chats: vec![refreshed_chat],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(1),
                    thread_topics: vec![thread_topic(10, 2), thread_topic(20, 3)],
                    selected_topic_id: Some(10),
                    messages: vec![refreshed_message],
                }),
            },
            0,
            &mut loop_state,
            &mark_read_loader,
        ));
        tokio::task::yield_now().await;

        assert!(marked_chat_ids.lock().unwrap().is_empty());
        assert_eq!(*marked_threads.lock().unwrap(), vec![(1, 10, 30)]);
        assert_eq!(app.state.chats[0].unread_count, 3);
        assert_eq!(app.state.folders[0].unread_count, 3);
        assert_eq!(
            app.state
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(app.state.thread_topics[0].unread_count, 0);
        assert_eq!(app.state.thread_topics[1].unread_count, 3);
    }

    #[tokio::test(start_paused = true)]
    async fn reconciliation_timeout_releases_deferred_updates() {
        let (mut loop_state, senders) = EventLoopState::new();
        loop_state.initial_state_pending = false;
        loop_state.reconciliation_pending = true;
        let mut app = App::new();
        app.state.folders = vec![all_folder(0)];
        app.state.chats = vec![chat(1)];
        let context = app.state.reconciliation_context();

        let mut reconciliation_loader =
            ReconciliationLoader::new(HangingReconciliationClient, senders.reconciliation);
        reconciliation_loader.spawn_reconciliation(context);
        tokio::task::yield_now().await;
        let client = HangingReconciliationClient;
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);

        let mut incoming = message(7);
        incoming.chat_id = 1;
        assert!(!handle_received_update(
            &mut loop_state,
            &mut app,
            Update::NewMessage(incoming),
            &mark_read_loader,
        ));
        tokio::time::advance(crate::actions::RECONCILIATION_TIMEOUT + Duration::from_millis(1))
            .await;
        tokio::task::yield_now().await;

        assert!(drain_ready_results(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        ));
        assert!(!loop_state.reconciliation_pending);
        assert!(loop_state.deferred_updates.is_empty());
        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].id, 7);
        assert_eq!(
            app.state.error_message.as_deref(),
            Some("Telegram state refresh timed out")
        );
    }

    #[test]
    fn loop_step_applies_ready_results_before_releasing_staged_input() {
        let (mut loop_state, senders) = EventLoopState::new();
        senders
            .initial_state
            .send(InitialStateLoadResult {
                result: Err("result applied first".to_string()),
            })
            .expect("initial receiver should be open");
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        loop_state.staged_terminal_event = Some(Event::Key(key));

        let client = MockTelegramClient::new();
        let subscribe_loader =
            SubscribeUpdatesLoader::new(client.clone(), senders.subscribe_updates);
        let reconciliation_loader =
            ReconciliationLoader::new(client.clone(), senders.reconciliation);
        let chat_loader = ChatMessageLoader::new(client.clone(), senders.chat_message);
        let older_loader = OlderMessageLoader::new(client.clone(), senders.older_message);
        let folder_loader = FolderChatLoader::new(client.clone(), senders.folder_chat);
        let mark_read_loader = MarkChatReadLoader::new(client.clone());
        let preview_loader = MediaPreviewLoader::new(client, senders.media_preview);
        let mut app = App::new();

        let step = prepare_loop_step(
            &mut loop_state,
            &mut app,
            &subscribe_loader,
            &reconciliation_loader,
            &chat_loader,
            &older_loader,
            &folder_loader,
            &mark_read_loader,
            &preview_loader,
        );

        assert!(step.dirty);
        assert_eq!(
            app.state.error_message.as_deref(),
            Some("result applied first")
        );
        assert_eq!(step.terminal_event, Some(Event::Key(key)));
        assert!(loop_state.staged_terminal_event.is_none());
    }

    #[tokio::test]
    async fn selection_input_demands_exactly_one_lazy_preview() {
        let mut app = App::new();
        app.state.chats = vec![chat(10)];
        app.state.messages = vec![message(1), message(2)];
        app.state.messages[1].media = Some(MessageMedia::photo());
        app.state.focused_panel = FocusedPanel::Messages;
        let mut client = MockTelegramClient::new();
        let mut progress = UiProgress::Silent;

        handle_key_event_with_progress(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut client,
            &mut progress,
            HandlerLoaders::none(),
        )
        .await
        .expect("selection input should succeed");
        assert_eq!(app.state.selected_message_index, 1);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut preview_loader = MediaPreviewLoader::new(client, tx);
        preview_loader.request(app.state.selected_media_preview_request());
        preview_loader.request(app.state.selected_media_preview_request());
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("preview should respond")
            .expect("preview receiver should stay open");
        assert!(
            tokio::time::timeout(Duration::from_millis(30), rx.recv())
                .await
                .is_err(),
            "one selection change must create exactly one preview request"
        );
    }

    #[test]
    fn production_frame_and_shutdown_ownership_are_explicit() {
        let source = include_str!("main.rs");
        assert_eq!(source.matches(concat!("terminal", ".draw(")).count(), 2);
        assert_eq!(source.matches(concat!("progress", ".show(")).count(), 9);
        let progress_show = source
            .split("fn show(&mut self")
            .nth(1)
            .and_then(|tail| tail.split("fn is_loading_progress_status").next())
            .expect("progress show body should remain findable");
        assert!(!progress_show.contains(".draw("));

        let run_with_client = source
            .split("async fn run_with_client")
            .nth(1)
            .and_then(|tail| tail.split("fn trim_prompt_input_line").next())
            .expect("run_with_client body should remain findable");
        assert!(
            run_with_client.find("run_app(").expect("run_app call")
                < run_with_client
                    .find("terminal_restore_start")
                    .expect("restore diagnostic")
        );
        let prepare_step = source
            .split("fn prepare_loop_step")
            .nth(1)
            .and_then(|tail| tail.split("async fn dispatch_terminal_event").next())
            .expect("loop-step body should remain findable");
        assert!(
            prepare_step
                .find("drain_ready_results")
                .expect("result drain")
                < prepare_step
                    .find("staged_terminal_event.take")
                    .expect("staged input release")
        );

        let run_app = source
            .split("async fn run_app")
            .nth(1)
            .and_then(|tail| tail.split("async fn run_event_loop").next())
            .expect("run_app body should remain findable");
        assert!(
            run_app.find("drop(events)").expect("event stream drop")
                < run_app
                    .find("terminal_event_stream_stopped")
                    .expect("stream-stop diagnostic")
        );
    }

    #[derive(Clone)]
    struct SlowFirstLatestMessagesClient;

    #[derive(Clone)]
    struct SlowFirstOlderMessagesClient;

    #[derive(Clone)]
    struct RecordingMarkReadClient {
        marked_chat_ids: Arc<Mutex<Vec<i64>>>,
        marked_threads: Arc<Mutex<Vec<(i64, i32, i32)>>>,
    }

    #[derive(Clone)]
    struct HangingReconciliationClient;

    impl TelegramClient for HangingReconciliationClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            std::future::pending().await
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            panic!("hanging reconciliation should time out while fetching folders")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            panic!("hanging reconciliation should not fetch messages")
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            panic!("hanging reconciliation should not fetch older messages")
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            panic!("hanging reconciliation should not send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            panic!("hanging reconciliation should not edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            panic!("hanging reconciliation should not reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            panic!("hanging reconciliation should not delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            panic!("hanging reconciliation should not subscribe to updates")
        }
    }

    impl TelegramClient for SlowFirstLatestMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            panic!("slow-first client should not fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            panic!("slow-first client should not fetch chats")
        }

        async fn get_messages(&self, chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            if chat_id == 1 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let mut loaded = message(chat_id as i32);
            loaded.chat_id = chat_id;
            Ok(vec![loaded])
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            panic!("slow-first client should not fetch older messages")
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            panic!("slow-first client should not send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            panic!("slow-first client should not edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            panic!("slow-first client should not reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            panic!("slow-first client should not delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            panic!("slow-first client should not subscribe to updates")
        }
    }

    impl TelegramClient for SlowFirstOlderMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            panic!("slow-first-older client should not fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            panic!("slow-first-older client should not fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            panic!("slow-first-older client should not fetch latest messages")
        }

        async fn get_messages_before(
            &self,
            chat_id: i64,
            before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            if before_message_id == 10 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let mut loaded = message(before_message_id.saturating_sub(1));
            loaded.chat_id = chat_id;
            Ok(vec![loaded])
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            panic!("slow-first-older client should not send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            panic!("slow-first-older client should not edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            panic!("slow-first-older client should not reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            panic!("slow-first-older client should not delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            panic!("slow-first-older client should not subscribe to updates")
        }
    }

    impl TelegramClient for RecordingMarkReadClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            panic!("recording mark-read client should not fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            panic!("recording mark-read client should not fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            panic!("recording mark-read client should not fetch latest messages")
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            panic!("recording mark-read client should not fetch older messages")
        }

        async fn mark_chat_read(&self, chat_id: i64) -> Result<()> {
            self.marked_chat_ids
                .lock()
                .expect("marked chat ids lock should not be poisoned")
                .push(chat_id);
            Ok(())
        }

        async fn mark_thread_read(
            &self,
            chat_id: i64,
            topic_id: i32,
            max_message_id: i32,
        ) -> Result<()> {
            self.marked_threads
                .lock()
                .expect("marked threads lock should not be poisoned")
                .push((chat_id, topic_id, max_message_id));
            Ok(())
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            panic!("recording mark-read client should not send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            panic!("recording mark-read client should not edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            panic!("recording mark-read client should not reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            panic!("recording mark-read client should not delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            panic!("recording mark-read client should not subscribe to updates")
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn parse_test_args<I, S>(args: I) -> super::Cli
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        parse_args_from(args).expect("CLI test arguments should parse")
    }

    fn test_path_str(path: &Path) -> &str {
        path.to_str().expect("test path should be unicode")
    }

    fn write_test_file(path: &Path, contents: impl AsRef<[u8]>) {
        std::fs::write(path, contents).expect("test file should be writable")
    }

    fn accepted_prompt_value(result: color_eyre::Result<String>) -> String {
        result.expect("test prompt value should be accepted")
    }

    fn successful_test_setup<T>(result: color_eyre::Result<T>) -> T {
        result.expect("test setup should succeed")
    }

    #[test]
    fn loading_progress_does_not_set_status_banner() {
        let mut app = App::new();
        let mut progress = UiProgress::Silent;

        progress
            .show(&mut app, LOADING_CHAT_MESSAGES_STATUS)
            .expect("loading progress should render without error");

        assert!(app.state.status_message.is_none());

        progress
            .show(&mut app, SENDING_MESSAGE_STATUS)
            .expect("non-loading progress should render without error");

        assert_eq!(
            app.state.status_message.as_deref(),
            Some(SENDING_MESSAGE_STATUS)
        );
    }

    #[test]
    fn app_preferences_are_saved_when_help_or_split_changes() {
        let path = unique_temp_session_path().with_extension("state.toml");
        let mut app = App::new();
        app.preferences_path = Some(path.clone());
        let before = crate::preferences::AppPreferences::from_state(&app.state);

        app.state.toggle_help_bar();
        app.state.adjust_split_right();
        save_app_preferences_if_changed(&mut app, before);

        let saved = crate::preferences::AppPreferences::load(&path)
            .expect("saved app preferences should reload");
        assert!(!saved.ui.show_help_bar);
        assert_eq!(saved.ui.split_ratio, app.state.split_ratio);

        app.state.screen_area = Rect::new(0, 0, 100, 24);
        app.state.begin_split_drag(30);
        app.state.drag_split_to(75);
        app.state.end_split_drag();
        save_app_preferences(&mut app);
        let saved = crate::preferences::AppPreferences::load(&path)
            .expect("dragged split preference should reload");
        assert_eq!(saved.ui.split_ratio, 0.75);

        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn abort_running_task_ignores_finished_handles() {
        let mut finished = Some(tokio::spawn(async {}));
        tokio::task::yield_now().await;

        assert!(!abort_running_task(&mut finished, "test_task_abort", 1));
        assert!(finished.is_none());

        let mut running = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));
        assert!(abort_running_task(&mut running, "test_task_abort", 2));
        assert!(running.is_none());
    }

    #[tokio::test]
    async fn accepted_chat_message_load_marks_read_only_when_unread() {
        let marked_chat_ids = Arc::new(Mutex::new(Vec::new()));
        let marked_threads = Arc::new(Mutex::new(Vec::new()));
        let mark_read_loader = MarkChatReadLoader::new(RecordingMarkReadClient {
            marked_chat_ids: marked_chat_ids.clone(),
            marked_threads: marked_threads.clone(),
        });
        let mut app = App::new();
        app.state.chats = vec![chat(1)];

        let mut read_message = message(1);
        read_message.chat_id = 1;
        apply_chat_message_load_result(
            &mut app,
            1,
            ChatMessageLoadResult {
                request_id: 1,
                chat_id: 1,
                topic_id: None,
                result: Ok(ChatMessageLoad {
                    messages: vec![read_message],
                    thread_topics: Some(Vec::new()),
                }),
            },
            &mark_read_loader,
        );
        tokio::task::yield_now().await;
        assert!(
            marked_chat_ids
                .lock()
                .expect("marked chat ids lock should not be poisoned")
                .is_empty()
        );

        app.state.chats[0].unread_count = 5;
        let mut unread_message = message(2);
        unread_message.chat_id = 1;
        apply_chat_message_load_result(
            &mut app,
            2,
            ChatMessageLoadResult {
                request_id: 2,
                chat_id: 1,
                topic_id: None,
                result: Ok(ChatMessageLoad {
                    messages: vec![unread_message],
                    thread_topics: Some(Vec::new()),
                }),
            },
            &mark_read_loader,
        );
        tokio::task::yield_now().await;
        assert_eq!(
            marked_chat_ids
                .lock()
                .expect("marked chat ids lock should not be poisoned")
                .as_slice(),
            &[1]
        );
        assert!(
            marked_threads
                .lock()
                .expect("marked threads lock should not be poisoned")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn accepted_thread_topic_message_load_marks_only_thread_read() {
        let marked_chat_ids = Arc::new(Mutex::new(Vec::new()));
        let marked_threads = Arc::new(Mutex::new(Vec::new()));
        let mark_read_loader = MarkChatReadLoader::new(RecordingMarkReadClient {
            marked_chat_ids: marked_chat_ids.clone(),
            marked_threads: marked_threads.clone(),
        });
        let mut app = App::new();
        app.state.chats = vec![chat(1)];
        app.state.chats[0].unread_count = 5;
        app.state
            .apply_loaded_selected_chat_thread_topics(vec![ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 102,
                unread_count: 2,
                is_closed: false,
                is_pinned: false,
            }]);

        let mut first_message = message(11);
        first_message.chat_id = 1;
        first_message.thread_topic_id = Some(102);
        let mut latest_message = message(12);
        latest_message.chat_id = 1;
        latest_message.thread_topic_id = Some(102);
        apply_chat_message_load_result(
            &mut app,
            3,
            ChatMessageLoadResult {
                request_id: 3,
                chat_id: 1,
                topic_id: Some(102),
                result: Ok(ChatMessageLoad {
                    messages: vec![first_message, latest_message],
                    thread_topics: None,
                }),
            },
            &mark_read_loader,
        );
        tokio::task::yield_now().await;

        assert!(
            marked_chat_ids
                .lock()
                .expect("marked chat ids lock should not be poisoned")
                .is_empty()
        );
        assert_eq!(
            marked_threads
                .lock()
                .expect("marked threads lock should not be poisoned")
                .as_slice(),
            &[(1, 102, 12)]
        );
    }

    #[tokio::test]
    async fn live_incoming_selected_thread_message_marks_thread_read() {
        let marked_chat_ids = Arc::new(Mutex::new(Vec::new()));
        let marked_threads = Arc::new(Mutex::new(Vec::new()));
        let mark_read_loader = MarkChatReadLoader::new(RecordingMarkReadClient {
            marked_chat_ids: marked_chat_ids.clone(),
            marked_threads: marked_threads.clone(),
        });
        let mut app = App::new();
        app.state.chats = vec![chat(1)];
        app.state
            .apply_loaded_selected_chat_thread_topics(vec![ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 102,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            }]);

        let mut incoming = message(55);
        incoming.chat_id = 1;
        incoming.thread_topic_id = Some(102);
        apply_update_with_read_ack(
            &mut app,
            Update::NewMessage(incoming.clone()),
            &mark_read_loader,
        );
        tokio::task::yield_now().await;

        assert_eq!(
            app.state.messages.last().map(|message| message.id),
            Some(55)
        );
        assert!(
            marked_chat_ids
                .lock()
                .expect("marked chat ids lock should not be poisoned")
                .is_empty()
        );
        assert_eq!(
            marked_threads
                .lock()
                .expect("marked threads lock should not be poisoned")
                .as_slice(),
            &[(1, 102, 55)]
        );
    }

    #[tokio::test]
    async fn live_incoming_other_thread_message_does_not_mark_selected_thread_read() {
        let marked_chat_ids = Arc::new(Mutex::new(Vec::new()));
        let marked_threads = Arc::new(Mutex::new(Vec::new()));
        let mark_read_loader = MarkChatReadLoader::new(RecordingMarkReadClient {
            marked_chat_ids,
            marked_threads: marked_threads.clone(),
        });
        let mut app = App::new();
        app.state.chats = vec![chat(1)];
        app.state.apply_loaded_selected_chat_thread_topics(vec![
            ThreadTopic {
                id: 101,
                title: "General".to_string(),
                top_message_id: 101,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
            ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 102,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
        ]);
        app.state.select_thread_topic_at(1);

        let mut incoming = message(56);
        incoming.chat_id = 1;
        incoming.thread_topic_id = Some(101);
        apply_update_with_read_ack(&mut app, Update::NewMessage(incoming), &mark_read_loader);
        tokio::task::yield_now().await;

        assert!(app.state.messages.is_empty());
        assert!(
            marked_threads
                .lock()
                .expect("marked threads lock should not be poisoned")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stale_async_chat_message_loads_are_ignored() {
        let mut app = App::new();
        app.state.chats = vec![chat(1), chat(2)];
        app.state.selected_chat_index = 1;
        let mark_read_loader = MarkChatReadLoader::new(MockTelegramClient::new());

        let mut stale_message = message(1);
        stale_message.chat_id = 2;
        apply_chat_message_load_result(
            &mut app,
            2,
            ChatMessageLoadResult {
                request_id: 1,
                chat_id: 2,
                topic_id: None,
                result: Ok(ChatMessageLoad {
                    messages: vec![stale_message],
                    thread_topics: Some(vec![ThreadTopic {
                        id: 102,
                        title: "Stale".to_string(),
                        top_message_id: 102,
                        unread_count: 0,
                        is_closed: false,
                        is_pinned: false,
                    }]),
                }),
            },
            &mark_read_loader,
        );
        assert!(app.state.messages.is_empty());

        let mut current_message = message(2);
        current_message.chat_id = 2;
        apply_chat_message_load_result(
            &mut app,
            2,
            ChatMessageLoadResult {
                request_id: 2,
                chat_id: 2,
                topic_id: None,
                result: Ok(ChatMessageLoad {
                    messages: vec![current_message],
                    thread_topics: Some(vec![ThreadTopic {
                        id: 101,
                        title: "Current".to_string(),
                        top_message_id: 101,
                        unread_count: 1,
                        is_closed: false,
                        is_pinned: true,
                    }]),
                }),
            },
            &mark_read_loader,
        );
        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].id, 2);
        assert_eq!(app.state.thread_topics.len(), 1);
        assert_eq!(app.state.thread_topics[0].title, "Current");
    }

    #[tokio::test]
    async fn stale_async_thread_topic_message_loads_are_ignored() {
        let mut app = App::new();
        app.state.chats = vec![chat(1)];
        app.state.apply_loaded_selected_chat_thread_topics(vec![
            ThreadTopic {
                id: 101,
                title: "General".to_string(),
                top_message_id: 101,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
            ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 102,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
        ]);
        app.state.select_thread_topic_at(1);
        let marked_chat_ids = Arc::new(Mutex::new(Vec::new()));
        let marked_threads = Arc::new(Mutex::new(Vec::new()));
        let mark_read_loader = MarkChatReadLoader::new(RecordingMarkReadClient {
            marked_chat_ids,
            marked_threads: marked_threads.clone(),
        });

        let mut stale_topic_message = message(10);
        stale_topic_message.chat_id = 1;
        stale_topic_message.thread_topic_id = Some(101);
        apply_chat_message_load_result(
            &mut app,
            7,
            ChatMessageLoadResult {
                request_id: 7,
                chat_id: 1,
                topic_id: Some(101),
                result: Ok(ChatMessageLoad {
                    messages: vec![stale_topic_message],
                    thread_topics: None,
                }),
            },
            &mark_read_loader,
        );

        tokio::task::yield_now().await;
        assert!(app.state.messages.is_empty());
        assert_eq!(app.state.selected_thread_topic().unwrap().id, 102);
        assert!(
            marked_threads
                .lock()
                .expect("marked threads lock should not be poisoned")
                .is_empty()
        );
    }

    #[test]
    fn async_subscribe_updates_result_installs_receiver() {
        let mut app = App::new();
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (mut loop_state, _) = EventLoopState::new();

        apply_subscribe_updates_result(
            &mut app,
            SubscribeUpdatesResult {
                request_id: 1,
                result: Ok(rx),
            },
            1,
            &mut loop_state,
        );

        assert!(loop_state.update_rx.is_some());
        assert!(app.state.error_message.is_none());
    }

    #[tokio::test]
    async fn async_subscribe_updates_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = SubscribeUpdatesLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_subscribe_updates();

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background subscribe should respond")
            .expect("background subscribe channel should stay open");
        let update_rx = result.result.expect("mock subscribe should succeed");
        drop(update_rx);
    }

    #[tokio::test]
    async fn async_initial_state_result_applies_startup_state_and_clears_status() {
        let mut app = App::new();
        app.state.set_status(LOADING_TELEGRAM_STATUS);
        let mut loaded_message = message(1);
        loaded_message.chat_id = 1;
        let mark_read_loader = MarkChatReadLoader::new(MockTelegramClient::new());

        apply_initial_state_load_result(
            &mut app,
            InitialStateLoadResult {
                result: Ok(crate::actions::InitialStateLoad {
                    folders: vec![all_folder(0)],
                    chats: vec![chat(1)],
                    messages: Ok(vec![loaded_message]),
                    thread_topics: Vec::new(),
                }),
            },
            &mark_read_loader,
        );

        assert_eq!(app.state.folders.len(), 1);
        assert_eq!(app.state.chats.len(), 1);
        assert_eq!(app.state.messages.len(), 1);
        assert!(app.state.status_message.is_none());
    }

    #[tokio::test]
    async fn async_initial_state_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let loader = InitialStateLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_initial_state();

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background initial load should respond")
            .expect("background initial load channel should stay open");
        let load = result.result.expect("mock initial load should succeed");
        assert_eq!(load.folders.len(), 3);
        assert_eq!(load.chats.len(), 4);
        assert_eq!(
            load.messages
                .expect("mock initial messages should succeed")
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn async_chat_message_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = ChatMessageLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_latest_chat_messages(3);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background message load should respond")
            .expect("background message load channel should stay open");
        assert_eq!(result.request_id, 1);
        assert_eq!(result.chat_id, 3);
        let load = result.result.expect("mock load should succeed");
        assert_eq!(load.messages.len(), 3);
        let thread_topics = load
            .thread_topics
            .expect("latest load should include topics");
        assert_eq!(thread_topics.len(), 2);
        assert_eq!(thread_topics[0].title, "General");
    }

    #[tokio::test]
    async fn async_chat_message_loader_sends_thread_topic_messages_without_replacing_topics() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = ChatMessageLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_thread_topic_messages(3, 102);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background thread message load should respond")
            .expect("background thread message channel should stay open");
        assert_eq!(result.request_id, 1);
        assert_eq!(result.chat_id, 3);
        let load = result.result.expect("mock thread load should succeed");
        assert_eq!(load.messages.len(), 1);
        assert_eq!(load.messages[0].id, 102);
        assert!(load.thread_topics.is_none());
    }

    #[tokio::test]
    async fn async_noop_chat_open_does_not_spawn_message_load() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = ChatMessageLoader::new(MockTelegramClient::new(), tx);
        let mut loader_ref = Some(&mut loader);
        let mut client = MockTelegramClient::new();
        let mut progress = UiProgress::Silent;
        let mut app = App::new();
        app.state.chats = vec![chat(1), chat(2)];
        app.state.selected_chat_index = 0;
        app.state.messages = vec![message(1)];

        open_chat_at_with_optional_async_loader(
            &mut app,
            &mut client,
            &mut progress,
            &mut loader_ref,
            0,
        )
        .await
        .expect("no-op chat open should succeed");

        assert_eq!(app.state.selected_chat_index, 0);
        assert_eq!(app.state.messages.len(), 1);
        assert!(app.state.status_message.is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(80), rx.recv())
                .await
                .is_err(),
            "no-op chat open should not spawn a message loader"
        );
    }

    #[tokio::test]
    async fn keyboard_mouse_and_context_search_commits_all_clear_search() {
        let mut keyboard_app = App::new();
        keyboard_app.state.chats = vec![chat(1), chat(2)];
        keyboard_app.state.focused_panel = FocusedPanel::Chats;
        keyboard_app.state.begin_chat_search();
        keyboard_app.state.push_chat_search_char('2');
        let mut keyboard_client = MockTelegramClient::new();

        handle_key_event(
            &mut keyboard_app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut keyboard_client,
        )
        .await
        .expect("keyboard search commit should succeed");
        assert_eq!(keyboard_app.state.selected_chat_index, 1);
        assert!(!keyboard_app.state.chat_search_active());

        let mut mouse_app = App::new();
        mouse_app.state.chats = vec![chat(1), chat(2)];
        mouse_app.state.chats_area = Rect::new(0, 5, 30, 8);
        mouse_app.state.focused_panel = FocusedPanel::Chats;
        mouse_app.state.begin_chat_search();
        mouse_app.state.push_chat_search_char('2');
        let mut mouse_client = MockTelegramClient::new();

        handle_mouse_event(
            &mut mouse_app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 6,
                modifiers: KeyModifiers::NONE,
            },
            &mut mouse_client,
        )
        .await
        .expect("mouse search commit should succeed");
        assert_eq!(mouse_app.state.selected_chat_index, 1);
        assert!(!mouse_app.state.chat_search_active());

        let mut menu_app = App::new();
        menu_app.state.chats = vec![chat(1), chat(2)];
        menu_app.state.focused_panel = FocusedPanel::Chats;
        menu_app.state.begin_chat_search();
        assert!(
            menu_app
                .state
                .open_context_menu(ContextMenuTarget::Chat { chat_id: 2 }, 1, 1,)
        );
        let mut menu_client = MockTelegramClient::new();

        handle_key_event(
            &mut menu_app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut menu_client,
        )
        .await
        .expect("context menu search commit should succeed");
        assert_eq!(menu_app.state.selected_chat_index, 1);
        assert!(!menu_app.state.chat_search_active());
    }

    #[tokio::test]
    async fn rapid_typing_sends_once_per_chat_during_cooldown() {
        let mut app = App::new();
        app.state.chats = vec![chat(1), chat(2)];
        app.state.focused_panel = FocusedPanel::Input;
        let mut client = MockTelegramClient::new();
        let observer = client.clone();
        let mut progress = UiProgress::Silent;

        for character in "A multi-sentence message. Still typing.".chars() {
            handle_input_focused(
                &mut app,
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
                &mut client,
                &mut progress,
                None,
                None,
                None,
            )
            .await
            .expect("typing should succeed");
        }
        tokio::task::yield_now().await;
        assert_eq!(observer.typing_action_count(), 1);

        app.state.selected_chat_index = 1;
        handle_input_focused(
            &mut app,
            KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
            &mut client,
            &mut progress,
            None,
            None,
            None,
        )
        .await
        .expect("typing after switching chats should succeed");
        tokio::task::yield_now().await;
        assert_eq!(observer.typing_action_count(), 2);
    }

    #[tokio::test]
    async fn media_preview_loader_deduplicates_until_selection_changes() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = MediaPreviewLoader::new(MockTelegramClient::new(), tx);

        loader.request(Some((1, 10)));
        loader.request(Some((1, 10)));
        let first = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("preview load should respond")
            .expect("preview channel should stay open");
        assert_eq!(first.request_id, 1);
        assert_eq!((first.chat_id, first.message_id), (1, 10));
        assert!(first.result.expect("mock preview should load").is_some());
        assert!(
            tokio::time::timeout(Duration::from_millis(40), rx.recv())
                .await
                .is_err(),
            "unchanged selection should not request another preview"
        );

        loader.request(None);
        loader.request(Some((1, 10)));
        let second = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("reselected preview should respond")
            .expect("preview channel should stay open");
        assert_eq!(second.request_id, 2);
    }

    #[test]
    fn media_preview_result_rejects_cross_chat_collision_and_keeps_errors_silent() {
        let mut app = App::new();
        app.state.chats = vec![chat(2)];
        app.state.messages = vec![message(7)];
        app.state.messages[0].chat_id = 2;
        app.state.messages[0].content = "unchanged".to_string();
        app.state.messages[0].media = Some(MessageMedia::photo());
        app.state.conversation_load_status = ConversationLoadStatus::Failed;

        apply_media_preview_result(
            &mut app,
            1,
            MediaPreviewResult {
                request_id: 1,
                chat_id: 1,
                message_id: 7,
                result: Ok(Some("/tmp/chat-a.png".into())),
            },
        );
        assert!(
            app.state.messages[0]
                .media
                .as_ref()
                .and_then(|media| media.local_path.as_ref())
                .is_none()
        );

        apply_media_preview_result(
            &mut app,
            2,
            MediaPreviewResult {
                request_id: 2,
                chat_id: 2,
                message_id: 7,
                result: Err("preview unavailable".to_string()),
            },
        );
        assert_eq!(app.state.messages[0].content, "unchanged");
        assert_eq!(
            app.state.conversation_load_status,
            ConversationLoadStatus::Failed
        );
        assert!(app.state.error_message.is_none());

        apply_media_preview_result(
            &mut app,
            3,
            MediaPreviewResult {
                request_id: 3,
                chat_id: 2,
                message_id: 7,
                result: Ok(Some("/tmp/chat-b.png".into())),
            },
        );
        assert_eq!(
            app.state.messages[0]
                .media
                .as_ref()
                .and_then(|media| media.local_image_path()),
            Some(Path::new("/tmp/chat-b.png"))
        );
    }

    #[tokio::test]
    async fn async_chat_message_loader_aborts_superseded_load() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = ChatMessageLoader::new(SlowFirstLatestMessagesClient, tx);

        loader.spawn_latest_chat_messages(1);
        loader.spawn_latest_chat_messages(2);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("newest background message load should respond")
            .expect("background message load channel should stay open");
        assert_eq!(result.request_id, 2);
        assert_eq!(result.chat_id, 2);
        assert_eq!(
            result
                .result
                .expect("newest load should succeed")
                .messages
                .len(),
            1
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(80), rx.recv())
                .await
                .is_err(),
            "aborted stale chat-message load should not send a result"
        );
    }

    #[test]
    fn stale_async_older_message_loads_are_ignored() {
        let mut app = App::new();
        app.state.chats = vec![chat(1), chat(2)];
        app.state.selected_chat_index = 1;
        let mut anchor = message(10);
        anchor.chat_id = 2;
        app.state.messages = vec![anchor];

        let mut stale_older = message(9);
        stale_older.chat_id = 2;
        apply_older_message_load_result(
            &mut app,
            2,
            OlderMessageLoadResult {
                request_id: 1,
                chat_id: 2,
                topic_id: None,
                before_message_id: 10,
                navigation: OlderMessageNavigation::OneLine,
                result: Ok(vec![stale_older]),
            },
        );
        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.selected_message_index, 0);

        let mut current_older = message(8);
        current_older.chat_id = 2;
        apply_older_message_load_result(
            &mut app,
            2,
            OlderMessageLoadResult {
                request_id: 2,
                chat_id: 2,
                topic_id: None,
                before_message_id: 10,
                navigation: OlderMessageNavigation::OneLine,
                result: Ok(vec![current_older]),
            },
        );
        assert_eq!(app.state.messages.len(), 2);
        assert_eq!(app.state.messages[0].id, 8);
        assert_eq!(app.state.selected_message_index, 0);
    }

    #[tokio::test]
    async fn async_older_message_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = OlderMessageLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_older_messages(1, None, 3, OlderMessageNavigation::OneLine);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background older-message load should respond")
            .expect("background older-message load channel should stay open");
        assert_eq!(result.request_id, 1);
        assert_eq!(result.chat_id, 1);
        assert_eq!(result.before_message_id, 3);
        assert_eq!(result.navigation, OlderMessageNavigation::OneLine);
        assert_eq!(
            result.result.expect("mock older load should succeed").len(),
            2
        );
    }

    #[tokio::test]
    async fn async_older_message_loader_fetches_topic_history_when_topic_selected() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = OlderMessageLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_older_messages(3, Some(101), 103, OlderMessageNavigation::OneLine);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background older topic load should respond")
            .expect("background older topic load channel should stay open");
        assert_eq!(result.chat_id, 3);
        assert_eq!(result.topic_id, Some(101));
        assert_eq!(result.before_message_id, 103);
        let messages = result.result.expect("mock older topic load should succeed");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 101);
    }

    #[tokio::test]
    async fn async_older_message_loader_aborts_superseded_load() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = OlderMessageLoader::new(SlowFirstOlderMessagesClient, tx);

        loader.spawn_older_messages(1, None, 10, OlderMessageNavigation::OneLine);
        loader.spawn_older_messages(1, None, 9, OlderMessageNavigation::Page);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("newest background older-message load should respond")
            .expect("background older-message load channel should stay open");
        assert_eq!(result.request_id, 2);
        assert_eq!(result.chat_id, 1);
        assert_eq!(result.before_message_id, 9);
        assert_eq!(result.navigation, OlderMessageNavigation::Page);
        assert_eq!(
            result
                .result
                .expect("newest older load should succeed")
                .len(),
            1
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(80), rx.recv())
                .await
                .is_err(),
            "aborted stale older-message load should not send a result"
        );
    }

    #[tokio::test]
    async fn stale_async_folder_chat_loads_are_ignored() {
        let mut app = App::new();
        app.state.folders = vec![all_folder(0), folder(2, "Personal")];
        app.state.selected_folder_index = 1;
        let mark_read_loader = MarkChatReadLoader::new(MockTelegramClient::new());

        apply_folder_chat_load_result(
            &mut app,
            2,
            FolderChatLoadResult {
                request_id: 1,
                folder_index: 1,
                folder_id: Some(2),
                result: Ok(crate::actions::FolderChatLoad {
                    chats: vec![chat(99)],
                    messages: Ok(Vec::new()),
                    thread_topics: Vec::new(),
                }),
            },
            &mark_read_loader,
        );
        assert!(app.state.chats.is_empty());

        let mut loaded_message = message(1);
        loaded_message.chat_id = 2;
        apply_folder_chat_load_result(
            &mut app,
            2,
            FolderChatLoadResult {
                request_id: 2,
                folder_index: 1,
                folder_id: Some(2),
                result: Ok(crate::actions::FolderChatLoad {
                    chats: vec![chat(2)],
                    messages: Ok(vec![loaded_message]),
                    thread_topics: Vec::new(),
                }),
            },
            &mark_read_loader,
        );
        assert_eq!(app.state.chats.len(), 1);
        assert_eq!(app.state.chats[0].id, 2);
        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].chat_id, 2);
    }

    #[tokio::test]
    async fn async_folder_chat_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = FolderChatLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_folder_chats(1, Some(2));

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background folder-chat load should respond")
            .expect("background folder-chat load channel should stay open");
        assert_eq!(result.request_id, 1);
        assert_eq!(result.folder_index, 1);
        assert_eq!(result.folder_id, Some(2));
        let load = result.result.expect("mock folder load should succeed");
        assert_eq!(load.chats.len(), 2);
        assert_eq!(
            load.messages
                .expect("mock message load should succeed")
                .len(),
            3
        );
    }

    #[test]
    fn async_send_message_result_replaces_pending_row() {
        let mut app = App::new();
        app.state.chats = vec![chat(1)];
        let pending =
            crate::actions::begin_send_message(&mut app.state, 1, None, "hello".to_string());
        let mut sent = message(10);
        sent.chat_id = 1;
        sent.content = "hello".to_string();

        apply_send_message_result(
            &mut app,
            SendMessageResult {
                temp_id: pending.temp_id,
                chat_id: 1,
                result: Ok(sent),
            },
        );

        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].id, 10);
        assert_eq!(app.state.messages[0].content, "hello");
    }

    #[tokio::test]
    async fn async_send_message_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let loader = SendMessageLoader::new(MockTelegramClient::new(), tx);
        let pending = crate::actions::PendingSend {
            temp_id: -1,
            chat_id: 1,
            thread_top_message_id: None,
            content: "hello".to_string(),
        };

        loader.spawn_send_message(pending);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background send should respond")
            .expect("background send channel should stay open");
        assert_eq!(result.temp_id, -1);
        assert_eq!(result.chat_id, 1);
        let sent = result.result.expect("mock send should succeed");
        assert_eq!(sent.chat_id, 1);
        assert_eq!(sent.content, "hello");
    }

    #[test]
    fn async_edit_message_result_updates_confirmed_row() {
        let mut app = App::new();
        let mut original = message(7);
        original.chat_id = 1;
        original.content = "old".to_string();
        app.state.chats = vec![chat(1)];
        app.state.messages = vec![original];

        apply_edit_message_result(
            &mut app,
            EditMessageResult {
                chat_id: 1,
                message_id: 7,
                content: "updated".to_string(),
                result: Ok(()),
            },
        );

        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].content, "updated");
        assert!(app.state.messages[0].is_edited);
    }

    #[tokio::test]
    async fn async_edit_message_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let loader = EditMessageLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_edit_message(1, 7, "updated".to_string());

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background edit should respond")
            .expect("background edit channel should stay open");
        assert_eq!(result.chat_id, 1);
        assert_eq!(result.message_id, 7);
        assert_eq!(result.content, "updated");
        result.result.expect("mock edit should succeed");
    }

    #[test]
    fn async_reply_message_result_appends_reply_row() {
        let mut app = App::new();
        app.state.chats = vec![chat(1)];
        let mut reply = message(11);
        reply.chat_id = 1;
        reply.content = "reply".to_string();
        reply.is_own = true;

        apply_reply_message_result(
            &mut app,
            ReplyMessageResult {
                chat_id: 1,
                message_id: 7,
                result: Ok(reply),
            },
        );

        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].id, 11);
        assert_eq!(app.state.messages[0].content, "reply");
    }

    #[tokio::test]
    async fn async_reply_message_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let loader = ReplyMessageLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_reply_message(1, None, 7, "reply".to_string());

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background reply should respond")
            .expect("background reply channel should stay open");
        assert_eq!(result.chat_id, 1);
        assert_eq!(result.message_id, 7);
        let sent = result.result.expect("mock reply should succeed");
        assert_eq!(sent.chat_id, 1);
        assert_eq!(sent.content, "reply");
        assert_eq!(sent.reply_to_content.as_deref(), Some("Replied message"));
    }

    #[test]
    fn async_delete_message_result_removes_confirmed_row() {
        let mut app = App::new();
        let mut message = message(7);
        message.chat_id = 1;
        app.state.chats = vec![chat(1)];
        app.state.messages = vec![message];
        app.state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 1,
            message_id: 7,
        });
        let confirmation = crate::actions::begin_confirm_delete(&mut app.state)
            .expect("delete confirmation should be present");
        assert!(app.state.delete_confirmation().is_none());

        apply_delete_message_result(
            &mut app,
            DeleteMessageResult {
                confirmation,
                result: Ok(()),
            },
        );

        assert!(app.state.messages.is_empty());
    }

    #[tokio::test]
    async fn async_delete_message_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let loader = DeleteMessageLoader::new(MockTelegramClient::new(), tx);
        let confirmation = DeleteConfirmation {
            chat_id: 1,
            message_id: 7,
        };

        loader.spawn_delete_message(confirmation);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background delete should respond")
            .expect("background delete channel should stay open");
        assert_eq!(result.confirmation, confirmation);
        result.result.expect("mock delete should succeed");
    }

    #[tokio::test]
    async fn async_reply_message_loader_sends_thread_reply_when_topic_selected() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let loader = ReplyMessageLoader::new(MockTelegramClient::new(), tx);

        loader.spawn_reply_message(3, Some(102), 7, "topic reply".to_string());

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background topic reply should respond")
            .expect("background topic reply channel should stay open");
        assert_eq!(result.chat_id, 3);
        assert_eq!(result.message_id, 7);
        let sent = result.result.expect("mock topic reply should succeed");
        assert_eq!(sent.chat_id, 3);
        assert_eq!(sent.content, "topic reply");
        assert_eq!(sent.reply_to_content.as_deref(), Some("topic 102 reply 7"));
    }

    #[test]
    fn message_submit_progress_statuses_are_shared_constants() {
        assert_eq!(
            message_submit_action_status(&crate::state::MessageSubmitAction::Edit {
                chat_id: 1,
                message_id: 2,
                content: "updated".to_string(),
            }),
            SAVING_EDIT_STATUS
        );
        assert_eq!(
            message_submit_action_status(&crate::state::MessageSubmitAction::Reply {
                chat_id: 1,
                thread_top_message_id: None,
                message_id: 2,
                content: "reply".to_string(),
            }),
            SENDING_REPLY_STATUS
        );
        assert_eq!(
            message_submit_action_status(&crate::state::MessageSubmitAction::Send {
                chat_id: 1,
                thread_top_message_id: None,
                content: "hello".to_string(),
            }),
            SENDING_MESSAGE_STATUS
        );
    }

    #[test]
    fn smoke_success_message_uses_shared_prefix() {
        assert_eq!(
            smoke_ok_message(3, 4, 5),
            format!(
                "{SMOKE_OK_PREFIX}: rendered 3 folders, 4 chats, 5 messages and exercised keyboard/mouse interactions"
            )
        );
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

    #[tokio::test]
    async fn mouse_scroll_up_at_loaded_top_moves_locally_without_loading_history() {
        let mut app = App::new();
        let mut client = MockTelegramClient::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages_area = older_scroll_message_area();
        app.state.messages = vec![message(1), message(2)];
        app.state.selected_message_index = 0;
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };

        handle_mouse_event(&mut app, scroll_up, &mut client)
            .await
            .expect("mouse scroll should remain local");

        assert_eq!(app.state.focused_panel, FocusedPanel::Messages);
        assert_eq!(app.state.selected_message_index, 0);
        assert_eq!(app.state.messages.len(), 2);
        assert!(app.state.status_message.is_none());
        assert!(app.state.error_message.is_none());
    }

    #[tokio::test]
    async fn mouse_click_on_thread_topic_tab_loads_topic_messages() {
        let mut app = App::new();
        let mut client = MockTelegramClient::new();
        app.state.chats = vec![Chat {
            id: 3,
            name: "Work Team".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: true,
            folder_id: None,
        }];
        app.state.thread_topics_area = Rect::new(30, 5, 60, 3);
        app.state.thread_topics = vec![
            ThreadTopic {
                id: 101,
                title: "General".to_string(),
                top_message_id: 1001,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
            ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 1002,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
        ];
        app.state.messages = vec![message(101)];

        let topic_click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 45,
            row: 6,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse_event(&mut app, topic_click, &mut client)
            .await
            .expect("topic tab mouse click should load topic messages");

        assert_eq!(app.state.focused_panel, FocusedPanel::Messages);
        assert_eq!(app.state.selected_thread_topic_index, 1);
        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].id, 102);
        assert!(app.state.messages[0].content.contains("Deployments topic"));
    }

    #[tokio::test]
    async fn context_menu_keyboard_blocks_underlying_quit_and_closes_before_delete_confirmation() {
        let mut app = App::new();
        let mut client = MockTelegramClient::new();
        let mut selected = message(7);
        selected.is_own = true;
        selected.can_edit = true;
        selected.can_delete = true;
        app.state.chats = vec![chat(10)];
        app.state.messages = vec![selected];
        assert!(app.state.open_context_menu(
            ContextMenuTarget::Message {
                chat_id: 10,
                message_id: 7,
            },
            1,
            1,
        ));

        let mut progress = UiProgress::Silent;
        handle_key_event_with_progress(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut client,
            &mut progress,
            HandlerLoaders::none(),
        )
        .await
        .expect("menu should consume unrelated key");
        assert!(!app.should_quit);
        assert!(app.state.context_menu().is_some());

        let last = app
            .state
            .context_menu()
            .expect("menu remains open")
            .actions
            .len()
            - 1;
        app.state
            .context_menu_mut()
            .expect("menu remains mutable")
            .highlighted = last;
        handle_key_event_with_progress(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut client,
            &mut progress,
            HandlerLoaders::none(),
        )
        .await
        .expect("delete menu action should be handled");

        assert!(app.state.context_menu().is_none());
        assert!(app.state.delete_confirmation().is_some_and(|confirmation| {
            confirmation.chat_id == 10 && confirmation.message_id == 7
        }));
    }

    #[tokio::test]
    async fn mouse_events_are_ignored_while_delete_confirmation_is_open() {
        let mut app = App::new();
        let mut client = MockTelegramClient::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages_area = older_scroll_message_area();
        app.state.input_area = Rect::new(0, 20, 40, 3);
        app.state.messages = vec![message(1), message(2)];
        app.state.selected_message_index = 0;
        app.state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 10,
            message_id: 1,
        });

        let scroll_up_at_loaded_top = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse_event(&mut app, scroll_up_at_loaded_top, &mut client)
            .await
            .expect("mouse event should be ignored without error");

        assert_eq!(app.state.focused_panel, FocusedPanel::Messages);
        assert_eq!(app.state.selected_message_index, 0);
        assert!(app.state.status_message.is_none());
        assert!(app.state.error_message.is_none());
        assert!(app.state.delete_confirmation().is_some());

        let input_click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 21,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse_event(&mut app, input_click, &mut client)
            .await
            .expect("mouse click should be ignored without error");

        assert_eq!(app.state.focused_panel, FocusedPanel::Messages);
        assert!(app.state.delete_confirmation().is_some());
    }

    #[test]
    fn empty_args_use_shared_default_config_path() {
        let cli = parse_test_args(std::iter::empty::<&str>());

        assert_eq!(cli.config_path, default_config_path_string());
        assert_eq!(cli.mode, RunMode::RealTelegram);
        assert!(!cli.smoke);
    }

    #[test]
    fn cli_usage_errors_exit_with_standard_misuse_code() {
        assert_eq!(CLI_USAGE_EXIT_CODE, 2);
    }

    #[test]
    fn setup_errors_exit_with_general_failure_code() {
        assert_eq!(SETUP_ERROR_EXIT_CODE, 1);
    }

    #[test]
    fn unknown_argument_points_to_shared_help_command() {
        let err = parse_args_from(["--wat"])
            .expect_err("unknown argument should report the help command");

        assert!(err.to_string().contains(&format!("`{APP_COMMAND} --help`")));
    }

    #[test]
    fn config_path_requires_explicit_path_argument() {
        let err = parse_args_from(["--config", "--smoke"])
            .expect_err("--config should reject another option as its path");

        assert_eq!(err.to_string(), CONFIG_PATH_ARGUMENT_REQUIRED);
    }

    #[test]
    fn log_path_parses_for_runtime_diagnostics() {
        let cli = parse_test_args(["--config", "real.toml", "--log", "dumbgram.log"]);

        assert_eq!(cli.config_path, "real.toml");
        assert_eq!(cli.log_path, Some("dumbgram.log".to_string()));
    }

    #[test]
    fn log_path_requires_explicit_path_argument() {
        let err = parse_args_from(["--log", "--mock"])
            .expect_err("--log should reject another option as its path");

        assert_eq!(err.to_string(), LOG_PATH_ARGUMENT_REQUIRED);
    }

    #[test]
    fn smoke_flag_forces_mock_mode() {
        let cli = parse_test_args(["--smoke"]);

        assert_eq!(cli.mode, RunMode::Mock);
        assert!(cli.smoke);
    }

    #[test]
    fn smoke_flag_overrides_real_config_path_without_real_mode() {
        let cli = parse_test_args(["--config", "real.toml", "--smoke"]);

        assert_eq!(cli.mode, RunMode::Mock);
        assert_eq!(cli.config_path, "real.toml");
        assert!(cli.smoke);
    }

    #[test]
    fn config_path_does_not_imply_mock_without_smoke() {
        let cli = parse_test_args(["--config", "real.toml"]);

        assert_eq!(cli.mode, RunMode::RealTelegram);
        assert_eq!(cli.config_path, "real.toml");
        assert!(!cli.smoke);
    }

    #[test]
    fn check_auth_parses_as_real_opt_in_diagnostic() {
        let cli = parse_test_args(["--check-auth", "--config", "real.toml"]);

        assert!(cli.check_auth);
        assert_eq!(cli.mode, RunMode::RealTelegram);
        assert_eq!(cli.config_path, "real.toml");
    }

    #[test]
    fn check_auth_messages_use_shared_command_and_prefix() {
        assert_eq!(
            check_auth_ok_message("session.dat"),
            format!("{CHECK_AUTH_OK_PREFIX} (session.dat)")
        );
        assert_eq!(
            check_auth_unauthorized_message("config.toml"),
            format!(
                "Telegram session is not authorized. Run `{APP_COMMAND} --config config.toml` to log in."
            )
        );
    }

    #[test]
    fn prompt_line_normalization_trims_regular_input_but_preserves_password_spaces() {
        assert_eq!(trim_prompt_input_line("  +123  \n"), "+123");
        assert_eq!(trim_prompt_input_line("  12345  \r\n"), "12345");
        assert_eq!(
            preserve_prompt_input_line_spaces("  pass phrase  \n"),
            "  pass phrase  "
        );
        assert_eq!(
            preserve_prompt_input_line_spaces("  pass phrase  \r\n"),
            "  pass phrase  "
        );
    }

    #[test]
    fn required_prompt_lines_reject_eof() {
        assert_eq!(
            accepted_prompt_value(require_prompt_line(5, "hello".to_string())),
            "hello"
        );
        assert_eq!(
            require_prompt_line(0, String::new())
                .expect_err("required prompt should reject EOF")
                .to_string(),
            PROMPT_EOF_ERROR
        );
    }

    #[test]
    fn required_prompt_responses_reject_empty_values() {
        assert_eq!(
            accepted_prompt_value(require_prompt_response("value".to_string())),
            "value"
        );
        assert_eq!(
            require_prompt_response(String::new())
                .expect_err("required prompt should reject empty responses")
                .to_string(),
            PROMPT_EMPTY_ERROR
        );
    }

    #[test]
    fn login_messages_use_shared_prompts_and_prefixes() {
        assert_eq!(LOGIN_HEADER, "\n=== Telegram Login Required ===\n");
        assert!(LOGIN_PHONE_PROMPT.contains("Enter phone number"));
        assert_eq!(LOGIN_REQUESTING_CODE_STATUS, "Requesting login code…");
        assert_eq!(LOGIN_CODE_PROMPT, "Enter verification code: ");
        assert_eq!(LOGIN_SIGNING_IN_STATUS, "Signing in…");
        assert_eq!(LOGIN_2FA_ENABLED_STATUS, "2FA enabled.");
        assert_eq!(LOGIN_2FA_PROMPT, "Enter 2FA password: ");
        assert_eq!(
            LOGIN_SESSION_SAVED_STATUS,
            "OK Session saved! Press Enter to start…"
        );
        assert_eq!(LOGIN_START_PROMPT, "");
        assert_eq!(
            login_code_sent_message("+123"),
            format!("{LOGIN_CODE_SENT_PREFIX} +123")
        );
        assert_eq!(
            login_signed_in_message("Alice"),
            format!("{LOGIN_SIGNED_IN_PREFIX} Alice")
        );
        assert_eq!(
            login_2fa_hint_message("word"),
            format!("{LOGIN_2FA_HINT_PREFIX} word")
        );
        assert_eq!(
            login_2fa_signed_in_message("Alice"),
            format!("{LOGIN_2FA_SIGNED_IN_PREFIX} Alice")
        );
        assert_eq!(
            login_failed_message("denied"),
            format!("{LOGIN_FAILED_PREFIX}: denied")
        );
    }

    #[test]
    fn smoke_cannot_be_combined_with_check_config() {
        let err = parse_args_from(["--smoke", "--check-config"])
            .expect_err("--smoke plus --check-config must be rejected as conflicting exit modes");

        assert_eq!(err.to_string(), SMOKE_CHECK_CONFIG_CONFLICT);
    }

    #[test]
    fn smoke_cannot_be_combined_with_check_auth() {
        let err = parse_args_from(["--smoke", "--check-auth"])
            .expect_err("--smoke plus --check-auth must be rejected to keep smoke mock-only");

        assert_eq!(err.to_string(), SMOKE_CHECK_AUTH_CONFLICT);
    }

    #[test]
    fn check_config_cannot_be_combined_with_check_auth() {
        let err = parse_args_from(["--check-config", "--check-auth"])
            .expect_err("config and auth diagnostics must be explicit separate commands");

        assert_eq!(err.to_string(), CHECK_CONFIG_AUTH_CONFLICT);
    }

    #[test]
    fn config_load_errors_include_starter_config_guidance() {
        let missing_config = unique_temp_session_path().with_extension("toml");

        let error = load_checked_config(test_path_str(&missing_config))
            .expect_err("missing config should include starter config guidance");

        let error = error.to_string();
        assert!(error.contains("failed to load"));
        assert!(error.contains(CONFIG_LOAD_HELP));
    }

    #[test]
    fn check_config_session_status_reports_existing_or_pending_session() {
        let missing_session = unique_temp_session_path();

        assert_eq!(
            check_config_session_status(&missing_session),
            CHECK_CONFIG_SESSION_WILL_CREATE_STATUS
        );

        write_test_file(&missing_session, "session");
        assert_eq!(
            check_config_session_status(&missing_session),
            CHECK_CONFIG_SESSION_EXISTS_STATUS
        );
        std::fs::remove_file(missing_session).ok();
    }

    #[test]
    fn check_config_message_uses_shared_session_status() {
        let missing_session = unique_temp_session_path();
        let config = Config {
            telegram: TelegramConfig {
                api_id: 42,
                api_hash: "hash".to_string(),
                session_file: missing_session.to_string_lossy().into_owned(),
            },
        };

        assert_eq!(
            check_config_message("test-config.toml", &config, &missing_session),
            format!(
                "Config OK: test-config.toml (api_id=42, session_file={} [{}])",
                missing_session.display(),
                CHECK_CONFIG_SESSION_WILL_CREATE_STATUS
            )
        );
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

        successful_test_setup(validate_config(&config, "test-config.toml"));
        assert!(!missing_parent.exists());
    }

    #[test]
    fn ensure_session_parent_dir_creates_missing_parent() {
        let missing_parent = unique_temp_session_path();
        let session_path = missing_parent.join("session.dat");

        successful_test_setup(ensure_session_parent_dir(&session_path));

        assert!(missing_parent.is_dir());
        std::fs::remove_dir_all(missing_parent).ok();
    }

    #[test]
    fn telegram_setup_creates_missing_session_parent() {
        let missing_parent = unique_temp_session_path();
        let session_path = missing_parent.join("session.dat");
        let config_path = unique_temp_session_path().with_extension("toml");
        let config = format!(
            "[telegram]\napi_id = 1\napi_hash = \"hash\"\nsession_file = \"{}\"\n",
            session_path.display()
        );
        write_test_file(&config_path, config);

        let (_, loaded_session_path) = successful_test_setup(
            load_checked_config_with_session_parent(test_path_str(&config_path)),
        );

        assert_eq!(loaded_session_path, session_path.to_string_lossy().as_ref());
        assert!(missing_parent.is_dir());
        std::fs::remove_file(config_path).ok();
        std::fs::remove_dir_all(missing_parent).ok();
    }
}
