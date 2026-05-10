use crate::state::{AppState, FocusedPanel};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalKeyOutcome {
    Handled,
    Continue,
}

pub fn handle_global_key(state: &mut AppState, key: KeyEvent) -> GlobalKeyOutcome {
    if key.code == KeyCode::Tab && state.delete_confirmation.is_none() {
        state.focus_next_panel();
        return GlobalKeyOutcome::Handled;
    }

    if key.code == KeyCode::Esc
        && state.focused_panel != FocusedPanel::Input
        && (state.editing_message_id.is_some() || state.replying_to_message_id.is_some())
    {
        state.cancel_compose_mode();
        return GlobalKeyOutcome::Handled;
    }

    GlobalKeyOutcome::Continue
}

#[cfg(test)]
mod tests {
    use super::{GlobalKeyOutcome, handle_global_key};
    use crate::state::{AppState, DeleteConfirmation, FocusedPanel};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn tab_cycles_focus_when_no_delete_confirmation_is_open() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Folders;

        assert_eq!(
            handle_global_key(&mut state, key(KeyCode::Tab)),
            GlobalKeyOutcome::Handled
        );

        assert_eq!(state.focused_panel, FocusedPanel::Chats);
    }

    #[test]
    fn tab_is_left_for_confirmation_prompt_when_delete_confirmation_is_open() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Folders;
        state.delete_confirmation = Some(DeleteConfirmation {
            chat_id: 1,
            message_id: 2,
        });

        assert_eq!(
            handle_global_key(&mut state, key(KeyCode::Tab)),
            GlobalKeyOutcome::Continue
        );

        assert_eq!(state.focused_panel, FocusedPanel::Folders);
    }

    #[test]
    fn escape_cancels_compose_mode_outside_input() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Messages;
        state.editing_message_id = Some(42);
        state.input_buffer = "edited".to_string();

        assert_eq!(
            handle_global_key(&mut state, key(KeyCode::Esc)),
            GlobalKeyOutcome::Handled
        );

        assert!(state.editing_message_id.is_none());
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn escape_continues_to_input_handler_when_input_is_focused() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Input;
        state.editing_message_id = Some(42);

        assert_eq!(
            handle_global_key(&mut state, key(KeyCode::Esc)),
            GlobalKeyOutcome::Continue
        );

        assert_eq!(state.editing_message_id, Some(42));
    }
}
