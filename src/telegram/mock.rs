use super::client::TelegramClient;
use super::types::{Chat, Folder, Message, MessageStatus, OWN_SENDER_NAME, Update, all_folder};
use chrono::Utc;
use color_eyre::Result;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Clone)]
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

    #[allow(clippy::manual_async_fn)]
    fn get_folders(&self) -> impl std::future::Future<Output = Result<Vec<Folder>>> + Send + '_ {
        async move {
            Ok(vec![
                all_folder(5),
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
    }

    async fn get_chats(&self, folder_id: Option<i32>, limit: usize) -> Result<Vec<Chat>> {
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

        let chats = if let Some(fid) = folder_id {
            all_chats
                .into_iter()
                .filter(|c| c.folder_id == Some(fid))
                .collect::<Vec<_>>()
        } else {
            all_chats
        };
        Ok(chats.into_iter().take(limit).collect())
    }

    #[allow(clippy::manual_async_fn)]
    fn get_messages(
        &self,
        chat_id: i64,
        _limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
        async move {
            match chat_id {
                1 => Ok(vec![
                    Message {
                        id: 1,
                        chat_id,
                        sender_name: "Alice".to_string(),
                        content: "Hey! How are you?".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 2,
                        chat_id,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "I'm doing great! How about you?".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: false,
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
                        content: "Pretty good! Want to grab coffee later?".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
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
                        content: "Did you see the game last night?".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 2,
                        chat_id,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "Yeah! It was incredible!".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: false,
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
                        content: "Team meeting at 3 PM today".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 2,
                        chat_id,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "Got it, I'll be there".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: false,
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
                        content: "Should I prepare the slides?".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
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
                        content: "Deploy is ready for staging".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 2,
                        chat_id,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "Great! Let's review it first".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: true,
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
                        content: "I can test it this afternoon".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
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
    }

    #[allow(clippy::manual_async_fn)]
    fn get_messages_before(
        &self,
        chat_id: i64,
        before_message_id: i32,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
        async move {
            let mut messages = self.get_messages(chat_id, usize::MAX).await?;
            messages.retain(|message| message.id < before_message_id);
            let start = messages.len().saturating_sub(limit);
            Ok(messages.split_off(start))
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn send_message(
        &self,
        chat_id: i64,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_ {
        async move {
            Ok(Message {
                id: 999,
                chat_id,
                sender_name: OWN_SENDER_NAME.to_string(),
                content,
                timestamp: Utc::now(),
                is_own: true,
                is_edited: false,
                reply_to_content: None,
                status: MessageStatus::Sent,
                can_edit: true,
                can_delete: true,
                error: None,
            })
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn edit_message(
        &self,
        _chat_id: i64,
        _message_id: i32,
        _content: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move { Ok(()) }
    }

    #[allow(clippy::manual_async_fn)]
    fn reply_to_message(
        &self,
        chat_id: i64,
        _reply_to: i32,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_ {
        async move {
            Ok(Message {
                id: 1000,
                chat_id,
                sender_name: OWN_SENDER_NAME.to_string(),
                content,
                timestamp: Utc::now(),
                is_own: true,
                is_edited: false,
                reply_to_content: Some("Replied message".to_string()),
                status: MessageStatus::Sent,
                can_edit: true,
                can_delete: true,
                error: None,
            })
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn delete_message(
        &self,
        _chat_id: i64,
        _message_id: i32,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move { Ok(()) }
    }

    #[allow(clippy::manual_async_fn)]
    fn subscribe_updates(
        &mut self,
    ) -> impl std::future::Future<Output = Result<mpsc::UnboundedReceiver<Update>>> + Send + '_
    {
        async move {
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
                            content: format!("Mock update message #{}", counter),
                            timestamp: Utc::now(),
                            is_own: false,
                            is_edited: false,
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
}
