use super::client::{DownloadedMedia, ReconciliationChatList, TelegramClient};
use super::session_file::secure_trusted_directory;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::session_file::{secure_private_directory, secure_private_file_handle};
use super::types::{
    Chat, Folder, Message, MessageMedia, MessageStatus, OWN_SENDER_NAME, SenderIdentity,
    ThreadTopic, Update, all_folder,
};
use base64::Engine;
use chrono::Utc;
use color_eyre::Result;
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

const UPDATE_CHANNEL_CAPACITY: usize = 100;
const MAX_MOCK_PATH_ATTEMPTS: u64 = 10_000;
const UPDATE_CHANNEL_SATURATED: &str =
    "Mock update buffer saturated; refreshing authoritative state";

#[derive(Clone)]
pub struct MockTelegramClient {
    connected: bool,
    typing_action_count: Arc<AtomicUsize>,
    preview_load_count: Arc<AtomicUsize>,
}

impl MockTelegramClient {
    pub fn new() -> Self {
        Self {
            connected: false,
            typing_action_count: Arc::new(AtomicUsize::new(0)),
            preview_load_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[cfg(test)]
    pub fn typing_action_count(&self) -> usize {
        self.typing_action_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn preview_load_count(&self) -> usize {
        self.preview_load_count.load(Ordering::Relaxed)
    }
}

const MOCK_IMAGE_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAKAAAABQCAIAAAARP+ljAAAA2klEQVR42u3bMQ0AIAxFwYpABBKRiKuioVMTegnzG/6tJdbJ0su7S0+/tx8GAgwAMADA+oD1AesD1gcM2ECAAQAGAFgfsD5gfcBjgQ36dx8wYACAAQDWB6wPWB+wPmDABgIMADAAwPqA9QHrA54LbFAXHQAAAwCsD1gfsD5gfcCADQQYAGAAgPUB6wPWBwzYQP4HG9RFBwDA+oD1AesD1gcMGABgAID1AesD1gesDxiwgQAD8D8YmIsOfcD6gPUBAzYQYACAAQDWB6wPWB+wPmDABgIMALB+a/8BI+/vC6JSYoIAAAAASUVORK5CYII=";
static MOCK_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);
static MOCK_IMAGE_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn secure_mock_directory(path: &Path) -> std::io::Result<()> {
    secure_private_directory(path).map_err(std::io::Error::other)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn secure_mock_directory(path: &Path) -> std::io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        Ok(())
    } else {
        Err(std::io::Error::other("mock media path is not a directory"))
    }
}

fn secure_mock_parent(path: &Path) -> std::io::Result<()> {
    secure_trusted_directory(path).map_err(std::io::Error::other)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn secure_mock_file_handle(file: &fs::File, path: &Path) -> std::io::Result<()> {
    secure_private_file_handle(file, path).map_err(std::io::Error::other)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn secure_mock_file_handle(file: &fs::File, path: &Path) -> std::io::Result<()> {
    if file.metadata()?.is_file() && fs::symlink_metadata(path)?.is_file() {
        Ok(())
    } else {
        Err(std::io::Error::other("mock media path is not a file"))
    }
}

fn private_mock_dir() -> Option<PathBuf> {
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| secure_mock_parent(home).is_ok())?;
    for _ in 0..MAX_MOCK_PATH_ATTEMPTS {
        let id = MOCK_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(".dumbgram-tui-mock-{}-{id}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => {
                if secure_mock_directory(&path).is_ok() {
                    return Some(path);
                }
                let _ = fs::remove_dir(&path);
                return None;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

fn create_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    secure_mock_file_handle(&file, path)
}

fn mock_image_path() -> Option<PathBuf> {
    MOCK_IMAGE_PATH
        .get_or_init(|| {
            let path = private_mock_dir()?.join("photo.png");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(MOCK_IMAGE_PNG_BASE64)
                .ok()?;
            create_private_file(&path, &bytes).ok()?;
            Some(path)
        })
        .clone()
}

fn reserve_mock_download(destination_dir: &Path, message_id: i32, bytes: &[u8]) -> Result<PathBuf> {
    fs::create_dir_all(destination_dir)?;
    for index in 0..MAX_MOCK_PATH_ATTEMPTS {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!(" ({index})")
        };
        let path = destination_dir.join(format!("dumbgram-mock-message-{message_id}{suffix}.png"));
        match create_private_file(&path, bytes) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    color_eyre::eyre::bail!("could not reserve a mock media download path")
}

#[cfg(test)]
fn mock_photo_media() -> MessageMedia {
    mock_image_path().map_or_else(MessageMedia::photo, |path| {
        MessageMedia::photo().with_local_path(path)
    })
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

    async fn send_typing_action(&self, _chat_id: i64, _topic_id: Option<i32>) -> Result<()> {
        self.typing_action_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn load_message_media_preview(
        &self,
        _chat_id: i64,
        _message_id: i32,
    ) -> Result<Option<PathBuf>> {
        self.preview_load_count.fetch_add(1, Ordering::Relaxed);
        Ok(mock_image_path())
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

    #[allow(clippy::manual_async_fn)]
    fn download_message_media(
        &self,
        _chat_id: i64,
        message_id: i32,
        destination_dir: PathBuf,
    ) -> impl std::future::Future<Output = Result<DownloadedMedia>> + Send + '_ {
        async move {
            let bytes = base64::engine::general_purpose::STANDARD.decode(MOCK_IMAGE_PNG_BASE64)?;
            let path = reserve_mock_download(&destination_dir, message_id, &bytes)?;
            Ok(DownloadedMedia {
                path,
                bytes: bytes.len() as u64,
            })
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

    async fn get_reconciliation_chats(
        &self,
        folder_id: Option<i32>,
        limit: usize,
    ) -> Result<ReconciliationChatList> {
        let chats = self.get_chats(folder_id, limit).await?;
        let mut last_message_ids = HashMap::new();
        for chat in &chats {
            if let Some(message) = self.get_messages(chat.id, usize::MAX).await?.last() {
                last_message_ids.insert(chat.id, message.id);
            }
        }
        Ok(ReconciliationChatList {
            chats,
            last_message_ids,
        })
    }

    async fn get_thread_topics(&self, chat_id: i64, limit: usize) -> Result<Vec<ThreadTopic>> {
        let topics = match chat_id {
            3 => vec![
                ThreadTopic {
                    id: 101,
                    title: "General".to_string(),
                    top_message_id: 1001,
                    unread_count: 1,
                    is_closed: false,
                    is_pinned: true,
                },
                ThreadTopic {
                    id: 102,
                    title: "Deployments".to_string(),
                    top_message_id: 1002,
                    unread_count: 0,
                    is_closed: false,
                    is_pinned: false,
                },
            ],
            _ => Vec::new(),
        };
        Ok(topics.into_iter().take(limit).collect())
    }

    async fn get_thread_topic(&self, chat_id: i64, topic_id: i32) -> Result<Option<ThreadTopic>> {
        Ok(self
            .get_thread_topics(chat_id, usize::MAX)
            .await?
            .into_iter()
            .find(|topic| topic.id == topic_id))
    }

    #[allow(clippy::manual_async_fn)]
    fn get_thread_messages(
        &self,
        chat_id: i64,
        topic_id: i32,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
        async move {
            let messages = match (chat_id, topic_id) {
                (3, 101) => vec![
                    Message {
                        id: 101,
                        chat_id,
                        thread_topic_id: Some(101),
                        sender_identity: None,
                        sender_name: "Manager".to_string(),
                        content: "General topic: weekly coordination".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        media: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 103,
                        chat_id,
                        thread_topic_id: Some(101),
                        sender_identity: None,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "I'll post the notes here.".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: false,
                        reply_to_content: Some("General topic: weekly coordination".to_string()),
                        media: None,
                        status: MessageStatus::Sent,
                        can_edit: true,
                        can_delete: true,
                        error: None,
                    },
                ],
                (3, 102) => vec![Message {
                    id: 102,
                    chat_id,
                    thread_topic_id: Some(102),
                    sender_identity: None,
                    sender_name: "Developer".to_string(),
                    content: "Deployments topic: staging is ready".to_string(),
                    timestamp: Utc::now(),
                    is_own: false,
                    is_edited: false,
                    reply_to_content: None,
                    media: None,
                    status: MessageStatus::Delivered,
                    can_edit: false,
                    can_delete: false,
                    error: None,
                }],
                _ => Vec::new(),
            };

            Ok(messages.into_iter().take(limit).collect())
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn get_thread_messages_before(
        &self,
        chat_id: i64,
        topic_id: i32,
        before_message_id: i32,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
        async move {
            let mut messages = self
                .get_thread_messages(chat_id, topic_id, usize::MAX)
                .await?;
            messages.retain(|message| message.id < before_message_id);
            let start = messages.len().saturating_sub(limit);
            Ok(messages.split_off(start))
        }
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
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: "Alice".to_string(),
                        content: "Hey! How are you?".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        media: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 2,
                        chat_id,
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "I'm doing great! How about you?".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: false,
                        reply_to_content: None,
                        media: None,
                        status: MessageStatus::Read,
                        can_edit: true,
                        can_delete: true,
                        error: None,
                    },
                    Message {
                        id: 3,
                        chat_id,
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: "Alice".to_string(),
                        content: "Pretty good! Want to grab coffee later?".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: Some("I'm doing great! How about you?".to_string()),
                        media: None,
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
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: "Bob".to_string(),
                        content: "Did you see the game last night?".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        media: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 2,
                        chat_id,
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "Yeah! It was incredible!".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: false,
                        reply_to_content: None,
                        media: None,
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
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: "Manager".to_string(),
                        content: "Team meeting at 3 PM today".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        media: None,
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 2,
                        chat_id,
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "Got it, I'll be there".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: false,
                        reply_to_content: None,
                        media: None,
                        status: MessageStatus::Sent,
                        can_edit: true,
                        can_delete: true,
                        error: None,
                    },
                    Message {
                        id: 3,
                        chat_id,
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: "Colleague".to_string(),
                        content: "Should I prepare the slides?".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        media: None,
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
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: "Developer".to_string(),
                        content: "Deploy is ready for staging".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: None,
                        media: Some(MessageMedia::photo()),
                        status: MessageStatus::Delivered,
                        can_edit: false,
                        can_delete: false,
                        error: None,
                    },
                    Message {
                        id: 2,
                        chat_id,
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: OWN_SENDER_NAME.to_string(),
                        content: "Great! Let's review it first".to_string(),
                        timestamp: Utc::now(),
                        is_own: true,
                        is_edited: true,
                        reply_to_content: None,
                        media: None,
                        status: MessageStatus::Delivered,
                        can_edit: true,
                        can_delete: true,
                        error: None,
                    },
                    Message {
                        id: 3,
                        chat_id,
                        thread_topic_id: None,
                        sender_identity: None,
                        sender_name: "QA".to_string(),
                        content: "I can test it this afternoon".to_string(),
                        timestamp: Utc::now(),
                        is_own: false,
                        is_edited: false,
                        reply_to_content: Some("Great! Let's review it first".to_string()),
                        media: None,
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
                thread_topic_id: None,
                sender_identity: None,
                sender_name: OWN_SENDER_NAME.to_string(),
                content,
                timestamp: Utc::now(),
                is_own: true,
                is_edited: false,
                reply_to_content: None,
                media: None,
                status: MessageStatus::Sent,
                can_edit: true,
                can_delete: true,
                error: None,
            })
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn send_message_to_thread(
        &self,
        chat_id: i64,
        topic_id: i32,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_ {
        async move {
            Ok(Message {
                id: 1000 + topic_id,
                chat_id,
                thread_topic_id: Some(topic_id),
                sender_identity: None,
                sender_name: OWN_SENDER_NAME.to_string(),
                content,
                timestamp: Utc::now(),
                is_own: true,
                is_edited: false,
                reply_to_content: Some(format!("topic {topic_id}")),
                media: None,
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
                thread_topic_id: None,
                sender_identity: None,
                sender_name: OWN_SENDER_NAME.to_string(),
                content,
                timestamp: Utc::now(),
                is_own: true,
                is_edited: false,
                reply_to_content: Some("Replied message".to_string()),
                media: None,
                status: MessageStatus::Sent,
                can_edit: true,
                can_delete: true,
                error: None,
            })
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn reply_to_message_in_thread(
        &self,
        chat_id: i64,
        topic_id: i32,
        reply_to: i32,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_ {
        async move {
            Ok(Message {
                id: 2000 + topic_id,
                chat_id,
                thread_topic_id: Some(topic_id),
                sender_identity: None,
                sender_name: OWN_SENDER_NAME.to_string(),
                content,
                timestamp: Utc::now(),
                is_own: true,
                is_edited: false,
                reply_to_content: Some(format!("topic {topic_id} reply {reply_to}")),
                media: None,
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
    ) -> impl std::future::Future<Output = Result<mpsc::Receiver<Update>>> + Send + '_ {
        async move {
            let (tx, rx) = mpsc::channel(UPDATE_CHANNEL_CAPACITY);

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
                            thread_topic_id: None,
                            sender_identity: None,
                            sender_name: "Alice".to_string(),
                            content: format!("Mock update message #{}", counter),
                            timestamp: Utc::now(),
                            is_own: false,
                            is_edited: false,
                            reply_to_content: None,
                            media: None,
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
                            topic_id: None,
                            sender_identity: SenderIdentity::User(1),
                            user_name: "Alice".to_string(),
                            is_typing: true,
                        },
                    };

                    match tx.try_send(update) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            if tx
                                .send(Update::Error(UPDATE_CHANNEL_SATURATED.to_string()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            });

            Ok(rx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MOCK_IMAGE_PNG_BASE64, MockTelegramClient, mock_photo_media, private_mock_dir};
    use crate::telegram::TelegramClient;
    use base64::Engine;

    #[tokio::test]
    async fn mock_thread_topics_are_available_for_threaded_group() {
        let client = MockTelegramClient::new();

        let topics = client.get_thread_topics(3, 10).await.unwrap();

        assert_eq!(topics.len(), 2);
        assert_eq!(topics[0].title, "General");
        assert_eq!(topics[0].top_message_id, 1001);
    }

    #[tokio::test]
    async fn mock_thread_messages_are_available_by_topic_id() {
        let client = MockTelegramClient::new();

        let messages = client.get_thread_messages(3, 101, 10).await.unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].id, 101);
        assert!(messages[0].content.contains("General topic"));
        assert_eq!(
            messages[1].reply_to_content.as_deref(),
            Some("General topic: weekly coordination")
        );
    }

    #[tokio::test]
    async fn mock_older_thread_messages_page_within_topic() {
        let client = MockTelegramClient::new();

        let messages = client
            .get_thread_messages_before(3, 101, 103, 10)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 101);
        assert!(messages[0].content.contains("General topic"));
    }

    #[tokio::test]
    async fn mock_thread_messages_respect_limit_and_unknown_threads() {
        let client = MockTelegramClient::new();

        assert_eq!(
            client.get_thread_messages(3, 101, 1).await.unwrap().len(),
            1
        );
        assert!(
            client
                .get_thread_messages(3, 1001, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            client
                .get_thread_messages(3, 999, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            client
                .get_thread_messages(1, 101, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn mock_reply_to_message_in_thread_marks_topic_context() {
        let client = MockTelegramClient::new();

        let message = client
            .reply_to_message_in_thread(3, 102, 7, "ack".to_string())
            .await
            .unwrap();

        assert_eq!(message.chat_id, 3);
        assert_eq!(message.id, 2102);
        assert_eq!(message.content, "ack");
        assert_eq!(
            message.reply_to_content.as_deref(),
            Some("topic 102 reply 7")
        );
    }

    #[tokio::test]
    async fn mock_send_message_to_thread_marks_topic_context() {
        let client = MockTelegramClient::new();

        let message = client
            .send_message_to_thread(3, 102, "ship it".to_string())
            .await
            .unwrap();

        assert_eq!(message.chat_id, 3);
        assert_eq!(message.id, 1102);
        assert_eq!(message.content, "ship it");
        assert_eq!(message.reply_to_content.as_deref(), Some("topic 102"));
    }

    #[tokio::test]
    async fn mock_thread_topics_respect_limit_and_non_threaded_chats() {
        let client = MockTelegramClient::new();

        assert_eq!(client.get_thread_topics(3, 1).await.unwrap().len(), 1);
        assert!(client.get_thread_topics(1, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn mock_media_download_never_overwrites_an_existing_file() {
        let client = MockTelegramClient::new();
        let root = private_mock_dir().expect("private mock directory should be available");
        let existing = root.join("dumbgram-mock-message-42.png");
        std::fs::write(&existing, b"keep me").unwrap();

        let saved = client
            .download_message_media(1, 42, root.clone())
            .await
            .unwrap();

        assert_eq!(std::fs::read(existing).unwrap(), b"keep me");
        assert_ne!(saved.path, root.join("dumbgram-mock-message-42.png"));
        assert!(saved.path.ends_with("dumbgram-mock-message-42 (1).png"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mock_photo_media_has_local_preview_file_for_kitty_smoke() {
        let media = mock_photo_media();

        let path = media
            .local_image_path()
            .expect("mock photo should have a local preview image");
        assert!(path.exists());
        assert!(std::fs::metadata(path).unwrap().len() > 200);
        assert!(
            path.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".dumbgram-tui-mock-")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                std::fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
                0o700
            );
            assert_eq!(std::fs::metadata(path).unwrap().mode() & 0o777, 0o600);
            assert_eq!(
                std::fs::metadata(path).unwrap().uid(),
                std::fs::metadata(path.parent().unwrap()).unwrap().uid()
            );
        }

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(MOCK_IMAGE_PNG_BASE64)
            .unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
