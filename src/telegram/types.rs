use crate::text::truncate_with_ellipsis;
use chrono::{DateTime, Utc};

pub const LAST_MESSAGE_PREVIEW_WIDTH: usize = 50;
pub const UNKNOWN_DELETE_UPDATE_CHAT_ID: i64 = 0;
pub const ALL_FOLDER_ID: i32 = 0;
pub const ALL_FOLDER_NAME: &str = "All";
pub const OWN_SENDER_NAME: &str = "You";
pub const UNKNOWN_SENDER_NAME: &str = "Unknown";

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
    Error(String),
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

pub fn all_folder(unread_count: usize) -> Folder {
    Folder {
        id: ALL_FOLDER_ID,
        name: ALL_FOLDER_NAME.to_string(),
        unread_count,
    }
}

pub fn is_all_folder(folder: &Folder) -> bool {
    folder.id == ALL_FOLDER_ID && folder.name == ALL_FOLDER_NAME
}

#[cfg(test)]
mod tests {
    use super::{
        ALL_FOLDER_ID, ALL_FOLDER_NAME, Folder, LAST_MESSAGE_PREVIEW_WIDTH, OWN_SENDER_NAME,
        UNKNOWN_DELETE_UPDATE_CHAT_ID, UNKNOWN_SENDER_NAME, all_folder, is_all_folder,
        message_preview,
    };
    use crate::text::display_width;

    #[test]
    fn message_preview_leaves_short_text_unchanged() {
        assert_eq!(message_preview("short message"), "short message");
    }

    #[test]
    fn unknown_delete_update_chat_id_is_explicit_wildcard() {
        assert_eq!(UNKNOWN_DELETE_UPDATE_CHAT_ID, 0);
    }

    #[test]
    fn sender_labels_are_shared_display_text() {
        assert_eq!(OWN_SENDER_NAME, "You");
        assert_eq!(UNKNOWN_SENDER_NAME, "Unknown");
    }

    #[test]
    fn all_folder_matching_uses_shared_identity() {
        let all = all_folder(5);
        let renamed_all = Folder {
            id: ALL_FOLDER_ID,
            name: "Everything".to_string(),
            unread_count: 0,
        };
        let copied_name = Folder {
            id: 2,
            name: ALL_FOLDER_NAME.to_string(),
            unread_count: 0,
        };

        assert_eq!(all.id, ALL_FOLDER_ID);
        assert_eq!(all.name, ALL_FOLDER_NAME);
        assert_eq!(all.unread_count, 5);
        assert!(is_all_folder(&all));
        assert!(!is_all_folder(&renamed_all));
        assert!(!is_all_folder(&copied_name));
    }

    #[test]
    fn message_preview_truncates_by_display_width() {
        let text = "好".repeat(40);
        let preview = message_preview(&text);

        assert!(preview.ends_with('…'));
        assert!(display_width(&preview) <= LAST_MESSAGE_PREVIEW_WIDTH);
    }
}
