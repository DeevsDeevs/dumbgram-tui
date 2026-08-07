use ratatui::style::Color;

pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub selection: Color,
    pub selection_foreground: Color,
    pub unread_chat: Color,
    pub selected_item: Color,
    pub own_message: Color,
    pub other_message: Color,
    pub error: Color,
    pub border: Color,
    pub border_focused: Color,
}

impl Theme {
    pub fn evergreen() -> Self {
        Self {
            background: Color::Reset,
            foreground: Color::Reset,
            selection: Color::Green,
            selection_foreground: Color::White,
            unread_chat: Color::Yellow,
            selected_item: Color::LightGreen,
            own_message: Color::LightGreen,
            other_message: Color::Reset,
            error: Color::Red,
            border: Color::DarkGray,
            border_focused: Color::LightGreen,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::evergreen()
    }
}

#[cfg(test)]
mod tests {
    use super::Theme;
    use ratatui::style::Color;

    #[test]
    fn evergreen_is_transparent_default_with_explicit_selection_contrast() {
        let theme = Theme::default();

        assert_eq!(theme.background, Color::Reset);
        assert_eq!(theme.foreground, Color::Reset);
        assert_eq!(theme.other_message, Color::Reset);
        assert_eq!(theme.selection, Color::Green);
        assert_eq!(theme.selection_foreground, Color::White);
        assert_eq!(theme.border_focused, Color::LightGreen);
    }
}
