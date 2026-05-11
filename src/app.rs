use crate::state::AppState;
use std::path::PathBuf;

pub struct TerminalImageRenderCache {
    pub path: PathBuf,
    pub column: u16,
    pub row: u16,
    pub requested_columns: u16,
    pub requested_rows: u16,
    pub columns: u16,
    pub rows: u16,
    pub sequence: String,
    pub byte_len: usize,
    pub source_format: &'static str,
}

pub struct App {
    pub state: AppState,
    pub should_quit: bool,
    pub preferences_path: Option<PathBuf>,
    pub terminal_image_diagnostic_key: Option<String>,
    pub terminal_image_render_cache: Option<TerminalImageRenderCache>,
    pub terminal_image_visible_key: Option<String>,
}

impl App {
    pub fn new() -> Self {
        Self {
            state: AppState::new(),
            should_quit: false,
            preferences_path: None,
            terminal_image_diagnostic_key: None,
            terminal_image_render_cache: None,
            terminal_image_visible_key: None,
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
