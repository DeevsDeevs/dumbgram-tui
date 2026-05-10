use chrono::{DateTime, Utc};
use color_eyre::Result;
use grammers_client::{Client, Config, InitParams, InputMessage, grammers_tl_types as tl};
use grammers_session::Session;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::client::TelegramClient;
use super::types::{Chat, Folder, Message, MessageStatus, Update, message_preview};

pub struct GrammersClient {
    client: Client,
    chat_cache: Arc<Mutex<HashMap<i64, grammers_client::types::Chat>>>,
    session_path: String,
}

impl GrammersClient {
    pub async fn new(api_id: i32, api_hash: String, session_path: &str) -> Result<Self> {
        let session = Session::load_file_or_create(session_path)?;

        let client = Client::connect(Config {
            session,
            api_id,
            api_hash,
            params: InitParams {
                device_model: "Dumbgram TUI".to_string(),
                system_version: env!("CARGO_PKG_VERSION").to_string(),
                app_version: "1.0.0".to_string(),
                catch_up: true,
                update_queue_limit: Some(100),
                flood_sleep_threshold: 60,
                ..Default::default()
            },
        })
        .await?;

        Ok(Self {
            client,
            chat_cache: Arc::new(Mutex::new(HashMap::new())),
            session_path: session_path.to_string(),
        })
    }

    pub fn inner(&self) -> &Client {
        &self.client
    }

    pub fn save_session(&self) -> Result<()> {
        self.client.session().save_to_file(&self.session_path)?;
        Ok(())
    }

    fn get_chat(&self, chat_id: i64) -> Option<grammers_client::types::Chat> {
        self.chat_cache.lock().unwrap().get(&chat_id).cloned()
    }

    fn cache_chat(&self, chat: grammers_client::types::Chat) {
        self.chat_cache.lock().unwrap().insert(chat.id(), chat);
    }
}

fn convert_message(msg: grammers_client::types::Message) -> Message {
    Message {
        id: msg.id(),
        chat_id: msg.chat().id(),
        sender_name: msg
            .sender()
            .map(|s| s.name().to_string())
            .unwrap_or_else(|| "Unknown".to_string()),
        content: msg.text().to_string(),
        timestamp: msg.date(),
        is_own: msg.outgoing(),
        is_edited: msg.edit_date().is_some(),
        reply_to_content: None,
        status: if msg.outgoing() {
            MessageStatus::Sent
        } else {
            MessageStatus::Delivered
        },
        can_edit: msg.outgoing() && is_within_edit_window(msg.date()),
        can_delete: msg.outgoing(),
        error: None,
    }
}

fn is_within_edit_window(date: DateTime<Utc>) -> bool {
    let now = chrono::Utc::now();
    (now - date).num_hours() < 48
}

fn dialog_unread_count(dialog: &grammers_client::types::Dialog) -> usize {
    match &dialog.raw {
        tl::enums::Dialog::Dialog(raw) => raw.unread_count.max(0) as usize,
        tl::enums::Dialog::Folder(raw) => {
            (raw.unread_muted_messages_count + raw.unread_unmuted_messages_count).max(0) as usize
        }
    }
}

fn dialog_folder_id(dialog: &grammers_client::types::Dialog) -> Option<i32> {
    match &dialog.raw {
        tl::enums::Dialog::Dialog(raw) => raw.folder_id,
        tl::enums::Dialog::Folder(_) => None,
    }
}

async fn collect_message_page(
    mut iter: grammers_client::client::messages::MessageIter,
    limit: usize,
) -> Result<Vec<Message>> {
    let mut messages = Vec::new();

    while let Some(msg) = iter.next().await? {
        messages.push(convert_message(msg));
        if messages.len() >= limit {
            break;
        }
    }

    messages.reverse();
    Ok(messages)
}

impl TelegramClient for GrammersClient {
    async fn connect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn send_message(&self, chat_id: i64, content: String) -> Result<Message> {
        let chat = self
            .get_chat(chat_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Chat not found in cache"))?;
        let msg = self
            .client
            .send_message(chat, InputMessage::text(content))
            .await?;
        Ok(convert_message(msg))
    }

    async fn edit_message(&self, chat_id: i64, message_id: i32, content: String) -> Result<()> {
        let chat = self
            .get_chat(chat_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Chat not found in cache"))?;
        self.client
            .edit_message(chat, message_id, InputMessage::text(content))
            .await?;
        Ok(())
    }

    async fn reply_to_message(
        &self,
        chat_id: i64,
        reply_to: i32,
        content: String,
    ) -> Result<Message> {
        let chat = self
            .get_chat(chat_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Chat not found in cache"))?;
        let input = InputMessage::text(content).reply_to(Some(reply_to));
        let msg = self.client.send_message(chat, input).await?;
        Ok(convert_message(msg))
    }

    async fn delete_message(&self, chat_id: i64, message_id: i32) -> Result<()> {
        let chat = self
            .get_chat(chat_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Chat not found in cache"))?;
        self.client.delete_messages(chat, &[message_id]).await?;
        Ok(())
    }

    async fn get_messages(&self, chat_id: i64, limit: usize) -> Result<Vec<Message>> {
        let chat = self
            .get_chat(chat_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Chat not found in cache"))?;
        let iter = self.client.iter_messages(chat);
        collect_message_page(iter, limit).await
    }

    async fn get_messages_before(
        &self,
        chat_id: i64,
        before_message_id: i32,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let chat = self
            .get_chat(chat_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("Chat not found in cache"))?;
        let iter = self.client.iter_messages(chat).offset_id(before_message_id);
        collect_message_page(iter, limit).await
    }

    async fn get_chats(&self, folder_id: Option<i32>) -> Result<Vec<Chat>> {
        let mut iter = self.client.iter_dialogs();
        let mut chats = Vec::new();

        while let Some(dialog) = iter.next().await? {
            let dialog_folder_id = dialog_folder_id(&dialog);
            if folder_id.is_some() && dialog_folder_id != folder_id {
                continue;
            }

            let chat = dialog.chat();
            self.cache_chat(chat.clone());

            chats.push(Chat {
                id: chat.id(),
                name: chat.name().to_string(),
                last_message: dialog
                    .last_message
                    .as_ref()
                    .map(|m| message_preview(m.text())),
                unread_count: dialog_unread_count(&dialog),
                is_group: matches!(chat, grammers_client::types::Chat::Group(_)),
                folder_id: dialog_folder_id,
            });
        }

        Ok(chats)
    }

    async fn get_folders(&self) -> Result<Vec<Folder>> {
        Ok(vec![Folder {
            id: 1,
            name: "All".to_string(),
            unread_count: 0,
        }])
    }

    async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let client = self.client.clone();

        tokio::spawn(async move {
            loop {
                match client.next_update().await {
                    Ok(update) => {
                        let our_update = match update {
                            grammers_client::Update::NewMessage(msg) => {
                                if !msg.outgoing() {
                                    Some(Update::NewMessage(convert_message(msg)))
                                } else {
                                    None
                                }
                            }
                            grammers_client::Update::MessageEdited(msg) => {
                                Some(Update::EditMessage {
                                    chat_id: msg.chat().id(),
                                    message_id: msg.id(),
                                    new_content: msg.text().to_string(),
                                })
                            }
                            grammers_client::Update::MessageDeleted(deletion) => deletion
                                .messages()
                                .first()
                                .map(|msg_id| Update::DeleteMessage {
                                    chat_id: 0,
                                    message_id: *msg_id,
                                }),
                            _ => None,
                        };

                        if let Some(update) = our_update
                            && tx.send(update).is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Update error: {}", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });

        Ok(rx)
    }
}
