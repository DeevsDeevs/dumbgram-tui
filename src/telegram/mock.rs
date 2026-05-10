use super::client::TelegramClient;
use super::types::{Chat, Folder, Message, MessageStatus, Update};
use chrono::Utc;
use color_eyre::Result;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct MockTelegramClient {
    connected: bool,
}

impl MockTelegramClient {
    pub fn new() -> Self {
        Self { connected: false }
    }
}

impl Default for MockTelegramClient {
    fn default() -> Self {
        Self::new()
    }
}

impl TelegramClient for MockTelegramClient {
    async fn connect(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }

    async fn get_folders(&self) -> Result<Vec<Folder>> {
        Ok(vec![
            Folder {
                id: 1,
                name: "All".to_string(),
                unread_count: 5,
            },
            Folder {
                id: 2,
                name: "Personal".to_string(),
                unread_count: 3,
            },
            Folder {
                id: 3,
                name: "Work".to_string(),
                unread_count: 2,
            },
        ])
    }

    async fn get_chats(&self, folder_id: Option<i32>) -> Result<Vec<Chat>> {
        let all_chats = vec![
            Chat {
                id: 1,
                name: "Alice".to_string(),
                last_message: Some("Hey! How are you?".to_string()),
                unread_count: 2,
                is_group: false,
                folder_id: Some(2),
            },
            Chat {
                id: 2,
                name: "Bob".to_string(),
                last_message: Some("See you tomorrow!".to_string()),
                unread_count: 0,
                is_group: false,
                folder_id: Some(2),
            },
            Chat {
                id: 3,
                name: "Work Team".to_string(),
                last_message: Some("Meeting at 3 PM".to_string()),
                unread_count: 1,
                is_group: true,
                folder_id: Some(3),
            },
            Chat {
                id: 4,
                name: "Project Alpha".to_string(),
                last_message: Some("Deploy is ready".to_string()),
                unread_count: 1,
                is_group: true,
                folder_id: Some(3),
            },
        ];

        if let Some(fid) = folder_id {
            if fid == 1 {
                Ok(all_chats)
            } else {
                Ok(all_chats
                    .into_iter()
                    .filter(|c| c.folder_id == Some(fid))
                    .collect())
            }
        } else {
            Ok(all_chats)
        }
    }

    async fn get_messages(&self, chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
        match chat_id {
            1 => Ok(vec![
                Message {
                    id: 1,
                    chat_id,
                    sender_name: "Alice".to_string(),
                    sender_id: 101,
                    content: "Hey! How are you?".to_string(),
                    timestamp: Utc::now(),
                    is_own: false,
                    is_edited: false,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Delivered,
                    can_edit: false,
                    can_delete: false,
                    error: None,
                },
                Message {
                    id: 2,
                    chat_id,
                    sender_name: "You".to_string(),
                    sender_id: 0,
                    content: "I'm doing great! How about you?".to_string(),
                    timestamp: Utc::now(),
                    is_own: true,
                    is_edited: false,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Read,
                    can_edit: true,
                    can_delete: true,
                    error: None,
                },
                Message {
                    id: 3,
                    chat_id,
                    sender_name: "Alice".to_string(),
                    sender_id: 101,
                    content: "Pretty good! Want to grab coffee later?".to_string(),
                    timestamp: Utc::now(),
                    is_own: false,
                    is_edited: false,
                    reply_to_id: Some(2),
                    reply_to_content: Some("I'm doing great! How about you?".to_string()),
                    status: MessageStatus::Delivered,
                    can_edit: false,
                    can_delete: false,
                    error: None,
                },
            ]),
            2 => Ok(vec![
                Message {
                    id: 1,
                    chat_id,
                    sender_name: "Bob".to_string(),
                    sender_id: 102,
                    content: "Did you see the game last night?".to_string(),
                    timestamp: Utc::now(),
                    is_own: false,
                    is_edited: false,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Delivered,
                    can_edit: false,
                    can_delete: false,
                    error: None,
                },
                Message {
                    id: 2,
                    chat_id,
                    sender_name: "You".to_string(),
                    sender_id: 0,
                    content: "Yeah! It was incredible!".to_string(),
                    timestamp: Utc::now(),
                    is_own: true,
                    is_edited: false,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Read,
                    can_edit: true,
                    can_delete: true,
                    error: None,
                },
            ]),
            3 => Ok(vec![
                Message {
                    id: 1,
                    chat_id,
                    sender_name: "Manager".to_string(),
                    sender_id: 103,
                    content: "Team meeting at 3 PM today".to_string(),
                    timestamp: Utc::now(),
                    is_own: false,
                    is_edited: false,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Delivered,
                    can_edit: false,
                    can_delete: false,
                    error: None,
                },
                Message {
                    id: 2,
                    chat_id,
                    sender_name: "You".to_string(),
                    sender_id: 0,
                    content: "Got it, I'll be there".to_string(),
                    timestamp: Utc::now(),
                    is_own: true,
                    is_edited: false,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Sent,
                    can_edit: true,
                    can_delete: true,
                    error: None,
                },
                Message {
                    id: 3,
                    chat_id,
                    sender_name: "Colleague".to_string(),
                    sender_id: 104,
                    content: "Should I prepare the slides?".to_string(),
                    timestamp: Utc::now(),
                    is_own: false,
                    is_edited: false,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Delivered,
                    can_edit: false,
                    can_delete: false,
                    error: None,
                },
            ]),
            4 => Ok(vec![
                Message {
                    id: 1,
                    chat_id,
                    sender_name: "Developer".to_string(),
                    sender_id: 105,
                    content: "Deploy is ready for staging".to_string(),
                    timestamp: Utc::now(),
                    is_own: false,
                    is_edited: false,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Delivered,
                    can_edit: false,
                    can_delete: false,
                    error: None,
                },
                Message {
                    id: 2,
                    chat_id,
                    sender_name: "You".to_string(),
                    sender_id: 0,
                    content: "Great! Let's review it first".to_string(),
                    timestamp: Utc::now(),
                    is_own: true,
                    is_edited: true,
                    reply_to_id: None,
                    reply_to_content: None,
                    status: MessageStatus::Delivered,
                    can_edit: true,
                    can_delete: true,
                    error: None,
                },
                Message {
                    id: 3,
                    chat_id,
                    sender_name: "QA".to_string(),
                    sender_id: 106,
                    content: "I can test it this afternoon".to_string(),
                    timestamp: Utc::now(),
                    is_own: false,
                    is_edited: false,
                    reply_to_id: Some(2),
                    reply_to_content: Some("Great! Let's review it first".to_string()),
                    status: MessageStatus::Delivered,
                    can_edit: false,
                    can_delete: false,
                    error: None,
                },
            ]),
            _ => Ok(vec![]),
        }
    }

    async fn send_message(&self, chat_id: i64, content: String) -> Result<Message> {
        Ok(Message {
            id: 999,
            chat_id,
            sender_name: "You".to_string(),
            sender_id: 0,
            content,
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_id: None,
            reply_to_content: None,
            status: MessageStatus::Sent,
            can_edit: true,
            can_delete: true,
            error: None,
        })
    }

    async fn edit_message(&self, _chat_id: i64, _message_id: i32, _content: String) -> Result<()> {
        Ok(())
    }

    async fn reply_to_message(
        &self,
        chat_id: i64,
        reply_to: i32,
        content: String,
    ) -> Result<Message> {
        Ok(Message {
            id: 1000,
            chat_id,
            sender_name: "You".to_string(),
            sender_id: 0,
            content,
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_id: Some(reply_to),
            reply_to_content: Some("Replied message".to_string()),
            status: MessageStatus::Sent,
            can_edit: true,
            can_delete: true,
            error: None,
        })
    }

    async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
        Ok(())
    }

    async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            let mut counter = 0;
            loop {
                interval.tick().await;
                counter += 1;

                let update = match counter % 3 {
                    0 => Update::NewMessage(Message {
                        id: 2000 + counter,
                        chat_id: 1,
                        sender_name: "Alice".to_string(),
                        sender_id: 101,
                        content: format!("Mock update message #{}", counter),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_id: None,
                        reply_to_content: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    }),
                    1 => Update::EditMessage {
                        chat_id: 1,
                        message_id: 1,
                        new_content: format!(
                            "Updated content at {}",
                            Utc::now().format("%H:%M:%S")
                        ),
                    },
                    _ => Update::TypingStatus {
                        chat_id: 1,
                        user_name: "Alice".to_string(),
                        is_typing: true,
                    },
                };

                if tx.send(update).is_err() {
                    break;
                }
            }
        });

        Ok(rx)
    }
}
