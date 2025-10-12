mod app;
mod config;
mod state;
mod telegram;
mod ui;

use app::App;
use color_eyre::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::Duration;
use std::sync::atomic::{AtomicI32, Ordering};
use telegram::{MockTelegramClient, TelegramClient};
use telegram::types::Update;

static TEMP_ID_COUNTER: AtomicI32 = AtomicI32::new(-1);

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    
    let mut terminal = setup_terminal()?;
    let mut app = App::new();
    let theme = config::Theme::default();
    let mut client = MockTelegramClient::new();
    
    client.connect().await?;
    
    app.state.folders = client.get_folders().await?;
    if !app.state.folders.is_empty() {
        let folder_id = Some(app.state.folders[0].id);
        app.state.chats = client.get_chats(folder_id).await?;
        
        if !app.state.chats.is_empty() {
            let chat_id = app.state.chats[0].id;
            app.state.messages = client.get_messages(chat_id, 50).await?;
        }
    }
    
    let mut update_rx = client.subscribe_updates().await?;
    
    let result = run_app(&mut terminal, &mut app, &theme, &mut client, &mut update_rx).await;
    
    restore_terminal(&mut terminal)?;
    
    result
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
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    theme: &config::Theme,
    client: &mut MockTelegramClient,
    update_rx: &mut tokio::sync::mpsc::UnboundedReceiver<Update>,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render_layout(f, app, theme))?;

        while let Ok(update) = update_rx.try_recv() {
            apply_update(&mut app.state, update);
        }
        
        app.state.check_error_timeout();

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key_event(app, key, client).await?;
                }
                Event::Mouse(mouse_event) => {
                    handle_mouse_event(app, mouse_event, client).await?;
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

fn apply_update(state: &mut state::AppState, update: Update) {
    match update {
        Update::NewMessage(msg) => {
            if state.chats.get(state.selected_chat_index).map(|c| c.id) == Some(msg.chat_id) {
                if !state.messages.iter().any(|m| m.id == msg.id) {
                    state.messages.push(msg.clone());
                }
            }
            
            if let Some(chat) = state.chats.iter_mut().find(|c| c.id == msg.chat_id) {
                chat.last_message = Some(msg.content.chars().take(50).collect());
                if !msg.is_own {
                    chat.unread_count += 1;
                }
            }
        }
        
        Update::EditMessage { chat_id, message_id, new_content } => {
            if let Some(msg) = state.messages.iter_mut().find(|m| m.id == message_id && m.chat_id == chat_id) {
                msg.content = new_content;
                msg.is_edited = true;
            }
        }
        
        Update::DeleteMessage { chat_id, message_id } => {
            state.messages.retain(|m| !(m.id == message_id && m.chat_id == chat_id));
            if state.selected_message_index >= state.messages.len() && !state.messages.is_empty() {
                state.selected_message_index = state.messages.len() - 1;
            }
        }
        
        Update::TypingStatus { chat_id, user_name, is_typing } => {
            if is_typing {
                let users = state.typing_users.entry(chat_id).or_insert_with(Vec::new);
                if !users.contains(&user_name) {
                    users.push(user_name);
                }
            } else {
                if let Some(users) = state.typing_users.get_mut(&chat_id) {
                    users.retain(|u| u != &user_name);
                    if users.is_empty() {
                        state.typing_users.remove(&chat_id);
                    }
                }
            }
        }
        
        Update::MessageStatusUpdate { chat_id, message_id, status } => {
            if let Some(msg) = state.messages.iter_mut().find(|m| m.id == message_id && m.chat_id == chat_id) {
                msg.status = status;
            }
        }
    }
}


async fn handle_key_event(app: &mut App, key: KeyEvent, client: &mut MockTelegramClient) -> Result<()> {
    if app.state.focused_panel == state::FocusedPanel::Input {
        handle_input_focused(app, key, client).await?;
    } else {
        handle_normal_navigation(app, key, client).await?;
    }
    Ok(())
}

async fn handle_normal_navigation(app: &mut App, key: KeyEvent, client: &mut MockTelegramClient) -> Result<()> {
    if app.state.confirm_delete_message_id.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let msg_id = app.state.confirm_delete_message_id.unwrap();
                let chat_id = app.state.chats[app.state.selected_chat_index].id;
                
                match client.delete_message(chat_id, msg_id).await {
                    Ok(_) => {
                        app.state.messages.retain(|m| m.id != msg_id);
                        app.state.confirm_delete_message_id = None;
                        if app.state.selected_message_index >= app.state.messages.len() && !app.state.messages.is_empty() {
                            app.state.selected_message_index = app.state.messages.len() - 1;
                        }
                    }
                    Err(e) => {
                        app.state.set_error(format!("Delete failed: {}", e));
                        app.state.confirm_delete_message_id = None;
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.state.confirm_delete_message_id = None;
            }
            _ => {}
        }
        return Ok(());
    }
    
    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Tab => app.state.focus_next_panel(),
        KeyCode::Down => {
            match app.state.focused_panel {
                state::FocusedPanel::Folders => {
                    app.state.focused_panel = state::FocusedPanel::Chats;
                }
                state::FocusedPanel::Chats => {
                    let old_index = app.state.selected_chat_index;
                    app.state.select_next_chat();
                    if old_index != app.state.selected_chat_index && !app.state.chats.is_empty() {
                        let chat_id = app.state.chats[app.state.selected_chat_index].id;
                        app.state.messages = client.get_messages(chat_id, 50).await?;
                        app.state.selected_message_index = 0;
                    }
                }
                state::FocusedPanel::Messages => {
                    if !app.state.messages.is_empty() && app.state.selected_message_index == app.state.messages.len() - 1 {
                        app.state.focused_panel = state::FocusedPanel::Input;
                    } else {
                        app.state.select_next_message();
                    }
                }
                state::FocusedPanel::Input => {}
            }
        }
        KeyCode::Up => {
            match app.state.focused_panel {
                state::FocusedPanel::Folders => {}
                state::FocusedPanel::Chats => {
                    if app.state.selected_chat_index == 0 {
                        app.state.focused_panel = state::FocusedPanel::Folders;
                        return Ok(());
                    }
                    let old_index = app.state.selected_chat_index;
                    app.state.select_prev_chat();
                    if old_index != app.state.selected_chat_index && !app.state.chats.is_empty() {
                        let chat_id = app.state.chats[app.state.selected_chat_index].id;
                        app.state.messages = client.get_messages(chat_id, 50).await?;
                        app.state.selected_message_index = 0;
                    }
                }
                state::FocusedPanel::Messages => app.state.select_prev_message(),
                state::FocusedPanel::Input => {
                    app.state.focused_panel = state::FocusedPanel::Messages;
                }
            }
        }
        KeyCode::Left => {
            match app.state.focused_panel {
                state::FocusedPanel::Folders => {
                    let old_index = app.state.selected_folder_index;
                    app.state.select_prev_folder();
                    app.state.ensure_selected_folder_visible();
                    if old_index != app.state.selected_folder_index && !app.state.folders.is_empty() {
                        let folder_id = if app.state.folders[app.state.selected_folder_index].name == "All" {
                            None
                        } else {
                            Some(app.state.folders[app.state.selected_folder_index].id)
                        };
                        app.state.chats = client.get_chats(folder_id).await?;
                        app.state.selected_chat_index = 0;
                        if !app.state.chats.is_empty() {
                            let chat_id = app.state.chats[0].id;
                            app.state.messages = client.get_messages(chat_id, 50).await?;
                        } else {
                            app.state.messages.clear();
                        }
                    }
                }
                state::FocusedPanel::Chats => {
                    app.state.focused_panel = state::FocusedPanel::Folders;
                }
                state::FocusedPanel::Messages => {
                    app.state.focused_panel = state::FocusedPanel::Chats;
                }
                state::FocusedPanel::Input => {}
            }
        }
        KeyCode::Right => {
            match app.state.focused_panel {
                state::FocusedPanel::Folders => {
                    let old_index = app.state.selected_folder_index;
                    app.state.select_next_folder();
                    app.state.ensure_selected_folder_visible();
                    if old_index != app.state.selected_folder_index && !app.state.folders.is_empty() {
                        let folder_id = if app.state.folders[app.state.selected_folder_index].name == "All" {
                            None
                        } else {
                            Some(app.state.folders[app.state.selected_folder_index].id)
                        };
                        app.state.chats = client.get_chats(folder_id).await?;
                        app.state.selected_chat_index = 0;
                        if !app.state.chats.is_empty() {
                            let chat_id = app.state.chats[0].id;
                            app.state.messages = client.get_messages(chat_id, 50).await?;
                        } else {
                            app.state.messages.clear();
                        }
                    }
                }
                state::FocusedPanel::Chats => {
                    app.state.focused_panel = state::FocusedPanel::Messages;
                }
                state::FocusedPanel::Messages => {}
                state::FocusedPanel::Input => {}
            }
        }
        KeyCode::Char('<') => app.state.adjust_split_left(),
        KeyCode::Char('>') => app.state.adjust_split_right(),
        KeyCode::Char('e') if app.state.focused_panel == state::FocusedPanel::Messages => {
            if !app.state.messages.is_empty() {
                if let Some(msg) = app.state.messages.get(app.state.selected_message_index) {
                    if msg.is_own && msg.can_edit {
                        app.state.enter_edit_mode(msg.id, msg.content.clone());
                    } else {
                        app.state.set_error("Cannot edit this message".to_string());
                    }
                }
            }
        }
        KeyCode::Char('r') if app.state.focused_panel == state::FocusedPanel::Messages => {
            if !app.state.messages.is_empty() {
                if let Some(msg) = app.state.messages.get(app.state.selected_message_index) {
                    app.state.enter_reply_mode(msg.id);
                }
            }
        }
        KeyCode::Char('d') if app.state.focused_panel == state::FocusedPanel::Messages => {
            if !app.state.messages.is_empty() {
                if let Some(msg) = app.state.messages.get(app.state.selected_message_index) {
                    if msg.is_own && msg.can_delete {
                        app.state.confirm_delete_message_id = Some(msg.id);
                    } else {
                        app.state.set_error("Cannot delete this message".to_string());
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_input_focused(app: &mut App, key: KeyEvent, client: &mut MockTelegramClient) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            app.state.clear_input_mode();
            app.state.focused_panel = state::FocusedPanel::Messages;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.state.clear_input_mode();
            app.state.focused_panel = state::FocusedPanel::Messages;
        }
        KeyCode::Char(c) => {
            app.state.input_buffer.push(c);
        }
        KeyCode::Backspace => {
            app.state.input_buffer.pop();
        }
        KeyCode::Enter => {
            handle_message_send(app, client).await?;
        }
        _ => {}
    }
    Ok(())
}

async fn handle_message_send(app: &mut App, client: &mut MockTelegramClient) -> Result<()> {
    if app.state.input_buffer.is_empty() {
        return Ok(());
    }
    
    if app.state.chats.is_empty() {
        app.state.set_error("No chat selected".to_string());
        return Ok(());
    }
    
    let chat_id = app.state.chats[app.state.selected_chat_index].id;
    let content = app.state.input_buffer.clone();
    
    if let Some(msg_id) = app.state.editing_message_id {
        match client.edit_message(chat_id, msg_id, content).await {
            Ok(_) => {
                if let Some(msg) = app.state.messages.iter_mut().find(|m| m.id == msg_id) {
                    msg.content = app.state.input_buffer.clone();
                    msg.is_edited = true;
                }
                app.state.clear_input_mode();
            }
            Err(e) => {
                app.state.set_error(format!("Edit failed: {}", e));
            }
        }
    } else if let Some(reply_id) = app.state.replying_to_message_id {
        match client.reply_to_message(chat_id, reply_id, content).await {
            Ok(new_msg) => {
                app.state.messages.push(new_msg);
                app.state.clear_input_mode();
            }
            Err(e) => {
                app.state.set_error(format!("Reply failed: {}", e));
            }
        }
    } else {
        let temp_id = TEMP_ID_COUNTER.fetch_sub(1, Ordering::SeqCst);
        
        let pending_msg = telegram::types::Message {
            id: temp_id,
            chat_id,
            sender_name: "You".to_string(),
            sender_id: 0,
            content: content.clone(),
            timestamp: chrono::Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_id: None,
            reply_to_content: None,
            status: telegram::types::MessageStatus::Sending,
            can_edit: true,
            can_delete: true,
            error: None,
        };
        
        app.state.messages.push(pending_msg.clone());
        app.state.input_buffer.clear();
        
        match client.send_message(chat_id, content).await {
            Ok(sent_msg) => {
                if let Some(msg) = app.state.messages.iter_mut().find(|m| m.id == temp_id) {
                    *msg = sent_msg;
                }
            }
            Err(e) => {
                if let Some(msg) = app.state.messages.iter_mut().find(|m| m.id == temp_id) {
                    msg.status = telegram::types::MessageStatus::Failed;
                    msg.error = Some(e.to_string());
                }
            }
        }
    }
    
    Ok(())
}

async fn handle_mouse_event(
    app: &mut App,
    mouse_event: crossterm::event::MouseEvent,
    client: &mut MockTelegramClient,
) -> Result<()> {
    
    match mouse_event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let x = mouse_event.column;
            let y = mouse_event.row;
            
            if app.state.folders_area.contains(ratatui::layout::Position::new(x, y)) {
                app.state.focused_panel = state::FocusedPanel::Folders;
                
                let (visible_folders, _, _) = app.state.get_visible_folders();
                if !visible_folders.is_empty() {
                    let border = 2;
                    let usable_width = app.state.folders_area.width.saturating_sub(border);
                    let segment_width = usable_width / visible_folders.len() as u16;
                    
                    let relative_x = x.saturating_sub(app.state.folders_area.x + 1);
                    let clicked_visible_index = (relative_x / segment_width.max(1)) as usize;
                    
                    if clicked_visible_index < visible_folders.len() {
                        let folder_idx = app.state.folder_scroll_offset + clicked_visible_index;
                        app.state.select_folder(folder_idx);
                        let folder_id = if app.state.folders[folder_idx].name == "All" {
                            None
                        } else {
                            Some(app.state.folders[folder_idx].id)
                        };
                        app.state.chats = client.get_chats(folder_id).await?;
                        app.state.selected_chat_index = 0;
                        if !app.state.chats.is_empty() {
                            let chat_id = app.state.chats[0].id;
                            app.state.messages = client.get_messages(chat_id, 50).await?;
                        } else {
                            app.state.messages.clear();
                        }
                    }
                }
            } else if app.state.chats_area.contains(ratatui::layout::Position::new(x, y)) {
                app.state.focused_panel = state::FocusedPanel::Chats;
                
                let border_offset = 1;
                let relative_y = y.saturating_sub(app.state.chats_area.y + border_offset);
                let height_per_chat = 2;
                let clicked_chat = (relative_y / height_per_chat) as usize;
                
                if clicked_chat < app.state.chats.len() {
                    app.state.select_chat(clicked_chat);
                    let chat_id = app.state.chats[clicked_chat].id;
                    app.state.messages = client.get_messages(chat_id, 50).await?;
                    app.state.selected_message_index = 0;
                }
            } else if app.state.messages_area.contains(ratatui::layout::Position::new(x, y)) {
                app.state.focused_panel = state::FocusedPanel::Messages;
            } else if app.state.input_area.contains(ratatui::layout::Position::new(x, y)) {
                app.state.focused_panel = state::FocusedPanel::Input;
            }
        }
        _ => {}
    }
    
    Ok(())
}
