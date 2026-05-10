pub mod chats;
pub mod folders;
pub mod input;
pub mod layout;
pub mod messages;

pub use chats::render_chats;
pub use folders::render_folders;
pub use input::render_input;
pub use layout::render_layout;
pub use messages::render_messages;

pub(crate) const SELECTED_ROW_SYMBOL: &str = "▶ ";
pub(crate) const LEGACY_SELECTED_ROW_SYMBOL: &str = ">>";
pub(crate) const LIST_TEXT_WIDTH_RESERVED_COLUMNS: u16 = 5;

pub(crate) fn list_text_width(area_width: u16) -> usize {
    area_width.saturating_sub(LIST_TEXT_WIDTH_RESERVED_COLUMNS) as usize
}

pub(crate) fn selected_list_index(selected_index: usize, item_count: usize) -> Option<usize> {
    if item_count == 0 {
        None
    } else {
        Some(selected_index.min(item_count - 1))
    }
}

#[cfg(test)]
const TEST_RENDER_WIDTH: u16 = 80;
#[cfg(test)]
const TEST_RENDER_HEIGHT: u16 = 24;

#[cfg(test)]
pub(crate) fn render_app_to_string_for_test(app: &mut crate::app::App) -> String {
    let theme = crate::config::Theme::default();
    let backend = ratatui::backend::TestBackend::new(TEST_RENDER_WIDTH, TEST_RENDER_HEIGHT);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal should initialize");

    terminal
        .draw(|frame| render_layout(frame, app, &theme))
        .expect("layout should render in test backend");

    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::{
        LEGACY_SELECTED_ROW_SYMBOL, LIST_TEXT_WIDTH_RESERVED_COLUMNS, SELECTED_ROW_SYMBOL, chats,
        folders, input, list_text_width, messages, render_app_to_string_for_test,
        selected_list_index,
    };
    use crate::{
        app::App,
        telegram::types::{Chat, Message, MessageStatus},
    };
    use chrono::Utc;

    #[test]
    fn list_text_width_reserves_border_and_selection_columns() {
        assert_eq!(LIST_TEXT_WIDTH_RESERVED_COLUMNS, 5);
        assert_eq!(list_text_width(80), 75);
        assert_eq!(list_text_width(3), 0);
    }

    #[test]
    fn selected_list_index_omits_empty_lists_and_clamps_to_last_item() {
        assert_eq!(selected_list_index(0, 0), None);
        assert_eq!(selected_list_index(0, 3), Some(0));
        assert_eq!(selected_list_index(99, 3), Some(2));
    }

    #[test]
    fn layout_uses_unicode_borders_without_emoji_status_glyphs() {
        let mut app = App::new();
        app.state.chats = vec![Chat {
            id: 1,
            name: "General".to_string(),
            last_message: Some("Latest message".to_string()),
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        app.state.messages = vec![Message {
            id: 10,
            chat_id: 1,
            sender_name: "Alice".to_string(),
            content: "Hello".to_string(),
            timestamp: Utc::now(),
            is_own: false,
            is_edited: false,
            reply_to_content: None,
            status: MessageStatus::Read,
            can_edit: false,
            can_delete: false,
            error: None,
        }];
        let rendered = render_app_to_string_for_test(&mut app);

        for border in ["┌", "┐", "└", "┘", "─", "│"] {
            assert!(rendered.contains(border), "missing border glyph {border}");
        }
        assert!(
            rendered.contains(SELECTED_ROW_SYMBOL),
            "missing selected-row arrow glyph"
        );
        assert!(
            !rendered.contains(LEGACY_SELECTED_ROW_SYMBOL),
            "ASCII selected-row marker should not render"
        );

        for emoji_like in [
            "\u{2705}",
            "\u{2611}",
            "\u{2714}",
            "\u{2713}",
            "\u{274c}",
            "\u{23f1}",
            "\u{1f534}",
            "\u{1f7e2}",
            "\u{1f7e1}",
            "\u{1f7e3}",
            "\u{1f535}",
        ] {
            assert!(
                !rendered.contains(emoji_like),
                "rendered emoji-like glyph {emoji_like}"
            );
        }
    }

    #[test]
    fn layout_renders_empty_state_placeholders() {
        let mut app = App::new();
        let rendered = render_app_to_string_for_test(&mut app);

        for placeholder in [
            folders::FOLDER_EMPTY_LABEL.trim(),
            chats::CHAT_EMPTY_NO_FOLDER_LABEL,
            messages::MESSAGE_EMPTY_NO_CHAT_LABEL,
            input::INPUT_EMPTY_PLACEHOLDER_LABEL,
        ] {
            assert!(
                rendered.contains(placeholder),
                "missing empty-state placeholder {placeholder}"
            );
        }
    }
}
