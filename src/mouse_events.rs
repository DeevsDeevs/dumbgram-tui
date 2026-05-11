use crate::{
    state::{AppState, FocusedPanel, message_visible_row_height_for_width_capped},
    telegram::types::message_display_content,
    text::{char_display_width, display_width},
    ui::{self, SELECTED_ROW_SYMBOL},
};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseScrollOutcome {
    Handled,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MouseClickOutcome {
    Handled,
    OpenFolderAt(usize),
    OpenChatAt(usize),
    OpenLink(String),
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
        let display_offset = if state.chat_search_active() {
            state.chat_search_scroll_offset
        } else {
            state.chat_scroll_offset
        };
        let display_index = display_offset + (relative_y / height_per_chat) as usize;
        let display_indices = state.chat_display_indices();
        let Some(chat_index) = display_indices.get(display_index).copied() else {
            return MouseClickOutcome::Handled;
        };
        if chat_index == state.selected_chat_index {
            MouseClickOutcome::Handled
        } else {
            MouseClickOutcome::OpenChatAt(chat_index)
        }
    } else if state.messages_area.contains(position) {
        state.focused_panel = FocusedPanel::Messages;
        let relative_y = y.saturating_sub(state.messages_area.y + 1) as usize;
        let clicked_link = message_link_at_click(state, x, y);
        state.select_message_at_visible_row(relative_y);
        if let Some(url) = clicked_link {
            MouseClickOutcome::OpenLink(url)
        } else {
            MouseClickOutcome::Handled
        }
    } else if state.input_area.contains(position) {
        state.focused_panel = FocusedPanel::Input;
        let relative_x = x.saturating_sub(state.input_area.x + 1) as usize;
        state.move_input_cursor_to_visible_column(relative_x);
        MouseClickOutcome::Handled
    } else {
        MouseClickOutcome::Ignored
    }
}

fn message_link_at_click(state: &AppState, x: u16, y: u16) -> Option<String> {
    let relative_y = y.saturating_sub(state.messages_area.y + 1) as usize;
    let relative_x = x.saturating_sub(state.messages_area.x + 1) as usize;
    let mut current_row = 0;

    for message in state.messages.iter().skip(state.message_scroll_offset) {
        let remaining_rows = state.message_visible_capacity().saturating_sub(current_row);
        let message_height = message_visible_row_height_for_width_capped(
            message,
            ui::list_text_width(state.messages_area.width),
            remaining_rows,
        );
        if relative_y < current_row + message_height {
            let line_index = relative_y - current_row;
            let time_str = message.timestamp.format("%H:%M").to_string();
            let metadata = ui::messages::message_metadata(
                &time_str,
                message.is_edited,
                ui::messages::message_status_label(&message.status, message.is_own),
                message.error.as_deref(),
            );
            let sender = format!("{}: ", message.sender_name);
            let text_width = ui::list_text_width(state.messages_area.width);
            let content_width = text_width
                .saturating_sub(display_width(&sender) + display_width(&metadata))
                .max(1);
            let display_content = message_display_content(message.media.as_ref(), &message.content);
            let line_ranges = wrap_display_line_ranges(
                &display_content,
                content_width,
                text_width.saturating_sub(display_width(&sender)).max(1),
                line_index + 1,
            );
            let (line_start, line) = line_ranges.get(line_index)?;
            let line_end = line_start + line.len();
            let content_start = display_width(SELECTED_ROW_SYMBOL) + display_width(&sender);
            let column = relative_x.checked_sub(content_start)?;

            return link_in_wrapped_line_at_column(&display_content, *line_start, line_end, column);
        }
        current_row += message_height;
        if current_row >= state.message_visible_capacity() {
            return None;
        }
    }

    None
}

fn wrap_display_line_ranges(
    text: &str,
    first_width: usize,
    subsequent_width: usize,
    max_lines: usize,
) -> Vec<(usize, String)> {
    if max_lines == 0 {
        return Vec::new();
    }

    let first_width = first_width.max(1);
    let subsequent_width = subsequent_width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_start = 0;
    let mut current_width = 0;
    let mut current_limit = first_width;

    for (byte_index, ch) in text.char_indices() {
        if ch == '\n' {
            lines.push((current_start, std::mem::take(&mut current)));
            if lines.len() >= max_lines {
                return lines;
            }
            current_start = byte_index + ch.len_utf8();
            current_width = 0;
            current_limit = subsequent_width;
            continue;
        }

        let char_width = char_display_width(ch);
        if current_width > 0 && current_width + char_width > current_limit {
            lines.push((current_start, std::mem::take(&mut current)));
            if lines.len() >= max_lines {
                return lines;
            }
            current_start = byte_index;
            current_width = 0;
            current_limit = subsequent_width;
        }

        current.push(ch);
        current_width += char_width;
    }

    lines.push((current_start, current));
    lines
}

fn link_in_wrapped_line_at_column(
    text: &str,
    line_start: usize,
    line_end: usize,
    column: usize,
) -> Option<String> {
    let line = &text[line_start..line_end];
    crate::links::links_in_text(text)
        .into_iter()
        .find_map(|link| {
            let overlap_start = link.start.max(line_start);
            let overlap_end = link.end.min(line_end);
            if overlap_start >= overlap_end {
                return None;
            }

            let start_column = display_width(&line[..overlap_start - line_start]);
            let end_column = display_width(&line[..overlap_end - line_start]);
            (column >= start_column && column < end_column).then_some(link.url)
        })
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
        message_with_content(id, &format!("message {id}"))
    }

    fn message_with_content(id: i32, content: &str) -> Message {
        Message {
            id,
            chat_id: 7,
            sender_name: "me".to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_content: None,
            media: None,
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
    fn chat_search_click_uses_filtered_scroll_offset() {
        let mut state = AppState::new();
        state.chats_area = Rect::new(0, 5, 30, 8);
        state.chats = vec![
            chat(1, "Chat 1"),
            chat(2, "Chat 2"),
            chat(3, "Chat 3"),
            chat(4, "Chat 4"),
            chat(5, "Chat 5"),
        ];
        state.begin_chat_search();
        state.push_chat_search_char('c');
        state.chat_search_scroll_offset = 2;

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    2,
                    8
                )
            ),
            MouseClickOutcome::OpenChatAt(3)
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

    #[test]
    fn message_click_on_visible_url_requests_link_open() {
        let mut state = AppState::new();
        state.messages_area = Rect::new(30, 5, 80, 8);
        state.messages = vec![message_with_content(1, "go https://example.org now")];

        let url_column = 30 + 1 + 2 + 4 + 3;
        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    url_column,
                    6
                )
            ),
            MouseClickOutcome::OpenLink("https://example.org".to_string())
        );
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert_eq!(state.selected_message_index, 0);
    }

    #[test]
    fn message_click_on_wrapped_url_continuation_requests_link_open() {
        let mut state = AppState::new();
        state.messages_area = Rect::new(30, 5, 35, 8);
        state.messages = vec![message_with_content(1, "prefix https://example.org/path")];

        let wrapped_url_column = 30 + 1 + 2 + 4 + 2;
        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    wrapped_url_column,
                    7
                )
            ),
            MouseClickOutcome::OpenLink("https://example.org/path".to_string())
        );
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert_eq!(state.selected_message_index, 0);
    }
}
