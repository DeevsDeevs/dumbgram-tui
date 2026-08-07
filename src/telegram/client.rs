use super::types::{Chat, Folder, Message, ThreadTopic, Update};
use color_eyre::{Result, eyre::eyre};
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedMedia {
    pub path: PathBuf,
    pub bytes: u64,
}

pub struct ReconciliationChatList {
    pub chats: Vec<Chat>,
    pub last_message_ids: HashMap<i64, i32>,
}

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
    fn get_reconciliation_chats(
        &self,
        folder_id: Option<i32>,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<ReconciliationChatList>> + Send + '_
    where
        Self: Sync,
    {
        async move {
            Ok(ReconciliationChatList {
                chats: self.get_chats(folder_id, limit).await?,
                last_message_ids: HashMap::new(),
            })
        }
    }
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
    fn mark_thread_read(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _max_message_id: i32,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move { Ok(()) }
    }
    fn send_typing_action(
        &self,
        _chat_id: i64,
        _topic_id: Option<i32>,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move { Ok(()) }
    }
    fn get_thread_topics(
        &self,
        _chat_id: i64,
        _limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<ThreadTopic>>> + Send + '_ {
        async move { Ok(Vec::new()) }
    }
    fn get_thread_messages(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
        async move { Ok(Vec::new()) }
    }
    fn get_thread_messages_before(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _before_message_id: i32,
        _limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
        async move { Ok(Vec::new()) }
    }
    /// Returns a cached local preview path, `None` when no preview exists, or an attempt error.
    fn load_message_media_preview(
        &self,
        _chat_id: i64,
        _message_id: i32,
    ) -> impl std::future::Future<Output = Result<Option<PathBuf>>> + Send + '_ {
        async move { Ok(None) }
    }
    fn download_message_media(
        &self,
        _chat_id: i64,
        _message_id: i32,
        _destination_dir: PathBuf,
    ) -> impl std::future::Future<Output = Result<DownloadedMedia>> + Send + '_ {
        async move { Err(eyre!("No downloadable media")) }
    }
    #[allow(clippy::manual_async_fn)]
    fn send_message(
        &self,
        chat_id: i64,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_;
    fn send_message_to_thread(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_ {
        async move { Err(eyre!("Thread sends are unsupported")) }
    }
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
    fn reply_to_message_in_thread(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _reply_to: i32,
        _content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_ {
        async move { Err(eyre!("Thread replies are unsupported")) }
    }
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
