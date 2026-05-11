use crate::state::{AppState, FocusedPanel};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderKeyOutcome {
    Handled,
    OpenPreviousFolder,
    OpenNextFolder,
    Ignored,
}

pub fn handle_folder_key(state: &mut AppState, key: KeyEvent) -> FolderKeyOutcome {
    if state.focused_panel != FocusedPanel::Folders {
        return FolderKeyOutcome::Ignored;
    }

    match key.code {
        KeyCode::Down => {
            state.focused_panel = FocusedPanel::Chats;
            FolderKeyOutcome::Handled
        }
        KeyCode::Up => FolderKeyOutcome::Handled,
        KeyCode::Left if state.folders.len() > 1 => FolderKeyOutcome::OpenPreviousFolder,
        KeyCode::Right if state.folders.len() > 1 => FolderKeyOutcome::OpenNextFolder,
        KeyCode::Left | KeyCode::Right => FolderKeyOutcome::Handled,
        _ => FolderKeyOutcome::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::{FolderKeyOutcome, handle_folder_key};
    use crate::{
        state::{AppState, FocusedPanel},
        telegram::types::{Folder, all_folder},
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn folder(id: i32, name: &str) -> Folder {
        Folder {
            id,
            name: name.to_string(),
            unread_count: 0,
        }
    }

    #[test]
    fn folder_keys_move_focus_down_to_chats() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Folders;

        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Down)),
            FolderKeyOutcome::Handled
        );

        assert_eq!(state.focused_panel, FocusedPanel::Chats);
    }

    #[test]
    fn folder_keys_request_previous_or_next_folder_when_alternates_exist() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Folders;
        state.folders = vec![all_folder(0), folder(2, "Work")];

        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Left)),
            FolderKeyOutcome::OpenPreviousFolder
        );
        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Right)),
            FolderKeyOutcome::OpenNextFolder
        );
    }

    #[test]
    fn folder_keys_handle_previous_or_next_when_no_alternate_folder_exists() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Folders;

        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Left)),
            FolderKeyOutcome::Handled
        );
        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Right)),
            FolderKeyOutcome::Handled
        );

        state.folders = vec![all_folder(0)];
        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Left)),
            FolderKeyOutcome::Handled
        );
        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Right)),
            FolderKeyOutcome::Handled
        );
    }

    #[test]
    fn folder_keys_handle_up_as_noop_at_top() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Folders;

        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Up)),
            FolderKeyOutcome::Handled
        );

        assert_eq!(state.focused_panel, FocusedPanel::Folders);
    }

    #[test]
    fn folder_keys_ignore_other_panels_and_unmapped_keys() {
        let mut state = AppState::new();
        state.focused_panel = FocusedPanel::Chats;

        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Down)),
            FolderKeyOutcome::Ignored
        );

        state.focused_panel = FocusedPanel::Folders;
        assert_eq!(
            handle_folder_key(&mut state, key(KeyCode::Char('x'))),
            FolderKeyOutcome::Ignored
        );
    }
}
