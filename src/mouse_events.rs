use crate::state::{AppState, FocusedPanel};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseScrollOutcome {
    Handled,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseClickOutcome {
    Handled,
    OpenFolderAt(usize),
    OpenChatAt(usize),
    Ignored,
}

pub fn handle_mouse_scroll(state: &mut AppState, mouse_event: MouseEvent) -> MouseScrollOutcome {
    let position = Position::new(mouse_event.column, mouse_event.row);

    match mouse_event.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            if state.chats_area.contains(position) =>
        {
            state.focused_panel = FocusedPanel::Chats;
            MouseScrollOutcome::Handled
        }
        MouseEventKind::ScrollDown if state.messages_area.contains(position) => {
            state.focused_panel = FocusedPanel::Messages;
            state.select_next_message();
            MouseScrollOutcome::Handled
        }
        MouseEventKind::ScrollUp if state.messages_area.contains(position) => {
            state.focused_panel = FocusedPanel::Messages;
            state.select_prev_message();
            MouseScrollOutcome::Handled
        }
        _ => MouseScrollOutcome::Ignored,
    }
}

pub fn handle_mouse_click(state: &mut AppState, mouse_event: MouseEvent) -> MouseClickOutcome {
    if !matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Left)) {
        return MouseClickOutcome::Ignored;
    }

    let x = mouse_event.column;
    let y = mouse_event.row;
    let position = Position::new(x, y);

    if state.folders_area.contains(position) {
        state.focused_panel = FocusedPanel::Folders;

        let relative_x = x.saturating_sub(state.folders_area.x + 1) as usize;
        if let Some(folder_index) = state.folder_index_at_visible_column(relative_x) {
            if folder_index == state.selected_folder_index {
                MouseClickOutcome::Handled
            } else {
                MouseClickOutcome::OpenFolderAt(folder_index)
            }
        } else {
            MouseClickOutcome::Handled
        }
    } else if state.chats_area.contains(position) {
        state.focused_panel = FocusedPanel::Chats;

        if state.chats.is_empty() {
            return MouseClickOutcome::Handled;
        }

        let border_offset = 1;
        let relative_y = y.saturating_sub(state.chats_area.y + border_offset);
        let height_per_chat = 2;
        let chat_index = state.chat_scroll_offset + (relative_y / height_per_chat) as usize;
        if chat_index >= state.chats.len() || chat_index == state.selected_chat_index {
            MouseClickOutcome::Handled
        } else {
            MouseClickOutcome::OpenChatAt(chat_index)
        }
    } else if state.messages_area.contains(position) {
        state.focused_panel = FocusedPanel::Messages;
        let relative_y = y.saturating_sub(state.messages_area.y + 1) as usize;
        state.select_message_at_visible_row(relative_y);
        MouseClickOutcome::Handled
    } else if state.input_area.contains(position) {
        state.focused_panel = FocusedPanel::Input;
        let relative_x = x.saturating_sub(state.input_area.x + 1) as usize;
        state.move_input_cursor_to_visible_column(relative_x);
        MouseClickOutcome::Handled
    } else {
        MouseClickOutcome::Ignored
    }
}

#[cfg(test)]
mod tests {
    use super::{MouseClickOutcome, MouseScrollOutcome, handle_mouse_click, handle_mouse_scroll};
    use crate::state::{AppState, FocusedPanel};
    use crate::telegram::types::{Chat, Folder, Message, MessageStatus, all_folder};
    use chrono::Utc;
    use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn chat(id: i64, name: &str) -> Chat {
        Chat {
            id,
            name: name.to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }
    }

    fn message(id: i32) -> Message {
        Message {
            id,
            chat_id: 7,
            sender_name: "me".to_string(),
            content: format!("message {id}"),
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_content: None,
            status: MessageStatus::Sent,
            can_edit: true,
            can_delete: true,
            error: None,
        }
    }

    fn folder(id: i32, name: &str) -> Folder {
        Folder {
            id,
            name: name.to_string(),
            unread_count: 0,
        }
    }

    #[test]
    fn chat_scroll_focuses_chats_without_opening_or_loading_chat() {
        let mut state = AppState::new();
        state.chats_area = Rect::new(10, 5, 20, 8);
        state.chats = vec![chat(1, "Chat 1"), chat(2, "Chat 2")];
        state.focused_panel = FocusedPanel::Messages;

        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollDown, 11, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
        assert_eq!(state.selected_chat_index, 0);

        state.selected_chat_index = 1;
        state.focused_panel = FocusedPanel::Messages;
        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollUp, 11, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
        assert_eq!(state.selected_chat_index, 1);
    }

    #[test]
    fn chat_scroll_is_handled_without_opening_chat_when_no_alternate_chat_exists() {
        let mut state = AppState::new();
        state.chats_area = Rect::new(10, 5, 20, 8);
        state.focused_panel = FocusedPanel::Messages;

        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollDown, 11, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);

        state.focused_panel = FocusedPanel::Messages;
        state.chats = vec![chat(1, "Chat 1")];
        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollUp, 11, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
    }

    #[test]
    fn message_scroll_moves_message_selection_and_focuses_messages() {
        let mut state = AppState::new();
        state.messages_area = Rect::new(30, 5, 40, 8);
        state.messages = vec![message(1), message(2)];
        state.focused_panel = FocusedPanel::Chats;

        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollDown, 31, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert_eq!(state.selected_message_index, 1);

        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollUp, 31, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.selected_message_index, 0);
    }

    #[test]
    fn non_scroll_or_out_of_bounds_mouse_events_are_ignored() {
        let mut state = AppState::new();
        state.chats_area = Rect::new(10, 5, 20, 8);

        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollDown, 0, 0)),
            MouseScrollOutcome::Ignored
        );
        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::Moved, 11, 6)),
            MouseScrollOutcome::Ignored
        );
    }

    #[test]
    fn folder_click_requests_opening_clicked_visible_folder() {
        let mut state = AppState::new();
        state.folders_area = Rect::new(0, 0, 80, 3);
        state.folders = vec![all_folder(0), folder(2, "Work")];
        state.selected_folder_index = 1;

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    2,
                    1
                )
            ),
            MouseClickOutcome::OpenFolderAt(0)
        );
        assert_eq!(state.focused_panel, FocusedPanel::Folders);
    }

    #[test]
    fn selected_folder_click_is_handled_without_opening_folder() {
        let mut state = AppState::new();
        state.folders_area = Rect::new(0, 0, 80, 3);
        state.folders = vec![all_folder(0), folder(2, "Work")];

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    2,
                    1
                )
            ),
            MouseClickOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Folders);
    }

    #[test]
    fn folder_click_uses_rendered_label_widths_not_equal_segments() {
        let mut state = AppState::new();
        state.folders_area = Rect::new(0, 0, 40, 3);
        state.folders = vec![folder(1, "好"), folder(2, "Work")];

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    5,
                    1
                )
            ),
            MouseClickOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Folders);

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    8,
                    1
                )
            ),
            MouseClickOutcome::OpenFolderAt(1)
        );
    }

    #[test]
    fn chat_click_requests_opening_clicked_chat() {
        let mut state = AppState::new();
        state.chats_area = Rect::new(0, 5, 30, 8);
        state.chats = vec![
            chat(1, "Chat 1"),
            chat(2, "Chat 2"),
            chat(3, "Chat 3"),
            chat(4, "Chat 4"),
            chat(5, "Chat 5"),
        ];
        state.chat_scroll_offset = 3;

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    2,
                    8
                )
            ),
            MouseClickOutcome::OpenChatAt(4)
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
    }

    #[test]
    fn selected_or_out_of_range_chat_click_is_handled_without_opening_chat() {
        let mut state = AppState::new();
        state.chats_area = Rect::new(0, 5, 30, 8);
        state.chats = vec![chat(1, "Chat 1"), chat(2, "Chat 2")];
        state.selected_chat_index = 1;

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    2,
                    8
                )
            ),
            MouseClickOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    2,
                    12
                )
            ),
            MouseClickOutcome::Handled
        );
    }

    #[test]
    fn message_and_input_clicks_update_local_state() {
        let mut state = AppState::new();
        state.messages_area = Rect::new(0, 5, 40, 8);
        state.input_area = Rect::new(0, 20, 20, 3);
        state.messages = vec![message(1), message(2)];
        state.input_buffer = "a好b".to_string();
        state.move_input_cursor_to_end();

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    2,
                    7
                )
            ),
            MouseClickOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert_eq!(state.selected_message_index, 1);

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    4,
                    21
                )
            ),
            MouseClickOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        assert_eq!(state.input_cursor(), 2);
    }
}
