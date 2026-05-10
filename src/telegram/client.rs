use super::types::{Chat, Folder, Message, Update};
use color_eyre::Result;
use tokio::sync::mpsc;

pub trait TelegramClient {
    async fn connect(&mut self) -> Result<()>;
    #[allow(clippy::manual_async_fn)]
    fn get_folders(&self) -> impl std::future::Future<Output = Result<Vec<Folder>>> + Send + '_;
    #[allow(clippy::manual_async_fn)]
    fn get_chats(
        &self,
        folder_id: Option<i32>,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Chat>>> + Send + '_;
    #[allow(clippy::manual_async_fn)]
    fn get_messages(
        &self,
        chat_id: i64,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_;
    #[allow(clippy::manual_async_fn)]
    fn get_messages_before(
        &self,
        chat_id: i64,
        before_message_id: i32,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_;
    fn mark_chat_read(
        &self,
        _chat_id: i64,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move { Ok(()) }
    }
    #[allow(clippy::manual_async_fn)]
    fn send_message(
        &self,
        chat_id: i64,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_;
    #[allow(clippy::manual_async_fn)]
    fn edit_message(
        &self,
        chat_id: i64,
        message_id: i32,
        content: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_;
    #[allow(clippy::manual_async_fn)]
    fn reply_to_message(
        &self,
        chat_id: i64,
        reply_to: i32,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_;
    #[allow(clippy::manual_async_fn)]
    fn delete_message(
        &self,
        chat_id: i64,
        message_id: i32,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_;
    #[allow(clippy::manual_async_fn)]
    fn subscribe_updates(
        &mut self,
    ) -> impl std::future::Future<Output = Result<mpsc::UnboundedReceiver<Update>>> + Send + '_;
}
