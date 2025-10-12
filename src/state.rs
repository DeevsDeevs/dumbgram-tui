use crate::telegram::types::{Chat, Folder, Message};
use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    Folders,
    Chats,
    Messages,
    Input,
}

pub struct AppState {
    pub folders: Vec<Folder>,
    pub chats: Vec<Chat>,
    pub messages: Vec<Message>,
    pub selected_folder_index: usize,
    pub selected_chat_index: usize,
    pub selected_message_index: usize,
    pub focused_panel: FocusedPanel,
    pub input_buffer: String,
    pub split_ratio: f32,
    pub folders_area: Rect,
    pub chats_area: Rect,
    pub messages_area: Rect,
    pub input_area: Rect,
    pub folder_scroll_offset: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            folders: Vec::new(),
            chats: Vec::new(),
            messages: Vec::new(),
            selected_folder_index: 0,
            selected_chat_index: 0,
            selected_message_index: 0,
            focused_panel: FocusedPanel::Folders,
            input_buffer: String::new(),
            split_ratio: 0.3,
            folders_area: Rect::default(),
            chats_area: Rect::default(),
            messages_area: Rect::default(),
            input_area: Rect::default(),
            folder_scroll_offset: 0,
        }
    }
    
    pub fn get_visible_folders(&self) -> (Vec<&Folder>, bool, bool) {
        if self.folders.is_empty() {
            return (Vec::new(), false, false);
        }
        
        let available_width = self.folders_area.width.saturating_sub(4);
        let mut visible_folders = Vec::new();
        let mut current_width = 0u16;
        
        let has_left_scroll = self.folder_scroll_offset > 0;
        let scroll_indicator_width = if has_left_scroll { 3 } else { 0 };
        current_width += scroll_indicator_width;
        
        for folder in self.folders.iter().skip(self.folder_scroll_offset) {
            let folder_width = folder.name.len() as u16 + 5;
            if current_width + folder_width + 3 > available_width {
                return (visible_folders, has_left_scroll, true);
            }
            visible_folders.push(folder);
            current_width += folder_width;
        }
        
        let has_right_scroll = self.folder_scroll_offset + visible_folders.len() < self.folders.len();
        (visible_folders, has_left_scroll, has_right_scroll)
    }
    
    pub fn scroll_folders_left(&mut self) {
        if self.folder_scroll_offset > 0 {
            self.folder_scroll_offset -= 1;
        }
    }
    
    pub fn scroll_folders_right(&mut self) {
        let (visible, _, has_right) = self.get_visible_folders();
        if has_right && self.folder_scroll_offset + visible.len() < self.folders.len() {
            self.folder_scroll_offset += 1;
        }
    }
    
    pub fn ensure_selected_folder_visible(&mut self) {
        if self.selected_folder_index < self.folder_scroll_offset {
            self.folder_scroll_offset = self.selected_folder_index;
        }
        
        let (visible, _, _) = self.get_visible_folders();
        let max_visible_index = self.folder_scroll_offset + visible.len().saturating_sub(1);
        if self.selected_folder_index > max_visible_index {
            self.folder_scroll_offset = self.selected_folder_index.saturating_sub(visible.len().saturating_sub(1));
        }
    }
    
    pub fn select_folder(&mut self, index: usize) {
        if index < self.folders.len() {
            self.selected_folder_index = index;
            self.ensure_selected_folder_visible();
        }
    }
    
    pub fn select_chat(&mut self, index: usize) {
        if index < self.chats.len() {
            self.selected_chat_index = index;
        }
    }

    pub fn select_next_folder(&mut self) {
        if !self.folders.is_empty() {
            self.selected_folder_index = (self.selected_folder_index + 1) % self.folders.len();
        }
    }

    pub fn select_prev_folder(&mut self) {
        if !self.folders.is_empty() {
            self.selected_folder_index = if self.selected_folder_index == 0 {
                self.folders.len() - 1
            } else {
                self.selected_folder_index - 1
            };
        }
    }

    pub fn select_next_chat(&mut self) {
        if !self.chats.is_empty() {
            self.selected_chat_index = (self.selected_chat_index + 1) % self.chats.len();
        }
    }

    pub fn select_prev_chat(&mut self) {
        if !self.chats.is_empty() {
            self.selected_chat_index = if self.selected_chat_index == 0 {
                self.chats.len() - 1
            } else {
                self.selected_chat_index - 1
            };
        }
    }

    pub fn select_next_message(&mut self) {
        if !self.messages.is_empty() {
            self.selected_message_index = (self.selected_message_index + 1) % self.messages.len();
        }
    }

    pub fn select_prev_message(&mut self) {
        if !self.messages.is_empty() {
            self.selected_message_index = if self.selected_message_index == 0 {
                self.messages.len() - 1
            } else {
                self.selected_message_index - 1
            };
        }
    }

    pub fn adjust_split_left(&mut self) {
        self.split_ratio = (self.split_ratio - 0.05).max(0.1);
    }

    pub fn adjust_split_right(&mut self) {
        self.split_ratio = (self.split_ratio + 0.05).min(0.9);
    }

    pub fn focus_next_panel(&mut self) {
        self.focused_panel = match self.focused_panel {
            FocusedPanel::Folders => FocusedPanel::Chats,
            FocusedPanel::Chats => FocusedPanel::Messages,
            FocusedPanel::Messages => FocusedPanel::Input,
            FocusedPanel::Input => FocusedPanel::Folders,
        };
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
