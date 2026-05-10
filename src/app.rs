use crate::state::AppState;
use std::path::PathBuf;

pub struct App {
    pub state: AppState,
    pub should_quit: bool,
    pub preferences_path: Option<PathBuf>,
    pub terminal_image_diagnostic_key: Option<String>,
}

impl App {
    pub fn new() -> Self {
        Self {
            state: AppState::new(),
            should_quit: false,
            preferences_path: None,
            terminal_image_diagnostic_key: None,
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
