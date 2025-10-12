use chrono::{DateTime, Utc};

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
    pub reply_to_id: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: String,
    pub last_name: Option<String>,
}
