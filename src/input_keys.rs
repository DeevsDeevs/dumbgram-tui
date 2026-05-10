use crate::state::AppState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKeyOutcome {
    Handled,
    Submit,
    Ignored,
}

pub fn handle_input_key(state: &mut AppState, key: KeyEvent) -> InputKeyOutcome {
    match key.code {
        KeyCode::Esc => {
            state.cancel_input_mode();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.move_input_cursor_to_start();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.move_input_cursor_left();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.cancel_input_mode();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.delete_input_char();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.move_input_cursor_to_end();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.move_input_cursor_right();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.delete_input_after_cursor();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.delete_input_before_cursor();
            InputKeyOutcome::Handled
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.delete_input_previous_word();
            InputKeyOutcome::Handled
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.insert_input_char(c);
            InputKeyOutcome::Handled
        }
        KeyCode::Backspace => {
            state.backspace_input_char();
            InputKeyOutcome::Handled
        }
        KeyCode::Delete => {
            state.delete_input_char();
            InputKeyOutcome::Handled
        }
        KeyCode::Left => {
            state.move_input_cursor_left();
            InputKeyOutcome::Handled
        }
        KeyCode::Right => {
            state.move_input_cursor_right();
            InputKeyOutcome::Handled
        }
        KeyCode::Home => {
            state.move_input_cursor_to_start();
            InputKeyOutcome::Handled
        }
        KeyCode::End => {
            state.move_input_cursor_to_end();
            InputKeyOutcome::Handled
        }
        KeyCode::Enter => InputKeyOutcome::Submit,
        _ => InputKeyOutcome::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::{InputKeyOutcome, handle_input_key};
    use crate::state::AppState;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn input_key_handler_edits_buffer_and_reports_submit() {
        let mut state = AppState::new();

        assert_eq!(
            handle_input_key(&mut state, key(KeyCode::Char('a'))),
            InputKeyOutcome::Handled
        );
        assert_eq!(
            handle_input_key(&mut state, key(KeyCode::Char('c'))),
            InputKeyOutcome::Handled
        );
        assert_eq!(
            handle_input_key(&mut state, key(KeyCode::Left)),
            InputKeyOutcome::Handled
        );
        assert_eq!(
            handle_input_key(&mut state, key(KeyCode::Char('b'))),
            InputKeyOutcome::Handled
        );
        assert_eq!(
            handle_input_key(&mut state, key(KeyCode::Enter)),
            InputKeyOutcome::Submit
        );

        assert_eq!(state.input_buffer, "abc");
        assert_eq!(state.input_cursor(), 2);
    }

    #[test]
    fn input_key_handler_ignores_unmapped_control_chars() {
        let mut state = AppState::new();

        assert_eq!(
            handle_input_key(&mut state, ctrl('x')),
            InputKeyOutcome::Ignored
        );

        assert!(state.input_buffer.is_empty());
    }
}
