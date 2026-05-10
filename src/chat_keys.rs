use crate::state::{AppState, FocusedPanel};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKeyOutcome {
    Handled,
    OpenNextChat,
    OpenChatAt(usize),
    Ignored,
}

pub fn handle_chat_key(state: &mut AppState, key: KeyEvent) -> ChatKeyOutcome {
    if state.focused_panel != FocusedPanel::Chats {
        return ChatKeyOutcome::Ignored;
    }

    match key.code {
        KeyCode::Down => ChatKeyOutcome::OpenNextChat,
        KeyCode::Up => {
            if state.chats.is_empty() || state.selected_chat_index == 0 {
                state.focused_panel = FocusedPanel::Folders;
                ChatKeyOutcome::Handled
            } else {
                ChatKeyOutcome::OpenChatAt(state.selected_chat_index - 1)
            }
        }
        KeyCode::Left => {
            state.focused_panel = FocusedPanel::Folders;
            ChatKeyOutcome::Handled
        }
        KeyCode::Right => {
            state.focused_panel = FocusedPanel::Messages;
            ChatKeyOutcome::Handled
        }
        _ => ChatKeyOutcome::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatKeyOutcome, handle_chat_key};
    use crate::state::{AppState, FocusedPanel};
    use crate::telegram::types::Chat;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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

    #[test]
    fn chat_keys_request_opening_next_or_previous_chat() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;
        state.chats = vec![chat(1), chat(2), chat(3)];
        state.selected_chat_index = 2;

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Down)),
            ChatKeyOutcome::OpenNextChat
        );
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Up)),
            ChatKeyOutcome::OpenChatAt(1)
        );
    }

    #[test]
    fn chat_keys_move_focus_between_neighboring_panels() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Left)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Folders);

        state.focused_panel = FocusedPanel::Chats;
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Right)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
    }

    #[test]
    fn chat_keys_move_up_to_folders_at_top_or_empty_list() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;
        state.chats = vec![chat(1)];
        state.selected_chat_index = 0;

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Up)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Folders);

        state.focused_panel = FocusedPanel::Chats;
        state.chats.clear();
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Up)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Folders);
    }

    #[test]
    fn chat_keys_ignore_other_panels_and_unmapped_keys() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Messages;

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Down)),
            ChatKeyOutcome::Ignored
        );

        state.focused_panel = FocusedPanel::Chats;
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('x'))),
            ChatKeyOutcome::Ignored
        );
    }
}
