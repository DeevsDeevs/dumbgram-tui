use crate::state::AppState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKeyOutcome {
    Handled,
    Quit,
    Ignored,
}

pub fn handle_app_key(state: &mut AppState, key: KeyEvent) -> AppKeyOutcome {
    let plain_or_shift = matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => AppKeyOutcome::Quit,
        KeyCode::Char('<') if plain_or_shift => {
            state.adjust_split_left();
            AppKeyOutcome::Handled
        }
        KeyCode::Char('>') if plain_or_shift => {
            state.adjust_split_right();
            AppKeyOutcome::Handled
        }
        KeyCode::Char('?') if plain_or_shift => {
            state.toggle_help_bar();
            AppKeyOutcome::Handled
        }
        _ => AppKeyOutcome::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::{AppKeyOutcome, handle_app_key};
    use crate::state::AppState;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn app_keys_report_quit_without_mutating_state() {
        let mut state = AppState::new();
        let split_ratio = state.split_ratio;

        assert_eq!(
            handle_app_key(&mut state, key(KeyCode::Char('q'))),
            AppKeyOutcome::Quit
        );

        assert_eq!(state.split_ratio, split_ratio);
    }

    #[test]
    fn app_keys_toggle_help_bar() {
        let mut state = AppState::new();

        assert!(state.show_help_bar);
        assert_eq!(
            handle_app_key(&mut state, key(KeyCode::Char('?'))),
            AppKeyOutcome::Handled
        );
        assert!(!state.show_help_bar);
        assert_eq!(
            handle_app_key(&mut state, key(KeyCode::Char('?'))),
            AppKeyOutcome::Handled
        );
        assert!(state.show_help_bar);
    }

    #[test]
    fn app_keys_resize_split_left_or_right() {
        let mut state = AppState::new();
        let initial = state.split_ratio;

        assert_eq!(
            handle_app_key(&mut state, key(KeyCode::Char('<'))),
            AppKeyOutcome::Handled
        );
        assert!(state.split_ratio < initial);

        assert_eq!(
            handle_app_key(&mut state, key(KeyCode::Char('>'))),
            AppKeyOutcome::Handled
        );
        assert_eq!(state.split_ratio, initial);
    }

    #[test]
    fn app_keys_accept_shift_reported_glyphs_and_reject_modified_commands() {
        let mut state = AppState::new();
        let initial = state.split_ratio;

        assert_eq!(
            handle_app_key(
                &mut state,
                KeyEvent::new(KeyCode::Char('<'), KeyModifiers::SHIFT),
            ),
            AppKeyOutcome::Handled
        );
        assert!(state.split_ratio < initial);
        assert_eq!(
            handle_app_key(
                &mut state,
                KeyEvent::new(KeyCode::Char('>'), KeyModifiers::SHIFT),
            ),
            AppKeyOutcome::Handled
        );
        assert_eq!(state.split_ratio, initial);
        assert_eq!(
            handle_app_key(
                &mut state,
                KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
            ),
            AppKeyOutcome::Handled
        );

        for (character, modifiers) in [
            ('q', KeyModifiers::CONTROL),
            ('q', KeyModifiers::ALT),
            ('?', KeyModifiers::META),
            ('<', KeyModifiers::SUPER),
            ('>', KeyModifiers::SHIFT | KeyModifiers::CONTROL),
        ] {
            assert_eq!(
                handle_app_key(
                    &mut state,
                    KeyEvent::new(KeyCode::Char(character), modifiers),
                ),
                AppKeyOutcome::Ignored
            );
        }
    }

    #[test]
    fn app_keys_ignore_unmapped_keys() {
        let mut state = AppState::new();

        assert_eq!(
            handle_app_key(&mut state, key(KeyCode::Char('x'))),
            AppKeyOutcome::Ignored
        );
    }
}
