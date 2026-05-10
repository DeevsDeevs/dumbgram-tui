use crate::text::truncate_with_ellipsis;
use chrono::{DateTime, Utc};

pub const LAST_MESSAGE_PREVIEW_WIDTH: usize = 50;

pub fn message_preview(content: &str) -> String {
    truncate_with_ellipsis(content, LAST_MESSAGE_PREVIEW_WIDTH)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageStatus {
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
}

#[derive(Debug, Clone)]
pub enum Update {
    NewMessage(Message),
    EditMessage {
        chat_id: i64,
        message_id: i32,
        new_content: String,
    },
    DeleteMessage {
        chat_id: i64,
        message_id: i32,
    },
    TypingStatus {
        chat_id: i64,
        user_name: String,
        is_typing: bool,
    },
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub id: i32,
    pub name: String,
    pub unread_count: usize,
}

#[derive(Debug, Clone)]
pub struct Chat {
    pub id: i64,
    pub name: String,
    pub last_message: Option<String>,
    pub unread_count: usize,
    pub is_group: bool,
    pub folder_id: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: i32,
    pub chat_id: i64,
    pub sender_name: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub is_own: bool,
    pub is_edited: bool,
    pub reply_to_content: Option<String>,
    pub status: MessageStatus,
    pub can_edit: bool,
    pub can_delete: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{LAST_MESSAGE_PREVIEW_WIDTH, message_preview};
    use crate::text::display_width;

    #[test]
    fn message_preview_leaves_short_text_unchanged() {
        assert_eq!(message_preview("short message"), "short message");
    }

    #[test]
    fn message_preview_truncates_by_display_width() {
        let text = "好".repeat(40);
        let preview = message_preview(&text);

        assert!(preview.ends_with('…'));
        assert!(display_width(&preview) <= LAST_MESSAGE_PREVIEW_WIDTH);
    }
}
