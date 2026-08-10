use crate::state::{AppState, FocusedPanel};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKeyOutcome {
    Handled,
    OpenSelectedThreadTopic,
    OpenSelectedLink,
    CopySelectedText,
    DownloadSelectedMedia,
    OpenDownloadedMedia,
    Ignored,
}

pub fn handle_message_key(state: &mut AppState, key: KeyEvent) -> MessageKeyOutcome {
    if state.focused_panel != FocusedPanel::Messages {
        return MessageKeyOutcome::Ignored;
    }

    match key.code {
        KeyCode::PageDown => state.page_messages_down(),
        KeyCode::PageUp => state.page_messages_up(),
        KeyCode::Home => state.select_first_message(),
        KeyCode::End => state.select_last_message(),
        KeyCode::Down => state.select_next_message(),
        KeyCode::Up => state.select_prev_message(),
        KeyCode::Left if state.thread_topics.is_empty() => {
            state.focused_panel = FocusedPanel::Chats
        }
        KeyCode::Left => {
            state.select_prev_thread_topic();
            return MessageKeyOutcome::OpenSelectedThreadTopic;
        }
        KeyCode::Right if state.thread_topics.is_empty() => {}
        KeyCode::Right => {
            state.select_next_thread_topic();
            return MessageKeyOutcome::OpenSelectedThreadTopic;
        }
        KeyCode::Enter => state.focused_panel = FocusedPanel::Input,
        KeyCode::Char('[') => {
            state.select_prev_thread_topic();
            return MessageKeyOutcome::OpenSelectedThreadTopic;
        }
        KeyCode::Char(']') => {
            state.select_next_thread_topic();
            return MessageKeyOutcome::OpenSelectedThreadTopic;
        }
        KeyCode::Char('e') => state.request_edit_selected_message(),
        KeyCode::Char('r') => state.request_reply_to_selected_message(),
        KeyCode::Char('d') => state.request_delete_selected_message(),
        KeyCode::Char('o') => return MessageKeyOutcome::OpenSelectedLink,
        KeyCode::Char('c') => return MessageKeyOutcome::CopySelectedText,
        KeyCode::Char('s') => return MessageKeyOutcome::DownloadSelectedMedia,
        KeyCode::Char('v') => return MessageKeyOutcome::OpenDownloadedMedia,
        _ => return MessageKeyOutcome::Ignored,
    }

    MessageKeyOutcome::Handled
}

#[cfg(test)]
mod tests {
    use super::{MessageKeyOutcome, handle_message_key};
    use crate::state::{AppState, FocusedPanel};
    use crate::telegram::types::{Message, MessageStatus, ThreadTopic};
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn message(id: i32) -> Message {
        Message {
            id,
            chat_id: 7,
            thread_topic_id: None,
            sender_identity: None,
            sender_name: "me".to_string(),
            content: format!("message {id}"),
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

    #[test]
    fn message_keys_move_selection_and_stop_at_bottom() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Messages;
        state.messages = vec![message(1), message(2)];

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Down)),
            MessageKeyOutcome::Handled
        );
        assert_eq!(state.selected_message_index, 1);
        assert_eq!(state.focused_panel, FocusedPanel::Messages);

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Down)),
            MessageKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert_eq!(state.selected_message_index, 1);
    }

    #[test]
    fn message_keys_trigger_selected_message_actions() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Messages;
        state.messages = vec![message(42)];

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('e'))),
            MessageKeyOutcome::Handled
        );
        assert_eq!(state.editing_message_id, Some(42));

        state.cancel_compose_mode();
        state.focused_panel = FocusedPanel::Messages;
        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('r'))),
            MessageKeyOutcome::Handled
        );
        assert_eq!(state.replying_to_message_id, Some(42));

        state.cancel_compose_mode();
        state.focused_panel = FocusedPanel::Messages;
        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('d'))),
            MessageKeyOutcome::Handled
        );
        assert!(state.delete_confirmation().is_some_and(|confirmation| {
            confirmation.chat_id == 7 && confirmation.message_id == 42
        }));
    }

    #[test]
    fn message_keys_select_thread_topics() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Messages;
        state.thread_topics = vec![
            ThreadTopic {
                id: 101,
                title: "General".to_string(),
                top_message_id: 101,
                unread_count: 1,
                is_closed: false,
                is_pinned: true,
            },
            ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 102,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
        ];

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Right)),
            MessageKeyOutcome::OpenSelectedThreadTopic
        );
        assert_eq!(state.selected_thread_topic_index, 1);

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Left)),
            MessageKeyOutcome::OpenSelectedThreadTopic
        );
        assert_eq!(state.selected_thread_topic_index, 0);

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Enter)),
            MessageKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        assert_eq!(state.selected_thread_topic_index, 0);

        state.focused_panel = FocusedPanel::Messages;
        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char(']'))),
            MessageKeyOutcome::OpenSelectedThreadTopic
        );
        assert_eq!(state.selected_thread_topic_index, 1);
        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('['))),
            MessageKeyOutcome::OpenSelectedThreadTopic
        );
        assert_eq!(state.selected_thread_topic_index, 0);
    }

    #[test]
    fn message_enter_focuses_input_without_thread_topics() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Messages;

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Enter)),
            MessageKeyOutcome::Handled
        );
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    #[test]
    fn message_keys_ignore_other_panels_and_unmapped_keys() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Down)),
            MessageKeyOutcome::Ignored
        );

        state.focused_panel = FocusedPanel::Messages;
        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('o'))),
            MessageKeyOutcome::OpenSelectedLink
        );

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('c'))),
            MessageKeyOutcome::CopySelectedText
        );

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('s'))),
            MessageKeyOutcome::DownloadSelectedMedia
        );

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('v'))),
            MessageKeyOutcome::OpenDownloadedMedia
        );

        assert_eq!(
            handle_message_key(&mut state, key(KeyCode::Char('x'))),
            MessageKeyOutcome::Ignored
        );
    }
}
