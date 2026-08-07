pub mod chats;
pub mod folders;
pub mod input;
pub mod layout;
pub mod messages;

pub use chats::render_chats;
pub use folders::render_folders;
pub use input::render_input;
pub use layout::render_layout;
pub use messages::{render_messages, render_thread_topics};

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
pub(crate) fn render_app_to_buffer_for_test(app: &mut crate::app::App) -> ratatui::buffer::Buffer {
    let theme = crate::config::Theme::default();
    let backend = ratatui::backend::TestBackend::new(TEST_RENDER_WIDTH, TEST_RENDER_HEIGHT);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal should initialize");

    terminal
        .draw(|frame| render_layout(frame, app, &theme))
        .expect("layout should render in test backend");

    terminal.backend().buffer().clone()
}

#[cfg(test)]
pub(crate) fn render_app_to_string_for_test(app: &mut crate::app::App) -> String {
    render_app_to_buffer_for_test(app)
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::{
        LEGACY_SELECTED_ROW_SYMBOL, LIST_TEXT_WIDTH_RESERVED_COLUMNS, SELECTED_ROW_SYMBOL, chats,
        folders, input, list_text_width, messages, render_app_to_buffer_for_test,
        render_app_to_string_for_test, selected_list_index,
    };
    use crate::{
        app::App,
        state::{ContextMenuTarget, DeleteConfirmation},
        telegram::types::{Chat, Message, MessageStatus, ThreadTopic},
    };
    use chrono::Utc;
    use ratatui::{
        layout::{Constraint, Direction, Layout},
        style::Color,
    };

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

    fn app_with_selected_message(content: String) -> App {
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
            thread_topic_id: None,
            sender_name: "Alice".to_string(),
            content,
            timestamp: Utc::now(),
            is_own: false,
            is_edited: false,
            reply_to_content: None,
            media: None,
            status: MessageStatus::Read,
            can_edit: false,
            can_delete: false,
            error: None,
        }];
        app
    }

    #[test]
    fn evergreen_render_keeps_base_transparent_and_selection_contrasted() {
        let mut app = app_with_selected_message("Hello".to_string());
        let buffer = render_app_to_buffer_for_test(&mut app);
        let mut selected_cells = 0;

        for cell in &buffer.content {
            assert!(
                matches!(cell.bg, Color::Reset | Color::DarkGray),
                "unexpected opaque background {:?}",
                cell.bg
            );
            if cell.bg == Color::DarkGray {
                selected_cells += 1;
                assert_eq!(cell.fg, Color::White);
            }
        }
        assert!(selected_cells > 0);
        for area in [app.state.chats_area, app.state.messages_area] {
            assert!(
                area.rows().flat_map(|row| row.columns()).any(|position| {
                    buffer[position].bg == Color::DarkGray && buffer[position].fg == Color::White
                }),
                "selected row in {area:?} did not use the explicit contrast pair"
            );
        }
    }

    #[test]
    fn production_ui_backgrounds_are_theme_background_or_selection_only() {
        for source in [
            include_str!("chats.rs"),
            include_str!("folders.rs"),
            include_str!("input.rs"),
            include_str!("layout.rs"),
            include_str!("messages.rs"),
        ] {
            for line in source.lines().filter(|line| line.contains(".bg(")) {
                assert!(
                    line.contains("theme.background") || line.contains("theme.selection"),
                    "unexpected production background style: {line}"
                );
            }
        }
    }

    #[test]
    fn delete_popup_clear_prevents_underlying_message_bleed() {
        let mut app = app_with_selected_message("UNDERLYING ".repeat(300));
        app.state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 1,
            message_id: 10,
        });
        let buffer = render_app_to_buffer_for_test(&mut app);
        let area = app.state.messages_area;
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(20),
                Constraint::Percentage(40),
            ])
            .split(area);
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .split(vertical[1]);
        let popup = horizontal[1];
        let rendered_popup = popup
            .rows()
            .flat_map(|row| row.columns())
            .map(|position| buffer[position].symbol())
            .collect::<String>();

        assert!(rendered_popup.contains("Delete?"));
        assert!(!rendered_popup.contains("UNDERLYING"));
    }

    #[test]
    fn context_menu_clear_preserves_transparency_and_blocks_underlying_text() {
        let mut app = app_with_selected_message("UNDERLYING ".repeat(300));
        app.state.screen_area = ratatui::layout::Rect::new(0, 0, 80, 24);
        assert!(app.state.open_context_menu(
            ContextMenuTarget::Message {
                chat_id: 1,
                message_id: 10,
            },
            79,
            23,
        ));

        let buffer = render_app_to_buffer_for_test(&mut app);
        let menu = app.state.context_menu_rect().expect("menu should render");
        let rendered = menu
            .rows()
            .flat_map(|row| row.columns())
            .map(|position| buffer[position].symbol())
            .collect::<String>();

        assert!(rendered.contains("Reply"));
        assert!(!rendered.contains("UNDERLYING"));
        for position in menu.rows().flat_map(|row| row.columns()) {
            assert!(matches!(
                buffer[position].bg,
                Color::Reset | Color::DarkGray
            ));
        }
    }

    #[test]
    fn offscreen_active_chat_does_not_force_selection_highlight_into_scrolled_view() {
        let mut app = app_with_selected_message("Hello".to_string());
        app.state.chats.extend((2..=8).map(|id| Chat {
            id,
            name: format!("Chat {id}"),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }));
        app.state.chat_scroll_offset = 3;

        let buffer = render_app_to_buffer_for_test(&mut app);
        assert!(
            app.state
                .chats_area
                .rows()
                .flat_map(|row| row.columns())
                .all(|position| buffer[position].bg != Color::DarkGray)
        );
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
            thread_topic_id: None,
            sender_name: "Alice".to_string(),
            content: "Hello".to_string(),
            timestamp: Utc::now(),
            is_own: false,
            is_edited: false,
            reply_to_content: None,
            media: None,
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
    fn layout_renders_thread_topics_as_tabs_above_messages() {
        let mut app = App::new();
        app.state.chats = vec![Chat {
            id: 3,
            name: "Work Team".to_string(),
            last_message: Some("Latest message".to_string()),
            unread_count: 0,
            is_group: true,
            folder_id: None,
        }];
        app.state.thread_topics = vec![
            ThreadTopic {
                id: 101,
                title: "General".to_string(),
                top_message_id: 101,
                unread_count: 1,
                is_closed: false,
                is_pinned: true,
            },
            ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 102,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
        ];
        app.state.messages = vec![Message {
            id: 10,
            chat_id: 3,
            thread_topic_id: None,
            sender_name: "Alice".to_string(),
            content: "Hello".to_string(),
            timestamp: Utc::now(),
            is_own: false,
            is_edited: false,
            reply_to_content: None,
            media: None,
            status: MessageStatus::Read,
            can_edit: false,
            can_delete: false,
            error: None,
        }];

        let rendered = render_app_to_string_for_test(&mut app);

        assert!(rendered.contains("Topics"));
        assert!(rendered.contains("General (1)"));
        assert!(rendered.contains("Deployments"));
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
