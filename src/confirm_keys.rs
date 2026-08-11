use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKeyOutcome {
    Confirm,
    Cancel,
    Ignored,
}

pub fn handle_confirm_key(key: KeyEvent) -> ConfirmKeyOutcome {
    match key.code {
        KeyCode::Char('y') if key.modifiers == KeyModifiers::NONE => ConfirmKeyOutcome::Confirm,
        KeyCode::Char('Y') if matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            ConfirmKeyOutcome::Confirm
        }
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
    fn confirm_keys_accept_plain_yes_and_shift_only_uppercase_yes() {
        assert_eq!(
            handle_confirm_key(key(KeyCode::Char('y'))),
            ConfirmKeyOutcome::Confirm
        );
        assert_eq!(
            handle_confirm_key(key(KeyCode::Char('Y'))),
            ConfirmKeyOutcome::Confirm
        );
        assert_eq!(
            handle_confirm_key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT,)),
            ConfirmKeyOutcome::Confirm
        );
    }

    #[test]
    fn confirm_keys_reject_modified_yes_chords() {
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::META,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            assert_eq!(
                handle_confirm_key(KeyEvent::new(KeyCode::Char('y'), modifiers)),
                ConfirmKeyOutcome::Ignored
            );
            assert_eq!(
                handle_confirm_key(KeyEvent::new(KeyCode::Char('Y'), modifiers)),
                ConfirmKeyOutcome::Ignored
            );
        }
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
