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

#[cfg(test)]
mod tests {
    use super::render_layout;
    use crate::{
        app::App,
        config::Theme,
        telegram::types::{Chat, Message, MessageStatus},
    };
    use chrono::Utc;
    use ratatui::{Terminal, backend::TestBackend};

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
        let theme = Theme::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_layout(frame, &mut app, &theme))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        for border in ["┌", "┐", "└", "┘", "─", "│"] {
            assert!(rendered.contains(border), "missing border glyph {border}");
        }
        assert!(rendered.contains("▶"), "missing selected-row arrow glyph");
        assert!(
            !rendered.contains(">>"),
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
}
