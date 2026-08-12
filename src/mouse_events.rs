use crate::{
    state::{
        AppState, ContextMenuAction, ContextMenuTarget, FocusedPanel,
        message_reply_preview_visible, message_visible_row_height_for_width_capped,
    },
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
    OpenThreadTopicAt(usize),
    OpenLink(String),
    ContextMenuAction(ContextMenuTarget, ContextMenuAction),
    Ignored,
}

pub fn handle_mouse_scroll(state: &mut AppState, mouse_event: MouseEvent) -> MouseScrollOutcome {
    let is_scroll = matches!(
        mouse_event.kind,
        MouseEventKind::ScrollDown
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight
    );
    if !is_scroll {
        return MouseScrollOutcome::Ignored;
    }
    if state.split_drag_active || state.context_menu().is_some() {
        return MouseScrollOutcome::Handled;
    }

    let position = Position::new(mouse_event.column, mouse_event.row);

    match mouse_event.kind {
        MouseEventKind::ScrollDown if state.chats_area.contains(position) => {
            state.focused_panel = FocusedPanel::Chats;
            state.scroll_chats(1);
            MouseScrollOutcome::Handled
        }
        MouseEventKind::ScrollUp if state.chats_area.contains(position) => {
            state.focused_panel = FocusedPanel::Chats;
            state.scroll_chats(-1);
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

fn panel_inner_contains(area: ratatui::layout::Rect, position: Position) -> bool {
    position.x > area.x
        && position.x < area.x.saturating_add(area.width).saturating_sub(1)
        && position.y > area.y
        && position.y < area.y.saturating_add(area.height).saturating_sub(1)
}

fn chat_index_at(state: &AppState, x: u16, y: u16) -> Option<usize> {
    if !panel_inner_contains(state.chats_area, Position::new(x, y)) || state.chats.is_empty() {
        return None;
    }
    let relative_y = y.saturating_sub(state.chats_area.y + 1);
    let display_offset = if state.chat_search_active() {
        state.chat_search_scroll_offset
    } else {
        state.chat_scroll_offset
    };
    let display_index = display_offset + (relative_y / 2) as usize;
    state.chat_display_indices().get(display_index).copied()
}

pub fn handle_mouse_click(state: &mut AppState, mouse_event: MouseEvent) -> MouseClickOutcome {
    let x = mouse_event.column;
    let y = mouse_event.row;
    let position = Position::new(x, y);

    if state.split_drag_active {
        match mouse_event.kind {
            MouseEventKind::Drag(MouseButton::Left) => state.drag_split_to(x),
            MouseEventKind::Up(MouseButton::Left) => state.end_split_drag(),
            _ => {}
        }
        return MouseClickOutcome::Handled;
    }

    if state.context_menu().is_some() {
        match mouse_event.kind {
            MouseEventKind::Moved => state.hover_context_menu_at(x, y),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = state.context_menu_item_at(x, y) {
                    if let Some((target, action)) = state.take_context_menu_action(index) {
                        return MouseClickOutcome::ContextMenuAction(target, action);
                    }
                } else {
                    state.close_context_menu();
                }
            }
            _ => {}
        }
        return MouseClickOutcome::Handled;
    }

    if matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Left))
        && state.split_divider_contains(x, y)
    {
        state.begin_split_drag(x);
        return MouseClickOutcome::Handled;
    }

    if matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Right)) {
        if let Some(chat_index) = chat_index_at(state, x, y) {
            state.focused_panel = FocusedPanel::Chats;
            let chat_id = state.chats[chat_index].id;
            state.open_context_menu(ContextMenuTarget::Chat { chat_id }, x, y);
            return MouseClickOutcome::Handled;
        }
        if panel_inner_contains(state.messages_area, position) {
            let relative_y = y.saturating_sub(state.messages_area.y + 1) as usize;
            if let Some(message_index) = state.message_index_at_visible_row(relative_y) {
                state.focused_panel = FocusedPanel::Messages;
                state.selected_message_index = message_index;
                state.ensure_selected_message_visible();
                let (chat_id, message_id) = {
                    let message = &state.messages[message_index];
                    (message.chat_id, message.id)
                };
                state.open_context_menu(
                    ContextMenuTarget::Message {
                        chat_id,
                        message_id,
                    },
                    x,
                    y,
                );
            }
            return MouseClickOutcome::Handled;
        }
        return MouseClickOutcome::Ignored;
    }

    if !matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Left)) {
        return MouseClickOutcome::Ignored;
    }

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
    } else if let Some(chat_index) = chat_index_at(state, x, y) {
        state.focused_panel = FocusedPanel::Chats;
        if chat_index == state.selected_chat_index {
            MouseClickOutcome::Handled
        } else {
            MouseClickOutcome::OpenChatAt(chat_index)
        }
    } else if state.chats_area.contains(position) {
        state.focused_panel = FocusedPanel::Chats;
        MouseClickOutcome::Handled
    } else if state.thread_topics_area.contains(position) {
        state.focused_panel = FocusedPanel::Messages;
        let relative_x = x.saturating_sub(state.thread_topics_area.x + 1) as usize;
        if let Some(topic_index) = state.thread_topic_index_at_visible_column(relative_x) {
            if topic_index == state.selected_thread_topic_index {
                MouseClickOutcome::Handled
            } else {
                state.select_thread_topic_at(topic_index);
                MouseClickOutcome::OpenThreadTopicAt(topic_index)
            }
        } else {
            MouseClickOutcome::Handled
        }
    } else if panel_inner_contains(state.messages_area, position) {
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
            if message_reply_preview_visible(message, remaining_rows)
                && line_index + 1 == message_height
            {
                return None;
            }
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
    use crate::state::{AppState, ContextMenuTarget, FocusedPanel};
    use crate::telegram::types::{Chat, Folder, Message, MessageStatus, ThreadTopic, all_folder};
    use chrono::Utc;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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
            thread_topic_id: None,
            sender_identity: None,
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

    fn thread_topic(id: i32, title: &str) -> ThreadTopic {
        ThreadTopic {
            id,
            title: title.to_string(),
            top_message_id: id + 1000,
            unread_count: 0,
            is_closed: false,
            is_pinned: false,
        }
    }

    #[test]
    fn chat_scroll_focuses_chats_without_opening_or_loading_chat() {
        let mut state = AppState::new();
        state.chats_area = Rect::new(10, 5, 20, 8);
        state.chats = (1..=5).map(|id| chat(id, &format!("Chat {id}"))).collect();
        state.focused_panel = FocusedPanel::Messages;

        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollDown, 11, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.chat_scroll_offset, 1);

        state.focused_panel = FocusedPanel::Messages;
        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollUp, 11, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.chat_scroll_offset, 0);
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
    fn thread_topic_click_selects_topic_and_requests_open() {
        let mut state = AppState::new();
        state.thread_topics_area = Rect::new(30, 5, 60, 3);
        state.thread_topics = vec![
            thread_topic(101, "General"),
            thread_topic(102, "Deployments"),
        ];

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    45,
                    6
                )
            ),
            MouseClickOutcome::OpenThreadTopicAt(1)
        );
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert_eq!(state.selected_thread_topic_index, 1);
    }

    #[test]
    fn selected_thread_topic_click_is_handled_without_reopening() {
        let mut state = AppState::new();
        state.thread_topics_area = Rect::new(30, 5, 60, 3);
        state.thread_topics = vec![
            thread_topic(101, "General"),
            thread_topic(102, "Deployments"),
        ];

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    32,
                    6
                )
            ),
            MouseClickOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert_eq!(state.selected_thread_topic_index, 0);
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
    fn right_click_opens_stable_context_targets_and_menu_blocks_wheel() {
        let mut state = AppState::new();
        state.screen_area = Rect::new(0, 0, 80, 24);
        state.chats_area = Rect::new(0, 5, 30, 8);
        state.messages_area = Rect::new(30, 5, 50, 8);
        state.chats = (1..=5).map(|id| chat(id, &format!("Chat {id}"))).collect();
        state.messages = vec![message(1), message(2)];

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(MouseEventKind::Down(MouseButton::Right), 2, 8)
            ),
            MouseClickOutcome::Handled
        );
        assert_eq!(state.selected_chat_index, 0);
        assert!(matches!(
            state.context_menu().map(|menu| menu.target),
            Some(ContextMenuTarget::Chat { chat_id: 2 })
        ));

        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollDown, 2, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.chat_scroll_offset, 0);

        let outside_click = mouse(MouseEventKind::Down(MouseButton::Left), 79, 0);
        assert_eq!(
            handle_mouse_scroll(&mut state, outside_click),
            MouseScrollOutcome::Ignored
        );
        assert_eq!(
            handle_mouse_click(&mut state, outside_click),
            MouseClickOutcome::Handled
        );
        assert!(state.context_menu().is_none());
        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(MouseEventKind::Down(MouseButton::Right), 29, 6)
            ),
            MouseClickOutcome::Ignored
        );
        assert!(state.context_menu().is_none());

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(MouseEventKind::Down(MouseButton::Right), 32, 7)
            ),
            MouseClickOutcome::Handled
        );
        assert_eq!(state.selected_message_index, 1);
        assert!(matches!(
            state.context_menu().map(|menu| menu.target),
            Some(ContextMenuTarget::Message {
                chat_id: 7,
                message_id: 2
            })
        ));
    }

    #[test]
    fn divider_drag_exclusively_captures_mouse_until_release() {
        let mut state = AppState::new();
        state.screen_area = Rect::new(0, 0, 100, 24);
        state.chats_area = Rect::new(0, 5, 30, 8);
        state.messages_area = Rect::new(30, 5, 70, 8);
        state.chats = (1..=5).map(|id| chat(id, &format!("Chat {id}"))).collect();

        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(MouseEventKind::Down(MouseButton::Left), 29, 6)
            ),
            MouseClickOutcome::Handled
        );
        assert!(state.split_drag_active);

        assert_eq!(
            handle_mouse_scroll(&mut state, mouse(MouseEventKind::ScrollDown, 2, 6)),
            MouseScrollOutcome::Handled
        );
        assert_eq!(state.chat_scroll_offset, 0);
        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(MouseEventKind::Down(MouseButton::Right), 2, 6)
            ),
            MouseClickOutcome::Handled
        );
        assert!(state.context_menu().is_none());

        handle_mouse_click(
            &mut state,
            mouse(MouseEventKind::Drag(MouseButton::Left), 29, 6),
        );
        assert_eq!(state.split_ratio, crate::state::DEFAULT_SPLIT_RATIO);

        let drag = mouse(MouseEventKind::Drag(MouseButton::Left), 60, 6);
        assert_eq!(
            handle_mouse_scroll(&mut state, drag),
            MouseScrollOutcome::Ignored
        );
        handle_mouse_click(&mut state, drag);
        assert_eq!(state.split_ratio, 0.6);
        let release = mouse(MouseEventKind::Up(MouseButton::Left), 120, 30);
        assert_eq!(
            handle_mouse_scroll(&mut state, release),
            MouseScrollOutcome::Ignored
        );
        handle_mouse_click(&mut state, release);
        assert!(!state.split_drag_active);
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
    fn message_border_click_does_not_open_visible_url() {
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
                    5
                )
            ),
            MouseClickOutcome::Ignored
        );
        assert_eq!(state.selected_message_index, 0);
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
    fn reply_preview_row_does_not_open_url_from_hidden_body_line() {
        let mut state = AppState::new();
        state.messages_area = Rect::new(30, 5, 35, 4);
        let mut reply = message_with_content(1, "prefix https://example.org/path");
        reply.reply_to_content = Some("visible reply preview".to_string());
        state.messages = vec![reply];

        let hidden_url_column = 30 + 1 + 2 + 4 + 2;
        assert_eq!(
            handle_mouse_click(
                &mut state,
                mouse(
                    MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    hidden_url_column,
                    7
                )
            ),
            MouseClickOutcome::Handled
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
