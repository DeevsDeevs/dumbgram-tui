mod app;
mod config;
mod state;
mod telegram;
mod ui;

use app::App;
use color_eyre::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use telegram::{MockTelegramClient, TelegramClient};

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
    
    let result = run_app(&mut terminal, &mut app, &theme, &mut client).await;
    
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
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render_layout(f, app, theme))?;

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                handle_key_event(app, key, client).await?;
            }
            Event::Mouse(mouse_event) => {
                handle_mouse_event(app, mouse_event, client).await?;
            }
            _ => {}
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

async fn handle_key_event(app: &mut App, key: KeyEvent, client: &mut MockTelegramClient) -> Result<()> {
    if app.state.focused_panel == state::FocusedPanel::Input {
        handle_input_focused(app, key);
    } else {
        handle_normal_navigation(app, key, client).await?;
    }
    Ok(())
}

async fn handle_normal_navigation(app: &mut App, key: KeyEvent, client: &mut MockTelegramClient) -> Result<()> {
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
        _ => {}
    }
    Ok(())
}

fn handle_input_focused(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.state.focused_panel = state::FocusedPanel::Messages;
        }
        KeyCode::Char(c) => {
            app.state.input_buffer.push(c);
        }
        KeyCode::Backspace => {
            app.state.input_buffer.pop();
        }
        KeyCode::Enter => {
            if !app.state.input_buffer.is_empty() {
                app.state.input_buffer.clear();
            }
        }
        _ => {}
    }
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
