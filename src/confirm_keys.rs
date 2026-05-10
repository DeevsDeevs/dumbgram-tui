use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKeyOutcome {
    Confirm,
    Cancel,
    Ignored,
}

pub fn handle_confirm_key(key: KeyEvent) -> ConfirmKeyOutcome {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmKeyOutcome::Confirm,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            ConfirmKeyOutcome::Cancel
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ConfirmKeyOutcome::Cancel,
        _ => ConfirmKeyOutcome::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfirmKeyOutcome, handle_confirm_key};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn confirm_keys_accept_upper_or_lower_yes() {
        assert_eq!(
            handle_confirm_key(key(KeyCode::Char('y'))),
            ConfirmKeyOutcome::Confirm
        );
        assert_eq!(
            handle_confirm_key(key(KeyCode::Char('Y'))),
            ConfirmKeyOutcome::Confirm
        );
    }

    #[test]
    fn confirm_keys_cancel_with_no_escape_or_ctrl_c() {
        assert_eq!(
            handle_confirm_key(key(KeyCode::Char('n'))),
            ConfirmKeyOutcome::Cancel
        );
        assert_eq!(
            handle_confirm_key(key(KeyCode::Char('N'))),
            ConfirmKeyOutcome::Cancel
        );
        assert_eq!(
            handle_confirm_key(key(KeyCode::Esc)),
            ConfirmKeyOutcome::Cancel
        );
        assert_eq!(handle_confirm_key(ctrl('c')), ConfirmKeyOutcome::Cancel);
    }

    #[test]
    fn confirm_keys_ignore_unrelated_keys() {
        assert_eq!(
            handle_confirm_key(key(KeyCode::Enter)),
            ConfirmKeyOutcome::Ignored
        );
        assert_eq!(
            handle_confirm_key(key(KeyCode::Char('x'))),
            ConfirmKeyOutcome::Ignored
        );
    }
}
