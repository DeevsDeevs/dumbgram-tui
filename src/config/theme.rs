use ratatui::style::Color;

pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub selection: Color,
    pub unread_chat: Color,
    pub selected_item: Color,
    pub own_message: Color,
    pub other_message: Color,
    pub error: Color,
    pub success: Color,
    pub border: Color,
    pub border_focused: Color,
}

impl Theme {
    pub fn catppuccin_mocha() -> Self {
        Self {
            background: Color::Rgb(30, 30, 46),
            foreground: Color::Rgb(205, 214, 244),
            selection: Color::Rgb(88, 91, 112),
            unread_chat: Color::Rgb(249, 226, 175),
            selected_item: Color::Rgb(203, 166, 247),
            own_message: Color::Rgb(137, 180, 250),
            other_message: Color::Rgb(205, 214, 244),
            error: Color::Rgb(243, 139, 168),
            success: Color::Rgb(166, 227, 161),
            border: Color::Rgb(88, 91, 112),
            border_focused: Color::Rgb(203, 166, 247),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::catppuccin_mocha()
    }
}
