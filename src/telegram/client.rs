use super::types::{Chat, Folder, Message, Update};
use color_eyre::Result;
use tokio::sync::mpsc;

pub trait TelegramClient {
    async fn connect(&mut self) -> Result<()>;
    async fn get_folders(&self) -> Result<Vec<Folder>>;
    async fn get_chats(&self, folder_id: Option<i32>) -> Result<Vec<Chat>>;
    async fn get_messages(&self, chat_id: i64, limit: usize) -> Result<Vec<Message>>;
    async fn send_message(&self, chat_id: i64, content: String) -> Result<Message>;
    async fn edit_message(&self, chat_id: i64, message_id: i32, content: String) -> Result<()>;
    async fn reply_to_message(
        &self,
        chat_id: i64,
        reply_to: i32,
        content: String,
    ) -> Result<Message>;
    async fn delete_message(&self, chat_id: i64, message_id: i32) -> Result<()>;
    async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>>;
}
