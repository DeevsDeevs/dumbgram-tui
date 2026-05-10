mod actions;
mod app;
mod app_keys;
mod chat_keys;
mod config;
mod confirm_keys;
mod diagnostics;
mod folder_keys;
mod global_keys;
mod input_keys;
mod links;
mod message_keys;
mod mouse_events;
mod preferences;
mod state;
mod telegram;
mod terminal_images;
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
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, io};
use telegram::types::{Message, Update};
use telegram::{GrammersClient, MockTelegramClient, TelegramClient};

const LOADING_TELEGRAM_STATUS: &str = "Loading Telegram data…";
const LOADING_OLDER_MESSAGES_STATUS: &str = "Loading older messages…";
const LOADING_CHAT_MESSAGES_STATUS: &str = "Loading chat messages…";
const LOADING_FOLDER_CHATS_STATUS: &str = "Loading folder chats…";
const LINK_OPENED_STATUS: &str = "Link opened";
const NO_LINK_IN_SELECTED_MESSAGE_STATUS: &str = "No link in selected message";
const OPEN_LINK_FAILED_PREFIX: &str = "Open link failed";
const DELETING_MESSAGE_STATUS: &str = "Deleting message…";
const SENDING_MESSAGE_STATUS: &str = "Sending message…";
const SAVING_EDIT_STATUS: &str = "Saving edit…";
const SENDING_REPLY_STATUS: &str = "Sending reply…";
const DEFAULT_CONFIG_PATH: &str = "config.toml";
const APP_COMMAND: &str = "dumbgram_tui";
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
            config_path: DEFAULT_CONFIG_PATH.to_string(),
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

fn print_help() {
    println!(
        "Dumbgram TUI\n\n\
Usage:\n  {APP_COMMAND} [OPTIONS]\n\n\
Options:\n  --mock             Run with built-in mock Telegram data for smoke testing\n  --smoke            Load mock data, render off-screen, exercise interactions, and exit\n  --check-config     Validate Telegram config and session path without connecting\n  --check-auth       Connect and verify saved Telegram session without login/TUI\n  -c, --config PATH  Load Telegram config from PATH (default: {DEFAULT_CONFIG_PATH})\n  --log PATH         Append privacy-safe runtime diagnostics to PATH\n  -h, --help         Print this help\n\n\
Examples:\n  {APP_COMMAND} --mock\n  {APP_COMMAND} --mock --smoke\n  {APP_COMMAND} --check-config --config {DEFAULT_CONFIG_PATH}\n  {APP_COMMAND} --check-auth --config {DEFAULT_CONFIG_PATH}\n  {APP_COMMAND} --config {DEFAULT_CONFIG_PATH}"
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

fn load_checked_config_with_session_parent(config_path: &str) -> Result<(config::Config, String)> {
    let config = load_checked_config(config_path)?;
    let session_path = config.telegram.session_path();
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
    let session_path = config.telegram.session_path();

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
    if app.state.delete_confirmation.is_some()
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
        .selected_chat_id()
        .and_then(|chat_id| app.state.typing_users.get(&chat_id))
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
    if app.state.delete_confirmation.is_none_or(|confirmation| {
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
    handle_key_event(app, smoke_key(KeyCode::Home), client).await?;
    if app.state.selected_message_index != 0 || app.state.message_scroll_offset != 0 {
        return Err(color_eyre::eyre::eyre!(
            "Home did not jump to the first message and reset message scroll"
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
        || app.state.selected_chat_index != 0
        || app.state.messages.len() != 3
    {
        return Err(color_eyre::eyre::eyre!(
            "mouse wheel over chats should focus chats without loading another chat"
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
    terminal.draw(|frame| ui::render_layout(frame, &mut app, theme))?;

    let result = async {
        app.state.set_status(LOADING_TELEGRAM_STATUS);
        run_app(&mut terminal, &mut app, theme, &mut client).await
    }
    .await;

    let restore_result = restore_terminal(&mut terminal);
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

fn save_app_preferences_if_changed(app: &mut App, before: preferences::AppPreferences) {
    let after = preferences::AppPreferences::from_state(&app.state);
    if before == after {
        return;
    }

    let Some(path) = app.preferences_path.as_deref() else {
        return;
    };

    match after.save(path) {
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

async fn run_app<C: TelegramClient + Clone + Send + Sync + 'static>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    theme: &config::Theme,
    client: &mut C,
) -> Result<()> {
    diagnostics::event("run_loop_start", "event_loop=true");
    let (subscribe_updates_tx, mut subscribe_updates_rx) = tokio::sync::mpsc::unbounded_channel();
    let subscribe_updates_loader =
        SubscribeUpdatesLoader::new(client.clone(), subscribe_updates_tx);
    subscribe_updates_loader.spawn_subscribe_updates();
    let mut update_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Update>> = None;
    let (initial_state_load_tx, mut initial_state_load_rx) = tokio::sync::mpsc::unbounded_channel();
    let initial_state_loader = InitialStateLoader::new(client.clone(), initial_state_load_tx);
    initial_state_loader.spawn_initial_state();
    let mut initial_state_pending = true;
    let mut deferred_updates = Vec::new();
    let (chat_message_load_tx, mut chat_message_load_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut chat_message_loader = ChatMessageLoader::new(client.clone(), chat_message_load_tx);
    let (older_message_load_tx, mut older_message_load_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut older_message_loader = OlderMessageLoader::new(client.clone(), older_message_load_tx);
    let (folder_chat_load_tx, mut folder_chat_load_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut folder_chat_loader = FolderChatLoader::new(client.clone(), folder_chat_load_tx);
    let mark_read_loader = MarkChatReadLoader::new(client.clone());
    let (send_message_tx, mut send_message_rx) = tokio::sync::mpsc::unbounded_channel();
    let send_message_loader = SendMessageLoader::new(client.clone(), send_message_tx);
    let (delete_message_tx, mut delete_message_rx) = tokio::sync::mpsc::unbounded_channel();
    let delete_message_loader = DeleteMessageLoader::new(client.clone(), delete_message_tx);
    let (edit_message_tx, mut edit_message_rx) = tokio::sync::mpsc::unbounded_channel();
    let edit_message_loader = EditMessageLoader::new(client.clone(), edit_message_tx);
    let (reply_message_tx, mut reply_message_rx) = tokio::sync::mpsc::unbounded_channel();
    let reply_message_loader = ReplyMessageLoader::new(client.clone(), reply_message_tx);
    loop {
        let draw_started = Instant::now();
        terminal.draw(|f| ui::render_layout(f, app, theme))?;
        terminal_images::render_selected_image(terminal.backend_mut(), app)?;
        log_draw_duration("main_loop", draw_started);

        while let Ok(subscribe_result) = subscribe_updates_rx.try_recv() {
            apply_subscribe_updates_result(app, subscribe_result, &mut update_rx);
        }
        while let Ok(load_result) = initial_state_load_rx.try_recv() {
            apply_initial_state_load_result(app, load_result, &mark_read_loader);
            initial_state_pending = false;
            for update in deferred_updates.drain(..) {
                app.state.apply_update(update);
            }
        }
        if let Some(rx) = update_rx.as_mut() {
            while let Ok(update) = rx.try_recv() {
                if initial_state_pending {
                    deferred_updates.push(update);
                } else {
                    app.state.apply_update(update);
                }
            }
        }
        while let Ok(load_result) = chat_message_load_rx.try_recv() {
            apply_chat_message_load_result(
                app,
                chat_message_loader.latest_request_id(),
                load_result,
                &mark_read_loader,
            );
        }
        while let Ok(load_result) = older_message_load_rx.try_recv() {
            apply_older_message_load_result(
                app,
                older_message_loader.latest_request_id(),
                load_result,
            );
        }
        while let Ok(load_result) = folder_chat_load_rx.try_recv() {
            apply_folder_chat_load_result(
                app,
                folder_chat_loader.latest_request_id(),
                load_result,
                &mark_read_loader,
            );
        }
        while let Ok(send_result) = send_message_rx.try_recv() {
            apply_send_message_result(app, send_result);
        }
        while let Ok(delete_result) = delete_message_rx.try_recv() {
            apply_delete_message_result(app, delete_result);
        }
        while let Ok(edit_result) = edit_message_rx.try_recv() {
            apply_edit_message_result(app, edit_result);
        }
        while let Ok(reply_result) = reply_message_rx.try_recv() {
            apply_reply_message_result(app, reply_result);
        }

        app.state.check_notification_timeout();

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let mut progress = UiProgress::Live { terminal, theme };
                    handle_key_event_with_progress(
                        app,
                        key,
                        client,
                        &mut progress,
                        HandlerLoaders {
                            chat_message: Some(&mut chat_message_loader),
                            older_message: Some(&mut older_message_loader),
                            folder_chat: Some(&mut folder_chat_loader),
                            send_message: Some(&send_message_loader),
                            delete_message: Some(&delete_message_loader),
                            edit_message: Some(&edit_message_loader),
                            reply_message: Some(&reply_message_loader),
                        },
                    )
                    .await?;
                }
                Event::Mouse(mouse_event) => {
                    let mut progress = UiProgress::Live { terminal, theme };
                    handle_mouse_event_with_progress(
                        app,
                        mouse_event,
                        client,
                        &mut progress,
                        Some(&mut chat_message_loader),
                        Some(&mut folder_chat_loader),
                    )
                    .await?;
                }
                _ => {}
            }
        }

        if app.should_quit {
            diagnostics::event("run_loop_quit", "should_quit=true");
            break;
        }
    }

    Ok(())
}

struct SubscribeUpdatesResult {
    result: std::result::Result<tokio::sync::mpsc::UnboundedReceiver<Update>, String>,
}

struct InitialStateLoadResult {
    result: std::result::Result<actions::InitialStateLoad, String>,
}

struct ChatMessageLoadResult {
    request_id: u64,
    chat_id: i64,
    result: std::result::Result<Vec<Message>, String>,
}

struct OlderMessageLoadResult {
    request_id: u64,
    chat_id: i64,
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

struct HandlerLoaders<'a, C> {
    chat_message: Option<&'a mut ChatMessageLoader<C>>,
    older_message: Option<&'a mut OlderMessageLoader<C>>,
    folder_chat: Option<&'a mut FolderChatLoader<C>>,
    send_message: Option<&'a SendMessageLoader<C>>,
    delete_message: Option<&'a DeleteMessageLoader<C>>,
    edit_message: Option<&'a EditMessageLoader<C>>,
    reply_message: Option<&'a ReplyMessageLoader<C>>,
}

impl<C> HandlerLoaders<'_, C> {
    fn none() -> Self {
        Self {
            chat_message: None,
            older_message: None,
            folder_chat: None,
            send_message: None,
            delete_message: None,
            edit_message: None,
            reply_message: None,
        }
    }
}

struct SubscribeUpdatesLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<SubscribeUpdatesResult>,
}

impl<C> SubscribeUpdatesLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<SubscribeUpdatesResult>) -> Self {
        Self { client, tx }
    }

    fn spawn_subscribe_updates(&self) {
        let mut client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event("subscribe_updates_spawn", "updates=true");
        tokio::spawn(async move {
            let result = client
                .subscribe_updates()
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(SubscribeUpdatesResult { result });
        });
    }
}

struct InitialStateLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<InitialStateLoadResult>,
}

impl<C> InitialStateLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<InitialStateLoadResult>) -> Self {
        Self { client, tx }
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

struct ChatMessageLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<ChatMessageLoadResult>,
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
}

impl<C> ChatMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<ChatMessageLoadResult>) -> Self {
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
            let result = actions::fetch_latest_chat_messages(&client, chat_id).await;
            let _ = tx.send(ChatMessageLoadResult {
                request_id,
                chat_id,
                result,
            });
        }));
    }
}

struct OlderMessageLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<OlderMessageLoadResult>,
    latest_request_id: u64,
    current_handle: Option<tokio::task::JoinHandle<()>>,
}

impl<C> OlderMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<OlderMessageLoadResult>) -> Self {
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

    fn spawn_older_messages(
        &mut self,
        chat_id: i64,
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
                "request_id={request_id} chat_id={chat_id} before_message_id={before_message_id}"
            ),
        );
        self.current_handle = Some(tokio::spawn(async move {
            let result =
                actions::fetch_older_chat_messages(&client, chat_id, before_message_id).await;
            let _ = tx.send(OlderMessageLoadResult {
                request_id,
                chat_id,
                before_message_id,
                navigation,
                result,
            });
        }));
    }
}

struct FolderChatLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<FolderChatLoadResult>,
    latest_request_id: u64,
    current_handle: Option<tokio::task::JoinHandle<()>>,
}

struct SendMessageLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<SendMessageResult>,
}

struct DeleteMessageLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<DeleteMessageResult>,
}

struct EditMessageLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<EditMessageResult>,
}

struct ReplyMessageLoader<C> {
    client: C,
    tx: tokio::sync::mpsc::UnboundedSender<ReplyMessageResult>,
}

impl<C> SendMessageLoader<C>
where
    C: TelegramClient + Clone + Send + Sync + 'static,
{
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<SendMessageResult>) -> Self {
        Self { client, tx }
    }

    fn spawn_send_message(&self, pending: actions::PendingSend) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "send_message_spawn",
            format!("temp_id={} chat_id={}", pending.temp_id, pending.chat_id),
        );
        tokio::spawn(async move {
            let result =
                actions::send_message_result(&client, pending.chat_id, pending.content).await;
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
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<DeleteMessageResult>) -> Self {
        Self { client, tx }
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
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<EditMessageResult>) -> Self {
        Self { client, tx }
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
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<ReplyMessageResult>) -> Self {
        Self { client, tx }
    }

    fn spawn_reply_message(&self, chat_id: i64, message_id: i32, content: String) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        diagnostics::event(
            "reply_message_spawn",
            format!("chat_id={chat_id} message_id={message_id}"),
        );
        tokio::spawn(async move {
            let result = actions::reply_message_result(&client, chat_id, message_id, content).await;
            let _ = tx.send(ReplyMessageResult {
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
    fn new(client: C, tx: tokio::sync::mpsc::UnboundedSender<FolderChatLoadResult>) -> Self {
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
    update_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<Update>>,
) {
    match load.result {
        Ok(rx) => {
            diagnostics::event("subscribe_updates_result", "updates=true");
            *update_rx = Some(rx);
        }
        Err(error) => {
            diagnostics::event("subscribe_updates_result", "updates=false");
            app.state.set_error(error);
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

fn apply_chat_message_load_result<C>(
    app: &mut App,
    latest_request_id: u64,
    load: ChatMessageLoadResult,
    mark_read_loader: &MarkChatReadLoader<C>,
) where
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
        return;
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
        return;
    }

    match load.result {
        Ok(messages) => {
            let should_mark_read = selected_chat_needs_read_ack(app, load.chat_id, &messages);
            app.state.apply_loaded_selected_chat_messages(messages);
            if should_mark_read {
                mark_read_loader.spawn_mark_chat_read(load.chat_id);
            }
            app.state.clear_status();
        }
        Err(error) => app.state.set_error(error),
    }
}

fn apply_folder_chat_load_result<C>(
    app: &mut App,
    latest_request_id: u64,
    load: FolderChatLoadResult,
    mark_read_loader: &MarkChatReadLoader<C>,
) where
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
        return;
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
        return;
    }

    let read_ack_chat_id = folder_chat_load_read_ack_chat_id(&load);
    actions::apply_folder_chat_load_result(&mut app.state, load.result);
    if let Some(chat_id) =
        read_ack_chat_id.filter(|chat_id| app.state.selected_chat_id() == Some(*chat_id))
    {
        mark_read_loader.spawn_mark_chat_read(chat_id);
    }
    app.state.clear_status();
}

fn apply_older_message_load_result(
    app: &mut App,
    latest_request_id: u64,
    load: OlderMessageLoadResult,
) {
    if load.request_id != latest_request_id {
        diagnostics::event(
            "older_messages_load_ignored",
            format!(
                "reason=stale_request request_id={} latest_request_id={} chat_id={} before_message_id={}",
                load.request_id, latest_request_id, load.chat_id, load.before_message_id
            ),
        );
        return;
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
        return;
    }

    if app.state.messages.first().map(|message| message.id) != Some(load.before_message_id) {
        diagnostics::event(
            "older_messages_load_ignored",
            format!(
                "reason=stale_anchor request_id={} chat_id={} before_message_id={}",
                load.request_id, load.chat_id, load.before_message_id
            ),
        );
        return;
    }

    let added = actions::apply_older_chat_messages_result(&mut app.state, load.result);
    if added > 0 {
        app.state.clear_status();
        apply_older_message_navigation(&mut app.state, load.navigation);
    }
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
        let status = status.into();
        let show_status_banner = !is_loading_progress_status(&status);
        diagnostics::event(
            "progress_status",
            format!("status={status} banner={show_status_banner}"),
        );
        if show_status_banner {
            app.state.set_status(status);
        }
        match self {
            Self::Live { terminal, theme } => {
                let draw_started = Instant::now();
                terminal.draw(|frame| ui::render_layout(frame, app, theme))?;
                log_draw_duration("progress", draw_started);
            }
            Self::Silent => {}
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
    loaders: HandlerLoaders<'_, C>,
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
    if app.state.delete_confirmation.is_some() {
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
            if let Some((chat_id, before_message_id)) =
                actions::selected_older_messages_request(&mut app.state)
            {
                loader.spawn_older_messages(chat_id, before_message_id, navigation);
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
        message_keys::MessageKeyOutcome::OpenSelectedLink => {
            open_selected_message_link(app);
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
    progress.show(app, LOADING_FOLDER_CHATS_STATUS)?;
    if let Some(loader) = folder_chat_loader.as_deref_mut() {
        if let Some((folder_index, folder_id)) =
            actions::begin_open_folder_at(&mut app.state, index)
        {
            loader.spawn_folder_chats(folder_index, folder_id);
        }
        return Ok(());
    }

    let result = actions::open_folder_at(&mut app.state, client, index).await;
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
    progress.show(app, LOADING_CHAT_MESSAGES_STATUS)?;
    if let Some(loader) = chat_message_loader.as_deref_mut() {
        if let Some(chat_id) = actions::begin_open_chat_at(&mut app.state, index) {
            loader.spawn_latest_chat_messages(chat_id);
        }
        return Ok(());
    }

    let result = actions::open_chat_at(&mut app.state, client, index).await;
    if result.is_ok() {
        app.state.clear_status();
    }
    result
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
    if input_keys::handle_input_key(&mut app.state, key) == input_keys::InputKeyOutcome::Submit {
        let Some(action) = app.state.prepare_message_submit() else {
            return Ok(());
        };

        match action {
            state::MessageSubmitAction::Send { chat_id, content } => {
                let pending = actions::begin_send_message(&mut app.state, chat_id, content);
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
                message_id,
                content,
            } => {
                progress.show(app, SENDING_REPLY_STATUS)?;
                if let Some(loader) = reply_message_loader {
                    loader.spawn_reply_message(chat_id, message_id, content);
                } else {
                    actions::execute_message_submit_action(
                        &mut app.state,
                        client,
                        state::MessageSubmitAction::Reply {
                            chat_id,
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

async fn handle_mouse_event<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    mouse_event: crossterm::event::MouseEvent,
    client: &mut C,
) -> Result<()> {
    let mut progress = UiProgress::Silent;
    handle_mouse_event_with_progress(app, mouse_event, client, &mut progress, None, None).await
}

async fn handle_mouse_event_with_progress<C: TelegramClient + Clone + Send + Sync + 'static>(
    app: &mut App,
    mouse_event: crossterm::event::MouseEvent,
    client: &mut C,
    progress: &mut UiProgress<'_>,
    mut chat_message_loader: Option<&mut ChatMessageLoader<C>>,
    mut folder_chat_loader: Option<&mut FolderChatLoader<C>>,
) -> Result<()> {
    if app.state.delete_confirmation.is_some() {
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
                &mut folder_chat_loader,
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
                &mut chat_message_loader,
                index,
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
        CLI_USAGE_EXIT_CODE, CONFIG_LOAD_HELP, CONFIG_PATH_ARGUMENT_REQUIRED,
        ChatMessageLoadResult, ChatMessageLoader, DEFAULT_CONFIG_PATH, DeleteMessageLoader,
        DeleteMessageResult, EditMessageLoader, EditMessageResult, FolderChatLoadResult,
        FolderChatLoader, InitialStateLoadResult, InitialStateLoader, LOADING_CHAT_MESSAGES_STATUS,
        LOADING_TELEGRAM_STATUS, LOG_PATH_ARGUMENT_REQUIRED, LOGIN_2FA_ENABLED_STATUS,
        LOGIN_2FA_HINT_PREFIX, LOGIN_2FA_PROMPT, LOGIN_2FA_SIGNED_IN_PREFIX, LOGIN_CODE_PROMPT,
        LOGIN_CODE_SENT_PREFIX, LOGIN_FAILED_PREFIX, LOGIN_HEADER, LOGIN_PHONE_PROMPT,
        LOGIN_REQUESTING_CODE_STATUS, LOGIN_SESSION_SAVED_STATUS, LOGIN_SIGNED_IN_PREFIX,
        LOGIN_SIGNING_IN_STATUS, LOGIN_START_PROMPT, MarkChatReadLoader, OlderMessageLoadResult,
        OlderMessageLoader, OlderMessageNavigation, PROMPT_EMPTY_ERROR, PROMPT_EOF_ERROR,
        ReplyMessageLoader, ReplyMessageResult, RunMode, SAVING_EDIT_STATUS,
        SENDING_MESSAGE_STATUS, SENDING_REPLY_STATUS, SETUP_ERROR_EXIT_CODE,
        SMOKE_CHECK_AUTH_CONFLICT, SMOKE_CHECK_CONFIG_CONFLICT, SMOKE_OK_PREFIX, SendMessageLoader,
        SendMessageResult, SubscribeUpdatesLoader, SubscribeUpdatesResult, UiProgress,
        abort_running_task, apply_chat_message_load_result, apply_delete_message_result,
        apply_edit_message_result, apply_folder_chat_load_result, apply_initial_state_load_result,
        apply_older_message_load_result, apply_reply_message_result, apply_send_message_result,
        apply_subscribe_updates_result, check_auth_ok_message, check_auth_unauthorized_message,
        check_config_message, check_config_session_status, ensure_session_parent_dir,
        handle_mouse_event, load_checked_config, load_checked_config_with_session_parent,
        login_2fa_hint_message, login_2fa_signed_in_message, login_code_sent_message,
        login_failed_message, login_signed_in_message, message_submit_action_status,
        older_message_key_navigation, parse_args_from, preserve_prompt_input_line_spaces,
        require_prompt_line, require_prompt_response, save_app_preferences_if_changed,
        smoke_ok_message, trim_prompt_input_line, validate_config,
    };
    use crate::app::App;
    use crate::config::telegram::{Config, TelegramConfig};
    use crate::state::{DeleteConfirmation, FocusedPanel};
    use crate::telegram::{
        MockTelegramClient, TelegramClient,
        types::{Chat, Folder, Message, MessageStatus, Update, all_folder},
    };
    use chrono::Utc;
    use color_eyre::Result;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::layout::Rect;
    use std::{
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

    #[derive(Clone)]
    struct SlowFirstLatestMessagesClient;

    #[derive(Clone)]
    struct SlowFirstOlderMessagesClient;

    #[derive(Clone)]
    struct RecordingMarkReadClient {
        marked_chat_ids: Arc<Mutex<Vec<i64>>>,
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
        let mark_read_loader = MarkChatReadLoader::new(RecordingMarkReadClient {
            marked_chat_ids: marked_chat_ids.clone(),
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
                result: Ok(vec![read_message]),
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
                result: Ok(vec![unread_message]),
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
                result: Ok(vec![stale_message]),
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
                result: Ok(vec![current_message]),
            },
            &mark_read_loader,
        );
        assert_eq!(app.state.messages.len(), 1);
        assert_eq!(app.state.messages[0].id, 2);
    }

    #[test]
    fn async_subscribe_updates_result_installs_receiver() {
        let mut app = App::new();
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut update_rx = None;

        apply_subscribe_updates_result(
            &mut app,
            SubscribeUpdatesResult { result: Ok(rx) },
            &mut update_rx,
        );

        assert!(update_rx.is_some());
        assert!(app.state.error_message.is_none());
    }

    #[tokio::test]
    async fn async_subscribe_updates_loader_sends_result_without_blocking_handler() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let loader = SubscribeUpdatesLoader::new(MockTelegramClient::new(), tx);

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

        loader.spawn_latest_chat_messages(1);

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("background message load should respond")
            .expect("background message load channel should stay open");
        assert_eq!(result.request_id, 1);
        assert_eq!(result.chat_id, 1);
        assert_eq!(result.result.expect("mock load should succeed").len(), 3);
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
        assert_eq!(result.result.expect("newest load should succeed").len(), 1);
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

        loader.spawn_older_messages(1, 3, OlderMessageNavigation::OneLine);

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
    async fn async_older_message_loader_aborts_superseded_load() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut loader = OlderMessageLoader::new(SlowFirstOlderMessagesClient, tx);

        loader.spawn_older_messages(1, 10, OlderMessageNavigation::OneLine);
        loader.spawn_older_messages(1, 9, OlderMessageNavigation::Page);

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
        let pending = crate::actions::begin_send_message(&mut app.state, 1, "hello".to_string());
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

        loader.spawn_reply_message(1, 7, "reply".to_string());

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
        app.state.delete_confirmation = Some(DeleteConfirmation {
            chat_id: 1,
            message_id: 7,
        });
        let confirmation = crate::actions::begin_confirm_delete(&mut app.state)
            .expect("delete confirmation should be present");
        assert!(app.state.delete_confirmation.is_none());

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
                message_id: 2,
                content: "reply".to_string(),
            }),
            SENDING_REPLY_STATUS
        );
        assert_eq!(
            message_submit_action_status(&crate::state::MessageSubmitAction::Send {
                chat_id: 1,
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
    async fn mouse_events_are_ignored_while_delete_confirmation_is_open() {
        let mut app = App::new();
        let mut client = MockTelegramClient::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages_area = older_scroll_message_area();
        app.state.input_area = Rect::new(0, 20, 40, 3);
        app.state.messages = vec![message(1), message(2)];
        app.state.selected_message_index = 0;
        app.state.delete_confirmation = Some(DeleteConfirmation {
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
        assert!(app.state.delete_confirmation.is_some());

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
        assert!(app.state.delete_confirmation.is_some());
    }

    #[test]
    fn empty_args_use_shared_default_config_path() {
        let cli = parse_test_args(std::iter::empty::<&str>());

        assert_eq!(cli.config_path, DEFAULT_CONFIG_PATH);
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
