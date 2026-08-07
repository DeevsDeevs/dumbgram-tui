use crate::state::{AppState, FocusedPanel};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    if state.chat_search_active() {
        return match key.code {
            KeyCode::Esc => {
                state.clear_chat_search();
                ChatKeyOutcome::Handled
            }
            KeyCode::Enter => {
                let selected_match = state.selected_chat_search_result_index();
                state.clear_chat_search();
                selected_match.map_or(ChatKeyOutcome::Handled, ChatKeyOutcome::OpenChatAt)
            }
            KeyCode::Backspace => {
                state.pop_chat_search_char();
                ChatKeyOutcome::Handled
            }
            KeyCode::Down => {
                state.select_next_chat_search_match();
                ChatKeyOutcome::Handled
            }
            KeyCode::Up => {
                state.select_previous_chat_search_match();
                ChatKeyOutcome::Handled
            }
            KeyCode::Char(ch) if key.modifiers == KeyModifiers::NONE => {
                state.push_chat_search_char(ch);
                ChatKeyOutcome::Handled
            }
            _ => ChatKeyOutcome::Handled,
        };
    }

    match key.code {
        KeyCode::Char('/') if key.modifiers == KeyModifiers::NONE => {
            state.begin_chat_search();
            ChatKeyOutcome::Handled
        }
        KeyCode::Down
            if state
                .selected_chat_index
                .checked_add(1)
                .is_some_and(|next_index| next_index < state.chats.len()) =>
        {
            ChatKeyOutcome::OpenNextChat
        }
        KeyCode::Down => ChatKeyOutcome::Handled,
        KeyCode::Up if state.chats.is_empty() || state.selected_chat_index == 0 => {
            ChatKeyOutcome::Handled
        }
        KeyCode::Up => ChatKeyOutcome::OpenChatAt(state.selected_chat_index - 1),
        KeyCode::Left => {
            state.focused_panel = FocusedPanel::Folders;
            ChatKeyOutcome::Handled
        }
        KeyCode::Right => {
            state.focused_panel = FocusedPanel::Messages;
            ChatKeyOutcome::Handled
        }
        KeyCode::Char(prefix)
            if key.modifiers == KeyModifiers::NONE && !prefix.eq_ignore_ascii_case(&'q') =>
        {
            state
                .next_chat_index_starting_with(prefix)
                .map_or(ChatKeyOutcome::Ignored, ChatKeyOutcome::OpenChatAt)
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
        named_chat(id, &format!("Chat {id}"))
    }

    fn named_chat(id: i64, name: &str) -> Chat {
        Chat {
            id,
            name: name.to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }
    }

    #[test]
    fn chat_keys_request_opening_next_or_previous_chat_without_wrapping() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;
        state.chats = vec![chat(1), chat(2), chat(3)];
        state.selected_chat_index = 1;

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Down)),
            ChatKeyOutcome::OpenNextChat
        );
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Up)),
            ChatKeyOutcome::OpenChatAt(0)
        );

        state.selected_chat_index = 2;
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Down)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
    }

    #[test]
    fn chat_keys_handle_down_when_no_alternate_chat_exists() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Down)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);

        state.chats = vec![chat(1)];
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Down)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
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
    fn chat_keys_stop_at_top_or_empty_list_without_changing_focus() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;
        state.chats = vec![chat(1)];
        state.selected_chat_index = 0;

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Up)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);

        state.chats.clear();
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Up)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
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

    #[test]
    fn chat_keys_search_loaded_chats_by_name_before_opening() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;
        state.chats = vec![
            named_chat(1, "Alice Personal"),
            named_chat(2, "Work Team"),
            named_chat(3, "Project Alpha"),
        ];

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('/'))),
            ChatKeyOutcome::Handled
        );
        assert!(state.chat_search_active());

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('p'))),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.chat_search_query(), "p");
        assert_eq!(state.selected_chat_index, 0);

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('r'))),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.chat_search_query(), "pr");
        assert_eq!(state.selected_chat_index, 0);

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('o'))),
            ChatKeyOutcome::Handled
        );
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('j'))),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.chat_search_query(), "proj");
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.selected_chat_search_result_index(), Some(2));

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Enter)),
            ChatKeyOutcome::OpenChatAt(2)
        );
        assert!(!state.chat_search_active());
    }

    #[test]
    fn chat_search_arrows_browse_without_opening_selected_result() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;
        state.chats = vec![
            named_chat(1, "Alpha"),
            named_chat(2, "Beta"),
            named_chat(3, "Gamma"),
        ];
        state.begin_chat_search();

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Down)),
            ChatKeyOutcome::Handled
        );
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.selected_chat_search_result_index(), Some(1));
        assert!(state.chat_search_active());

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Enter)),
            ChatKeyOutcome::OpenChatAt(1)
        );
        assert!(!state.chat_search_active());
    }

    #[test]
    fn chat_search_enter_with_no_matches_does_not_open_stale_selection() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;
        state.chats = vec![named_chat(1, "Alpha"), named_chat(2, "Beta")];
        state.selected_chat_index = 1;
        state.begin_chat_search();
        for ch in "zzz".chars() {
            assert_eq!(
                handle_chat_key(&mut state, key(KeyCode::Char(ch))),
                ChatKeyOutcome::Handled
            );
        }
        assert!(state.chat_display_indices().is_empty());

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Enter)),
            ChatKeyOutcome::Handled
        );
        assert!(!state.chat_search_active());
        assert_eq!(state.selected_chat_index, 1);
    }

    #[test]
    fn chat_keys_jump_to_next_chat_by_first_letter_without_intercepting_quit() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;
        state.chats = vec![
            named_chat(1, "General"),
            named_chat(2, "Random"),
            named_chat(3, "Release"),
        ];
        state.selected_chat_index = 0;

        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('r'))),
            ChatKeyOutcome::OpenChatAt(1)
        );

        state.selected_chat_index = 1;
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('r'))),
            ChatKeyOutcome::OpenChatAt(2)
        );

        state.selected_chat_index = 2;
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('r'))),
            ChatKeyOutcome::OpenChatAt(1)
        );
        assert_eq!(
            handle_chat_key(&mut state, key(KeyCode::Char('q'))),
            ChatKeyOutcome::Ignored
        );
    }
}
