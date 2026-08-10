use chrono::{DateTime, Utc};
use color_eyre::Result;
use grammers_client::{
    Client, Config, FixedReconnect, InitParams, InputMessage, grammers_tl_types as tl,
    types::{ChatMap, Downloadable, photo_sizes::PhotoSize},
};
use grammers_session::{PackedChat, Session};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::mpsc;

use super::client::{DownloadedMedia, ReconciliationChatList, TelegramClient};
use super::types::{
    Chat, Folder, Message, MessageMedia, MessageMediaKind, MessageStatus, OWN_SENDER_NAME,
    ThreadTopic, UNKNOWN_DELETE_UPDATE_CHAT_ID, UNKNOWN_SENDER_NAME, Update, all_folder,
    message_display_preview,
};
use crate::diagnostics;

const CHAT_NOT_FOUND_IN_CACHE_PREFIX: &str = "Chat not found in cache";
const CHAT_CACHE_LOCK_FAILED: &str = "Chat cache lock failed";
const UPDATE_ERROR_PREFIX: &str = "Update error";
static MEDIA_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const TELEGRAM_RECONNECT_POLICY: FixedReconnect = FixedReconnect {
    attempts: 5,
    delay: Duration::from_secs(2),
};

enum BoundedChatReadRequest {
    Channel(tl::functions::channels::ReadHistory),
    Messages(tl::functions::messages::ReadHistory),
}

fn bounded_chat_read_request(
    chat: PackedChat,
    max_message_id: i32,
) -> Result<BoundedChatReadRequest> {
    if max_message_id <= 0 {
        color_eyre::eyre::bail!("chat read maximum must be positive");
    }
    Ok(if let Some(channel) = chat.try_to_input_channel() {
        BoundedChatReadRequest::Channel(tl::functions::channels::ReadHistory {
            channel,
            max_id: max_message_id,
        })
    } else {
        BoundedChatReadRequest::Messages(tl::functions::messages::ReadHistory {
            peer: chat.to_input_peer(),
            max_id: max_message_id,
        })
    })
}

type ChatCache = HashMap<i64, grammers_client::types::Chat>;
type UserNameCache = HashMap<i64, String>;
type DialogFilterCache = HashMap<i32, tl::enums::DialogFilter>;
type OutboxReadMaxIdCache = HashMap<i64, i32>;

#[derive(Clone)]
pub struct GrammersClient {
    client: Client,
    chat_cache: Arc<Mutex<ChatCache>>,
    user_name_cache: Arc<Mutex<UserNameCache>>,
    dialog_filter_cache: Arc<Mutex<DialogFilterCache>>,
    outbox_read_max_id_cache: Arc<Mutex<OutboxReadMaxIdCache>>,
    session_path: PathBuf,
    media_cache_dir: PathBuf,
}

impl GrammersClient {
    pub async fn new(api_id: i32, api_hash: String, session_path: &Path) -> Result<Self> {
        let session = Session::load_file_or_create(session_path)?;

        let client = Client::connect(Config {
            session,
            api_id,
            api_hash,
            params: dumbgram_init_params(),
        })
        .await?;

        Ok(Self {
            client,
            chat_cache: Arc::new(Mutex::new(HashMap::new())),
            user_name_cache: Arc::new(Mutex::new(HashMap::new())),
            dialog_filter_cache: Arc::new(Mutex::new(HashMap::new())),
            outbox_read_max_id_cache: Arc::new(Mutex::new(HashMap::new())),
            session_path: session_path.to_path_buf(),
            media_cache_dir: media_cache_dir(session_path)?,
        })
    }

    pub fn inner(&self) -> &Client {
        &self.client
    }

    pub fn save_session(&self) -> Result<()> {
        self.client.session().save_to_file(&self.session_path)?;
        Ok(())
    }

    fn chat_cache(&self) -> Result<MutexGuard<'_, ChatCache>> {
        self.chat_cache
            .lock()
            .map_err(|error| color_eyre::eyre::eyre!(chat_cache_lock_failed_message(error)))
    }

    fn get_chat(&self, chat_id: i64) -> Result<Option<grammers_client::types::Chat>> {
        Ok(self.chat_cache()?.get(&chat_id).cloned())
    }

    fn cached_chat(&self, chat_id: i64) -> Result<grammers_client::types::Chat> {
        self.get_chat(chat_id)?
            .ok_or_else(|| color_eyre::eyre::eyre!(chat_not_found_in_cache_message(chat_id)))
    }

    fn cache_chat(&self, chat: grammers_client::types::Chat) -> Result<()> {
        cache_user_name_from_chat(&self.user_name_cache, &chat);
        self.chat_cache()?.insert(chat.id(), chat);
        Ok(())
    }

    fn dialog_filter_cache(&self) -> Result<MutexGuard<'_, DialogFilterCache>> {
        self.dialog_filter_cache
            .lock()
            .map_err(|error| color_eyre::eyre::eyre!(chat_cache_lock_failed_message(error)))
    }

    fn cache_dialog_filters(&self, filters: &[tl::enums::DialogFilter]) -> Result<()> {
        let mut cache = self.dialog_filter_cache()?;
        cache.clear();
        for filter in filters {
            if let Some(folder) = folder_from_dialog_filter(filter) {
                cache.insert(folder.id, filter.clone());
            }
        }
        Ok(())
    }

    fn cached_dialog_filter(&self, folder_id: i32) -> Result<Option<tl::enums::DialogFilter>> {
        Ok(self.dialog_filter_cache()?.get(&folder_id).cloned())
    }

    fn outbox_read_max_id_cache(&self) -> Result<MutexGuard<'_, OutboxReadMaxIdCache>> {
        self.outbox_read_max_id_cache
            .lock()
            .map_err(|error| color_eyre::eyre::eyre!(chat_cache_lock_failed_message(error)))
    }

    fn cache_outbox_read_max_id(&self, chat_id: i64, max_message_id: i32) -> Result<()> {
        cache_outbox_read_max_id(&self.outbox_read_max_id_cache, chat_id, max_message_id)
    }

    fn cached_outbox_read_max_id(&self, chat_id: i64) -> Result<Option<i32>> {
        Ok(self.outbox_read_max_id_cache()?.get(&chat_id).copied())
    }

    fn cache_dialog_read_state(&self, dialog: &grammers_client::types::Dialog) -> Result<()> {
        if let Some(max_message_id) = dialog_outbox_read_max_id(dialog) {
            self.cache_outbox_read_max_id(dialog.chat().id(), max_message_id)?;
        }
        Ok(())
    }

    async fn load_chats(
        &self,
        folder_id: Option<i32>,
        limit: usize,
    ) -> Result<ReconciliationChatList> {
        if limit == 0 {
            return Ok(ReconciliationChatList {
                chats: Vec::new(),
                last_message_ids: HashMap::new(),
            });
        }

        let dialog_filter = if let Some(folder_id) = folder_id {
            self.cached_dialog_filter(folder_id)?
        } else {
            None
        };
        let mut iter = self.client.iter_dialogs();
        let mut chats = Vec::new();
        let mut last_message_ids = HashMap::new();

        while let Some(dialog) = iter.next().await? {
            if chats.len() >= limit {
                break;
            }
            let dialog_folder_id = dialog_folder_id(&dialog);
            let matches_selected_folder = match (folder_id, dialog_filter.as_ref()) {
                (None, _) => true,
                (Some(_), Some(filter)) => dialog_matches_filter(&dialog, filter),
                (Some(folder_id), None) => dialog_folder_id == Some(folder_id),
            };
            if !matches_selected_folder {
                continue;
            }

            let chat = dialog.chat();
            self.cache_chat(chat.clone())?;
            self.cache_dialog_read_state(&dialog)?;
            if let Some(message) = dialog.last_message.as_ref() {
                last_message_ids.insert(chat.id(), message.id());
            }
            chats.push(Chat {
                id: chat.id(),
                name: chat.name().to_string(),
                last_message: dialog
                    .last_message
                    .as_ref()
                    .map(message_preview_from_grammers_message),
                unread_count: dialog_unread_count(&dialog),
                is_group: matches!(chat, grammers_client::types::Chat::Group(_)),
                folder_id: folder_id.or(dialog_folder_id),
            });
        }

        Ok(ReconciliationChatList {
            chats,
            last_message_ids,
        })
    }
}

fn dumbgram_init_params() -> InitParams {
    InitParams {
        device_model: "Dumbgram TUI".to_string(),
        system_version: env!("CARGO_PKG_VERSION").to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        // Startup and folder/chat loads already fetch the recent visible history. Replaying
        // offline update backlog makes hours-old messages appear as new live messages.
        catch_up: false,
        reconnection_policy: &TELEGRAM_RECONNECT_POLICY,
        update_queue_limit: Some(100),
        flood_sleep_threshold: 60,
        ..Default::default()
    }
}

fn chat_not_found_in_cache_message(chat_id: i64) -> String {
    format!("{CHAT_NOT_FOUND_IN_CACHE_PREFIX}: {chat_id}")
}

fn chat_cache_lock_failed_message(error: impl std::fmt::Display) -> String {
    format!("{CHAT_CACHE_LOCK_FAILED}: {error}")
}

fn update_error_message(error: impl std::fmt::Display) -> String {
    format!("{UPDATE_ERROR_PREFIX}: {error}")
}

fn delete_update_chat_id(channel_id: Option<i64>) -> i64 {
    channel_id.unwrap_or(UNKNOWN_DELETE_UPDATE_CHAT_ID)
}

fn delete_message_updates(channel_id: Option<i64>, message_ids: &[i32]) -> Vec<Update> {
    let chat_id = delete_update_chat_id(channel_id);
    message_ids
        .iter()
        .map(|message_id| Update::DeleteMessage {
            chat_id,
            message_id: *message_id,
        })
        .collect()
}

fn chat_id_from_peer(peer: &tl::enums::Peer) -> i64 {
    match peer {
        tl::enums::Peer::User(peer) => peer.user_id,
        tl::enums::Peer::Chat(peer) => peer.chat_id,
        tl::enums::Peer::Channel(peer) => peer.channel_id,
    }
}

fn read_outbox_update_from_raw(update: &tl::enums::Update) -> Option<Update> {
    match update {
        tl::enums::Update::ReadHistoryOutbox(update) => Some(Update::ReadOutgoingMessages {
            chat_id: chat_id_from_peer(&update.peer),
            max_message_id: update.max_id,
        }),
        tl::enums::Update::ReadChannelOutbox(update) => Some(Update::ReadOutgoingMessages {
            chat_id: update.channel_id,
            max_message_id: update.max_id,
        }),
        _ => None,
    }
}

fn cached_user_name(cache: &Arc<Mutex<UserNameCache>>, user_id: i64) -> Option<String> {
    cache.lock().ok()?.get(&user_id).cloned()
}

fn cache_user_name_from_chat(
    cache: &Arc<Mutex<UserNameCache>>,
    chat: &grammers_client::types::Chat,
) {
    let grammers_client::types::Chat::User(_) = chat else {
        return;
    };
    let name = chat.name().trim();
    if name.is_empty() {
        return;
    }
    if let Ok(mut cache) = cache.lock() {
        cache.insert(chat.id(), name.to_string());
    }
}

fn cache_sender_name_from_message(
    cache: &Arc<Mutex<UserNameCache>>,
    message: &grammers_client::types::Message,
) {
    if let Some(sender) = message.sender() {
        cache_user_name_from_chat(cache, &sender);
    }
}

fn typing_user_label(peer: &tl::enums::Peer, user_name: Option<String>) -> String {
    match peer {
        tl::enums::Peer::User(peer) => {
            user_name.unwrap_or_else(|| format!("User {}", peer.user_id))
        }
        tl::enums::Peer::Chat(peer) => format!("Chat {}", peer.chat_id),
        tl::enums::Peer::Channel(peer) => format!("Channel {}", peer.channel_id),
    }
}

fn send_action_is_typing(action: &tl::enums::SendMessageAction) -> bool {
    !matches!(
        action,
        tl::enums::SendMessageAction::SendMessageCancelAction
    )
}

fn typing_status_update_from_raw_with_user_names(
    update: &tl::enums::Update,
    user_name_for_id: impl Fn(i64) -> Option<String>,
) -> Option<Update> {
    match update {
        tl::enums::Update::ChannelUserTyping(update) => {
            let user_name = match &update.from_id {
                tl::enums::Peer::User(peer) => user_name_for_id(peer.user_id),
                _ => None,
            };
            Some(Update::TypingStatus {
                chat_id: update.channel_id,
                topic_id: update.top_msg_id,
                user_name: typing_user_label(&update.from_id, user_name),
                is_typing: send_action_is_typing(&update.action),
            })
        }
        tl::enums::Update::ChatUserTyping(update) => {
            let user_name = match &update.from_id {
                tl::enums::Peer::User(peer) => user_name_for_id(peer.user_id),
                _ => None,
            };
            Some(Update::TypingStatus {
                chat_id: update.chat_id,
                topic_id: None,
                user_name: typing_user_label(&update.from_id, user_name),
                is_typing: send_action_is_typing(&update.action),
            })
        }
        tl::enums::Update::UserTyping(update) => Some(Update::TypingStatus {
            chat_id: update.user_id,
            topic_id: None,
            user_name: user_name_for_id(update.user_id)
                .unwrap_or_else(|| format!("User {}", update.user_id)),
            is_typing: send_action_is_typing(&update.action),
        }),
        _ => None,
    }
}

#[cfg(test)]
fn typing_status_update_from_raw(update: &tl::enums::Update) -> Option<Update> {
    typing_status_update_from_raw_with_user_names(update, |_| None)
}

fn non_empty_text(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

fn cache_outbox_read_max_id(
    cache: &Arc<Mutex<OutboxReadMaxIdCache>>,
    chat_id: i64,
    max_message_id: i32,
) -> Result<()> {
    let mut cache = cache
        .lock()
        .map_err(|error| color_eyre::eyre::eyre!(chat_cache_lock_failed_message(error)))?;
    let entry = cache.entry(chat_id).or_insert(max_message_id);
    *entry = (*entry).max(max_message_id);
    Ok(())
}

fn message_status_for_read_state(
    is_outgoing: bool,
    message_id: i32,
    outbox_read_max_id: Option<i32>,
) -> MessageStatus {
    if is_outgoing {
        if outbox_read_max_id.is_some_and(|max_id| message_id <= max_id) {
            MessageStatus::Read
        } else {
            MessageStatus::Sent
        }
    } else {
        MessageStatus::Delivered
    }
}

fn message_sender_name(
    is_outgoing: bool,
    sender_name: Option<String>,
    post_author: Option<String>,
    chat_name: Option<String>,
) -> String {
    if is_outgoing {
        OWN_SENDER_NAME.to_string()
    } else {
        non_empty_text(sender_name)
            .or_else(|| non_empty_text(post_author))
            .or_else(|| non_empty_text(chat_name))
            .unwrap_or_else(|| UNKNOWN_SENDER_NAME.to_string())
    }
}

fn grammers_chat_is_forum(chat: &grammers_client::types::Chat) -> bool {
    match chat {
        grammers_client::types::Chat::Group(group) => match &group.raw {
            tl::enums::Chat::Channel(channel) => channel.forum,
            _ => false,
        },
        _ => false,
    }
}

fn thread_topic_from_tl(topic: tl::enums::ForumTopic) -> Option<ThreadTopic> {
    match topic {
        tl::enums::ForumTopic::Topic(topic) => Some(ThreadTopic {
            id: topic.id,
            title: topic.title,
            top_message_id: topic.top_message,
            unread_count: topic.unread_count.max(0) as usize,
            is_closed: topic.closed,
            is_pinned: topic.pinned,
        }),
        tl::enums::ForumTopic::Deleted(_) => None,
    }
}

const GENERAL_FORUM_TOPIC_ID: i32 = 1;

fn message_thread_topic_id(
    reply_to: Option<&tl::enums::MessageReplyHeader>,
    chat_is_forum: bool,
) -> Option<i32> {
    if !chat_is_forum {
        return None;
    }
    match reply_to {
        Some(tl::enums::MessageReplyHeader::Header(header)) if header.forum_topic => {
            header.reply_to_top_id.or(header.reply_to_msg_id)
        }
        _ => Some(GENERAL_FORUM_TOPIC_ID),
    }
}

fn convert_message(
    msg: grammers_client::types::Message,
    outbox_read_max_id: Option<i32>,
) -> Message {
    let is_outgoing = msg.outgoing();
    let chat = msg.chat();
    let media = message_media(msg.media().as_ref());
    let thread_topic_id =
        message_thread_topic_id(msg.raw.reply_to.as_ref(), grammers_chat_is_forum(&chat));
    Message {
        id: msg.id(),
        chat_id: chat.id(),
        thread_topic_id,
        sender_name: message_sender_name(
            is_outgoing,
            msg.sender().map(|s| s.name().to_string()),
            msg.raw.post_author.clone(),
            Some(chat.name().to_string()),
        ),
        content: msg.text().to_string(),
        timestamp: msg.date(),
        is_own: is_outgoing,
        is_edited: msg.edit_date().is_some(),
        reply_to_content: None,
        media,
        status: message_status_for_read_state(is_outgoing, msg.id(), outbox_read_max_id),
        can_edit: is_outgoing && is_within_edit_window(msg.date()),
        can_delete: is_outgoing,
        error: None,
    }
}

fn canonical_session_identity(session_path: &Path) -> io::Result<PathBuf> {
    if session_path.exists() {
        return session_path.canonicalize();
    }
    let file_name = session_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "session path has no file name")
    })?;
    let parent = session_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(parent.canonicalize()?.join(file_name))
}

#[cfg(unix)]
fn ensure_trusted_cache_parent(parent: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    for ancestor in parent.ancestors() {
        let metadata = std::fs::metadata(ancestor)?;
        if !metadata.is_dir() || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "media cache parent is writable by untrusted users: {}",
                    ancestor.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_trusted_cache_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

fn ensure_private_cache_dir(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "media cache path is not a regular directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => std::fs::create_dir(path)?,
        Err(error) => return Err(error),
    }
    set_private_permissions(path, 0o700)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "media cache path changed during setup",
        ));
    }
    Ok(())
}

fn media_cache_dir(session_path: &Path) -> io::Result<PathBuf> {
    let session_identity = canonical_session_identity(session_path)?;
    let parent = session_identity
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "session path has no parent"))?;
    ensure_trusted_cache_parent(parent)?;
    let mut cache_name = session_identity
        .file_name()
        .map(OsString::from)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "session path has no file name")
        })?;
    cache_name.push(".dumbgram-media-cache");
    let cache_dir = parent.join(cache_name);
    ensure_private_cache_dir(&cache_dir)?;
    Ok(cache_dir)
}

fn message_media(media: Option<&grammers_client::types::Media>) -> Option<MessageMedia> {
    use grammers_client::types::Media;

    match media? {
        Media::Photo(_) => Some(MessageMedia::photo()),
        Media::Document(document)
            if document
                .mime_type()
                .is_some_and(|mime| mime.starts_with("image/")) =>
        {
            Some(MessageMedia::image())
        }
        Media::Document(document)
            if document
                .mime_type()
                .is_some_and(|mime| mime.starts_with("video/")) =>
        {
            Some(MessageMedia::new(MessageMediaKind::Video, "[video]"))
        }
        Media::Sticker(_) => Some(MessageMedia::new(MessageMediaKind::Sticker, "[sticker]")),
        Media::Document(_) => Some(MessageMedia::new(MessageMediaKind::Document, "[document]")),
        Media::WebPage(_) => Some(MessageMedia::new(MessageMediaKind::WebPage, "[web page]")),
        Media::Geo(_) => Some(MessageMedia::new(MessageMediaKind::Other, "[location]")),
        Media::GeoLive(_) => Some(MessageMedia::new(
            MessageMediaKind::Other,
            "[live location]",
        )),
        Media::Contact(_) => Some(MessageMedia::new(MessageMediaKind::Other, "[contact]")),
        Media::Poll(_) => Some(MessageMedia::new(MessageMediaKind::Other, "[poll]")),
        Media::Dice(_) => Some(MessageMedia::new(MessageMediaKind::Other, "[dice]")),
        Media::Venue(_) => Some(MessageMedia::new(MessageMediaKind::Other, "[venue]")),
        _ => Some(MessageMedia::new(MessageMediaKind::Other, "[media]")),
    }
}

fn media_thumbnail_cache_path(cache_dir: &Path, chat_id: i64, message_id: i32) -> PathBuf {
    cache_dir.join(format!("chat-{chat_id}-message-{message_id}-thumb.jpg"))
}

fn cached_media_thumbnail_path(cache_dir: &Path, chat_id: i64, message_id: i32) -> Option<PathBuf> {
    let path = media_thumbnail_cache_path(cache_dir, chat_id, message_id);
    std::fs::symlink_metadata(&path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())?;
    set_private_permissions(&path, 0o600).ok()?;
    Some(path)
}

fn create_private_download_dir_with(
    cache_dir: &Path,
    mut next_id: impl FnMut() -> u64,
) -> io::Result<PathBuf> {
    for _ in 0..100 {
        let candidate = cache_dir.join(format!(".download-{}-{}", std::process::id(), next_id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => {
                if let Err(error) = set_private_permissions(&candidate, 0o700) {
                    let _ = std::fs::remove_dir(&candidate);
                    return Err(error);
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a private media download directory",
    ))
}

fn create_private_download_dir(cache_dir: &Path) -> io::Result<PathBuf> {
    create_private_download_dir_with(cache_dir, || {
        MEDIA_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    })
}

fn cleanup_private_download(temp_dir: &Path, temp_file: &Path) {
    if std::fs::remove_file(temp_file).is_err() {
        let _ = std::fs::remove_dir(temp_file);
    }
    let _ = std::fs::remove_dir(temp_dir);
}

fn complete_private_download(
    temp_dir: &Path,
    temp_file: &Path,
    final_path: &Path,
    download_result: io::Result<()>,
) -> io::Result<PathBuf> {
    let result = download_result.and_then(|()| {
        publish_downloaded_thumbnail(temp_file, final_path)?;
        Ok(final_path.to_path_buf())
    });
    cleanup_private_download(temp_dir, temp_file);
    result
}

fn publish_downloaded_thumbnail(temp_file: &Path, final_path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(temp_file)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "downloaded thumbnail is not a regular file",
        ));
    }
    set_private_permissions(temp_file, 0o600)?;
    std::fs::rename(temp_file, final_path)?;
    let published = std::fs::symlink_metadata(final_path)?;
    if !published.file_type().is_file() || published.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "published thumbnail is not a regular file",
        ));
    }
    Ok(())
}

async fn download_media_thumbnail(
    client: &Client,
    cache_dir: &Path,
    chat_id: i64,
    message_id: i32,
    media: &grammers_client::types::Media,
) -> std::io::Result<Option<PathBuf>> {
    let thumbnail = media_thumbnail(media);
    let Some(thumbnail) = thumbnail else {
        return Ok(None);
    };
    if let Some(path) = cached_media_thumbnail_path(cache_dir, chat_id, message_id) {
        return Ok(Some(path));
    }

    ensure_private_cache_dir(cache_dir)?;
    let path = media_thumbnail_cache_path(cache_dir, chat_id, message_id);
    let temp_dir = create_private_download_dir(cache_dir)?;
    let temp_file = temp_dir.join("thumbnail");
    let download_result = client
        .download_media(&Downloadable::PhotoSize(thumbnail), &temp_file)
        .await
        .map(|_| ());
    complete_private_download(&temp_dir, &temp_file, &path, download_result).map(Some)
}

fn media_download_file_name(
    chat_id: i64,
    message_id: i32,
    media: &grammers_client::types::Media,
) -> String {
    use grammers_client::types::Media;

    match media {
        Media::Photo(_) => format!("dumbgram-chat-{chat_id}-message-{message_id}.jpg"),
        Media::Document(document) => {
            sanitize_download_file_name(document.name()).unwrap_or_else(|| {
                let extension = document
                    .mime_type()
                    .and_then(download_extension_for_mime)
                    .unwrap_or("bin");
                format!("dumbgram-chat-{chat_id}-message-{message_id}.{extension}")
            })
        }
        _ => format!("dumbgram-chat-{chat_id}-message-{message_id}.bin"),
    }
}

fn download_extension_for_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "application/pdf" => Some("pdf"),
        "text/plain" => Some("txt"),
        _ => None,
    }
}

fn sanitize_download_file_name(name: &str) -> Option<String> {
    let sanitized = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '\0' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .chars()
        .take(120)
        .collect::<String>();

    (!sanitized.is_empty()).then_some(sanitized)
}

fn available_download_path(destination_dir: &Path, file_name: &str) -> PathBuf {
    let candidate = destination_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("dumbgram-media");
    let extension = path.extension().and_then(|extension| extension.to_str());

    for copy_index in 1.. {
        let candidate_name = if let Some(extension) = extension {
            format!("{stem}-{copy_index}.{extension}")
        } else {
            format!("{stem}-{copy_index}")
        };
        let candidate = destination_dir.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded copy-index loop must return an available path")
}

fn media_thumbnail(media: &grammers_client::types::Media) -> Option<PhotoSize> {
    use grammers_client::types::Media;

    let mut thumbs = match media {
        Media::Photo(photo) => photo.thumbs(),
        Media::Document(document) => document.thumbs(),
        _ => Vec::new(),
    };
    thumbs.sort_by_key(PhotoSize::size);
    thumbs.pop()
}

fn message_preview_from_grammers_message(message: &grammers_client::types::Message) -> String {
    message_display_preview(
        message_media(message.media().as_ref()).as_ref(),
        message.text(),
    )
}

fn is_within_edit_window(date: DateTime<Utc>) -> bool {
    let now = chrono::Utc::now();
    (now - date).num_hours() < 48
}

fn dialog_outbox_read_max_id(dialog: &grammers_client::types::Dialog) -> Option<i32> {
    match &dialog.raw {
        tl::enums::Dialog::Dialog(raw) => Some(raw.read_outbox_max_id),
        tl::enums::Dialog::Folder(_) => None,
    }
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

fn dialog_folder_metadata(dialog: &grammers_client::types::Dialog) -> Option<(i32, String)> {
    match &dialog.raw {
        tl::enums::Dialog::Folder(raw) => {
            let folder: tl::types::Folder = raw.folder.clone().into();
            Some((folder.id, folder.title))
        }
        tl::enums::Dialog::Dialog(_) => None,
    }
}

#[cfg(test)]
struct DialogPageParts {
    dialogs: Vec<tl::enums::Dialog>,
    messages: Vec<tl::enums::Message>,
    chats: Vec<tl::enums::Chat>,
    users: Vec<tl::enums::User>,
}

#[cfg(test)]
fn peers_match(left: &tl::enums::Peer, right: &tl::enums::Peer) -> bool {
    match (left, right) {
        (tl::enums::Peer::User(left), tl::enums::Peer::User(right)) => {
            left.user_id == right.user_id
        }
        (tl::enums::Peer::Chat(left), tl::enums::Peer::Chat(right)) => {
            left.chat_id == right.chat_id
        }
        (tl::enums::Peer::Channel(left), tl::enums::Peer::Channel(right)) => {
            left.channel_id == right.channel_id
        }
        _ => false,
    }
}

#[cfg(test)]
fn raw_message_preview_for_dialog(
    messages: &[tl::enums::Message],
    peer: &tl::enums::Peer,
    top_message_id: i32,
) -> Option<String> {
    messages.iter().find_map(|message| match message {
        tl::enums::Message::Message(message)
            if message.id == top_message_id && peers_match(&message.peer_id, peer) =>
        {
            Some(super::types::message_preview(&message.message))
        }
        tl::enums::Message::Service(message)
            if message.id == top_message_id && peers_match(&message.peer_id, peer) =>
        {
            Some(super::types::message_preview(""))
        }
        _ => None,
    })
}

#[cfg(test)]
fn dialog_chats_from_page_parts(
    parts: DialogPageParts,
    requested_folder_id: Option<i32>,
    limit: usize,
) -> Vec<(Chat, grammers_client::types::Chat)> {
    let chat_map = ChatMap::new(parts.users, parts.chats);
    let mut chats = Vec::new();

    for dialog in &parts.dialogs {
        if chats.len() >= limit {
            break;
        }
        let tl::enums::Dialog::Dialog(dialog) = dialog else {
            continue;
        };
        let Some(chat) = chat_map.get(&dialog.peer).cloned() else {
            continue;
        };
        let folder_id = dialog.folder_id.or(requested_folder_id);

        chats.push((
            Chat {
                id: chat.id(),
                name: chat.name().to_string(),
                last_message: raw_message_preview_for_dialog(
                    &parts.messages,
                    &dialog.peer,
                    dialog.top_message,
                ),
                unread_count: dialog.unread_count.max(0) as usize,
                is_group: matches!(chat, grammers_client::types::Chat::Group(_)),
                folder_id,
            },
            chat,
        ));
    }

    chats
}

fn folder_from_dialog_filter(filter: &tl::enums::DialogFilter) -> Option<Folder> {
    match filter {
        tl::enums::DialogFilter::Filter(filter) if filter.id > 0 => Some(Folder {
            id: filter.id,
            name: filter.title.clone(),
            unread_count: 0,
        }),
        tl::enums::DialogFilter::Chatlist(filter) if filter.id > 0 => Some(Folder {
            id: filter.id,
            name: filter.title.clone(),
            unread_count: 0,
        }),
        _ => None,
    }
}

fn input_peer_matches_chat(
    peer: &tl::enums::InputPeer,
    chat: &grammers_client::types::Chat,
) -> bool {
    match (peer, chat) {
        (tl::enums::InputPeer::PeerSelf, grammers_client::types::Chat::User(user)) => {
            user.is_self()
        }
        (tl::enums::InputPeer::User(peer), grammers_client::types::Chat::User(user)) => {
            peer.user_id == user.id()
        }
        (tl::enums::InputPeer::Chat(peer), grammers_client::types::Chat::Group(group)) => {
            peer.chat_id == group.id()
        }
        (tl::enums::InputPeer::Channel(peer), grammers_client::types::Chat::Group(group)) => {
            peer.channel_id == group.id()
        }
        (tl::enums::InputPeer::Channel(peer), grammers_client::types::Chat::Channel(channel)) => {
            peer.channel_id == channel.id()
        }
        _ => false,
    }
}

fn input_peers_contain_chat(
    peers: &[tl::enums::InputPeer],
    chat: &grammers_client::types::Chat,
) -> bool {
    peers.iter().any(|peer| input_peer_matches_chat(peer, chat))
}

fn chat_matches_filter_categories(
    filter: &tl::types::DialogFilter,
    chat: &grammers_client::types::Chat,
) -> bool {
    match chat {
        grammers_client::types::Chat::User(user) => {
            (filter.contacts && user.contact())
                || (filter.non_contacts && !user.contact() && !user.is_bot())
                || (filter.bots && user.is_bot())
        }
        grammers_client::types::Chat::Group(_) => filter.groups,
        grammers_client::types::Chat::Channel(_) => filter.broadcasts,
    }
}

fn dialog_matches_filter(
    dialog: &grammers_client::types::Dialog,
    filter: &tl::enums::DialogFilter,
) -> bool {
    let chat = dialog.chat();
    match filter {
        tl::enums::DialogFilter::Filter(filter) => {
            if filter.exclude_read && dialog_unread_count(dialog) == 0 {
                return false;
            }
            if filter.exclude_archived && dialog_folder_id(dialog) == Some(1) {
                return false;
            }
            if input_peers_contain_chat(&filter.exclude_peers, chat) {
                return false;
            }
            input_peers_contain_chat(&filter.pinned_peers, chat)
                || input_peers_contain_chat(&filter.include_peers, chat)
                || chat_matches_filter_categories(filter, chat)
        }
        tl::enums::DialogFilter::Chatlist(filter) => {
            input_peers_contain_chat(&filter.pinned_peers, chat)
                || input_peers_contain_chat(&filter.include_peers, chat)
        }
        _ => false,
    }
}

fn folders_from_dialog_filters(
    filters: Vec<tl::enums::DialogFilter>,
    all_unread_count: usize,
    mut folder_unread_counts: HashMap<i32, usize>,
    mut folder_names: HashMap<i32, String>,
) -> Vec<Folder> {
    let mut folders = vec![all_folder(all_unread_count)];
    let mut seen_folder_ids = HashSet::new();

    for filter in filters {
        let Some(mut folder) = folder_from_dialog_filter(&filter) else {
            continue;
        };
        if !seen_folder_ids.insert(folder.id) {
            continue;
        }

        folder.unread_count = folder_unread_counts.remove(&folder.id).unwrap_or_default();
        folder_names.remove(&folder.id);
        folders.push(folder);
    }

    let mut fallback_folders = folder_unread_counts.into_iter().collect::<Vec<_>>();
    fallback_folders.sort_by_key(|(folder_id, _)| *folder_id);
    for (folder_id, unread_count) in fallback_folders {
        if seen_folder_ids.insert(folder_id) {
            folders.push(Folder {
                id: folder_id,
                name: folder_names
                    .remove(&folder_id)
                    .unwrap_or_else(|| format!("Folder {folder_id}")),
                unread_count,
            });
        }
    }

    folders
}

async fn collect_message_page(
    user_name_cache: &Arc<Mutex<UserNameCache>>,
    mut iter: grammers_client::client::messages::MessageIter,
    chat_id: i64,
    outbox_read_max_id: Option<i32>,
    limit: usize,
    direction: &str,
) -> Result<Vec<Message>> {
    let mut messages = Vec::new();

    while let Some(msg) = iter.next().await? {
        cache_sender_name_from_message(user_name_cache, &msg);
        messages.push(convert_message(msg, outbox_read_max_id));
        if messages.len() % 10 == 0 || messages.len() >= limit {
            diagnostics::event(
                "message_iter_progress",
                format!(
                    "chat_id={chat_id} direction={direction} count={} limit={limit}",
                    messages.len()
                ),
            );
        }
        if messages.len() >= limit {
            break;
        }
    }

    messages.reverse();
    Ok(messages)
}

fn messages_response_parts(
    response: tl::enums::messages::Messages,
) -> (
    Vec<tl::enums::Message>,
    Vec<tl::enums::Chat>,
    Vec<tl::enums::User>,
) {
    match response {
        tl::enums::messages::Messages::Messages(messages) => {
            (messages.messages, messages.chats, messages.users)
        }
        tl::enums::messages::Messages::Slice(messages) => {
            (messages.messages, messages.chats, messages.users)
        }
        tl::enums::messages::Messages::ChannelMessages(messages) => {
            (messages.messages, messages.chats, messages.users)
        }
        tl::enums::messages::Messages::NotModified(_) => (Vec::new(), Vec::new(), Vec::new()),
    }
}

fn topic_send_random_id() -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as i64);
    nanos ^ i64::from(std::process::id())
}

fn sent_message_id_from_updates(updates: &tl::enums::Updates, random_id: i64) -> Option<i32> {
    fn from_update(update: &tl::enums::Update, random_id: i64) -> Option<i32> {
        match update {
            tl::enums::Update::MessageId(update) if update.random_id == random_id => {
                Some(update.id)
            }
            tl::enums::Update::NewMessage(update) => raw_message_id(&update.message),
            tl::enums::Update::NewChannelMessage(update) => raw_message_id(&update.message),
            tl::enums::Update::NewScheduledMessage(update) => raw_message_id(&update.message),
            _ => None,
        }
    }

    match updates {
        tl::enums::Updates::UpdateShortSentMessage(update) => Some(update.id),
        tl::enums::Updates::Updates(updates) => updates
            .updates
            .iter()
            .find_map(|update| from_update(update, random_id)),
        tl::enums::Updates::Combined(updates) => updates
            .updates
            .iter()
            .find_map(|update| from_update(update, random_id)),
        _ => None,
    }
}

fn raw_message_id(message: &tl::enums::Message) -> Option<i32> {
    match message {
        tl::enums::Message::Message(message) => Some(message.id),
        tl::enums::Message::Service(message) => Some(message.id),
        tl::enums::Message::Empty(_) => None,
    }
}

async fn collect_raw_message_page(
    client: &Client,
    user_name_cache: &Arc<Mutex<UserNameCache>>,
    response: tl::enums::messages::Messages,
    chat_id: i64,
    outbox_read_max_id: Option<i32>,
    limit: usize,
    direction: &str,
) -> Vec<Message> {
    let (raw_messages, chats, users) = messages_response_parts(response);
    let chat_map = Arc::new(ChatMap::new(users, chats));
    let mut messages = Vec::new();

    for raw_message in raw_messages.into_iter().take(limit) {
        if let Some(message) =
            grammers_client::types::Message::from_raw(client, raw_message, &chat_map)
        {
            cache_sender_name_from_message(user_name_cache, &message);
            messages.push(convert_message(message, outbox_read_max_id));
        }
    }

    diagnostics::event(
        "message_iter_progress",
        format!(
            "chat_id={chat_id} direction={direction} count={} limit={limit}",
            messages.len()
        ),
    );
    messages.reverse();
    messages
}

impl TelegramClient for GrammersClient {
    async fn connect(&mut self) -> Result<()> {
        Ok(())
    }

    #[allow(clippy::manual_async_fn)]
    fn get_thread_topics(
        &self,
        chat_id: i64,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<ThreadTopic>>> + Send + '_ {
        async move {
            if limit == 0 {
                return Ok(Vec::new());
            }

            let chat = self.cached_chat(chat_id)?;
            if !grammers_chat_is_forum(&chat) {
                return Ok(Vec::new());
            }

            let Some(channel) = chat.pack().try_to_input_channel() else {
                return Ok(Vec::new());
            };

            let topics = self
                .client
                .invoke(&tl::functions::channels::GetForumTopics {
                    channel,
                    q: None,
                    offset_date: 0,
                    offset_id: 0,
                    offset_topic: 0,
                    limit: limit.min(i32::MAX as usize) as i32,
                })
                .await?;
            let tl::enums::messages::ForumTopics::Topics(topics) = topics;
            Ok(topics
                .topics
                .into_iter()
                .filter_map(thread_topic_from_tl)
                .collect())
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn get_thread_topic(
        &self,
        chat_id: i64,
        topic_id: i32,
    ) -> impl std::future::Future<Output = Result<Option<ThreadTopic>>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            if !grammers_chat_is_forum(&chat) {
                return Ok(None);
            }
            let Some(channel) = chat.pack().try_to_input_channel() else {
                return Ok(None);
            };

            let topics = self
                .client
                .invoke(&tl::functions::channels::GetForumTopicsById {
                    channel,
                    topics: vec![topic_id],
                })
                .await?;
            let tl::enums::messages::ForumTopics::Topics(topics) = topics;
            Ok(topics
                .topics
                .into_iter()
                .find_map(|topic| thread_topic_from_tl(topic).filter(|topic| topic.id == topic_id)))
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn get_thread_messages(
        &self,
        chat_id: i64,
        topic_id: i32,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
        async move {
            if limit == 0 {
                return Ok(Vec::new());
            }

            let chat = self.cached_chat(chat_id)?;
            let outbox_read_max_id = self.cached_outbox_read_max_id(chat_id)?;
            let response = self
                .client
                .invoke(&tl::functions::messages::GetReplies {
                    peer: chat.pack().to_input_peer(),
                    msg_id: topic_id,
                    offset_id: 0,
                    offset_date: 0,
                    add_offset: 0,
                    limit: limit.min(i32::MAX as usize) as i32,
                    max_id: 0,
                    min_id: 0,
                    hash: 0,
                })
                .await?;

            Ok(collect_raw_message_page(
                &self.client,
                &self.user_name_cache,
                response,
                chat_id,
                outbox_read_max_id,
                limit,
                "thread",
            )
            .await)
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
            if limit == 0 {
                return Ok(Vec::new());
            }

            let chat = self.cached_chat(chat_id)?;
            let outbox_read_max_id = self.cached_outbox_read_max_id(chat_id)?;
            let response = self
                .client
                .invoke(&tl::functions::messages::GetReplies {
                    peer: chat.pack().to_input_peer(),
                    msg_id: topic_id,
                    offset_id: before_message_id,
                    offset_date: 0,
                    add_offset: 0,
                    limit: limit.min(i32::MAX as usize) as i32,
                    max_id: 0,
                    min_id: 0,
                    hash: 0,
                })
                .await?;

            Ok(collect_raw_message_page(
                &self.client,
                &self.user_name_cache,
                response,
                chat_id,
                outbox_read_max_id,
                limit,
                "older_thread",
            )
            .await)
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn load_message_media_preview(
        &self,
        chat_id: i64,
        message_id: i32,
    ) -> impl std::future::Future<Output = Result<Option<PathBuf>>> + Send + '_ {
        async move {
            if let Some(path) =
                cached_media_thumbnail_path(&self.media_cache_dir, chat_id, message_id)
            {
                diagnostics::event(
                    "media_preview_cache_hit",
                    format!("chat_id={chat_id} message_id={message_id}"),
                );
                return Ok(Some(path));
            }

            let chat = self.cached_chat(chat_id)?;
            let mut messages = self.client.get_messages_by_id(&chat, &[message_id]).await?;
            let Some(message) = messages.pop().flatten() else {
                return Ok(None);
            };
            let Some(media) = message.media() else {
                return Ok(None);
            };
            if !matches!(
                message_media(Some(&media)).map(|media| media.kind),
                Some(MessageMediaKind::Photo | MessageMediaKind::Image)
            ) {
                return Ok(None);
            }
            Ok(download_media_thumbnail(
                &self.client,
                &self.media_cache_dir,
                chat_id,
                message_id,
                &media,
            )
            .await?)
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn download_message_media(
        &self,
        chat_id: i64,
        message_id: i32,
        destination_dir: PathBuf,
    ) -> impl std::future::Future<Output = Result<DownloadedMedia>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            std::fs::create_dir_all(&destination_dir)?;
            let mut messages = self.client.get_messages_by_id(&chat, &[message_id]).await?;
            let message = messages
                .pop()
                .flatten()
                .ok_or_else(|| color_eyre::eyre::eyre!("Message not found"))?;
            let media = message
                .media()
                .ok_or_else(|| color_eyre::eyre::eyre!("No downloadable media"))?;
            let path = available_download_path(
                &destination_dir,
                &media_download_file_name(chat_id, message_id, &media),
            );
            self.client
                .download_media(&Downloadable::Media(media), &path)
                .await?;
            let bytes = std::fs::metadata(&path).map(|metadata| metadata.len())?;
            Ok(DownloadedMedia { path, bytes })
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn send_message(
        &self,
        chat_id: i64,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            let msg = self
                .client
                .send_message(chat, InputMessage::text(content))
                .await?;
            Ok(convert_message(msg, None))
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
            let chat = self.cached_chat(chat_id)?;
            let random_id = topic_send_random_id();
            let updates = self
                .client
                .invoke(&tl::functions::messages::SendMessage {
                    no_webpage: true,
                    silent: false,
                    background: false,
                    clear_draft: false,
                    noforwards: false,
                    update_stickersets_order: false,
                    invert_media: false,
                    peer: chat.pack().to_input_peer(),
                    reply_to: Some(
                        tl::types::InputReplyToMessage {
                            reply_to_msg_id: topic_id,
                            top_msg_id: Some(topic_id),
                            reply_to_peer_id: None,
                            quote_text: None,
                            quote_entities: None,
                            quote_offset: None,
                        }
                        .into(),
                    ),
                    message: content,
                    random_id,
                    reply_markup: None,
                    entities: None,
                    schedule_date: None,
                    send_as: None,
                    quick_reply_shortcut: None,
                    effect: None,
                })
                .await?;
            let message_id = sent_message_id_from_updates(&updates, random_id)
                .ok_or_else(|| color_eyre::eyre::eyre!("Sent topic message id not found"))?;
            let mut messages = self.client.get_messages_by_id(&chat, &[message_id]).await?;
            let message = messages
                .pop()
                .flatten()
                .ok_or_else(|| color_eyre::eyre::eyre!("Sent topic message not found"))?;
            Ok(convert_message(message, None))
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn edit_message(
        &self,
        chat_id: i64,
        message_id: i32,
        content: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            self.client
                .edit_message(chat, message_id, InputMessage::text(content))
                .await?;
            Ok(())
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn reply_to_message(
        &self,
        chat_id: i64,
        reply_to: i32,
        content: String,
    ) -> impl std::future::Future<Output = Result<Message>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            let input = InputMessage::text(content).reply_to(Some(reply_to));
            let msg = self.client.send_message(chat, input).await?;
            Ok(convert_message(msg, None))
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
            let chat = self.cached_chat(chat_id)?;
            let random_id = topic_send_random_id();
            let updates = self
                .client
                .invoke(&tl::functions::messages::SendMessage {
                    no_webpage: true,
                    silent: false,
                    background: false,
                    clear_draft: false,
                    noforwards: false,
                    update_stickersets_order: false,
                    invert_media: false,
                    peer: chat.pack().to_input_peer(),
                    reply_to: Some(
                        tl::types::InputReplyToMessage {
                            reply_to_msg_id: reply_to,
                            top_msg_id: Some(topic_id),
                            reply_to_peer_id: None,
                            quote_text: None,
                            quote_entities: None,
                            quote_offset: None,
                        }
                        .into(),
                    ),
                    message: content,
                    random_id,
                    reply_markup: None,
                    entities: None,
                    schedule_date: None,
                    send_as: None,
                    quick_reply_shortcut: None,
                    effect: None,
                })
                .await?;
            let message_id = sent_message_id_from_updates(&updates, random_id)
                .ok_or_else(|| color_eyre::eyre::eyre!("Sent topic reply id not found"))?;
            let mut messages = self.client.get_messages_by_id(&chat, &[message_id]).await?;
            let message = messages
                .pop()
                .flatten()
                .ok_or_else(|| color_eyre::eyre::eyre!("Sent topic reply not found"))?;
            Ok(convert_message(message, None))
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn delete_message(
        &self,
        chat_id: i64,
        message_id: i32,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            self.client.delete_messages(chat, &[message_id]).await?;
            Ok(())
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn get_messages(
        &self,
        chat_id: i64,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            let outbox_read_max_id = self.cached_outbox_read_max_id(chat_id)?;
            let iter = self.client.iter_messages(chat);
            collect_message_page(
                &self.user_name_cache,
                iter,
                chat_id,
                outbox_read_max_id,
                limit,
                "latest",
            )
            .await
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
            let chat = self.cached_chat(chat_id)?;
            let outbox_read_max_id = self.cached_outbox_read_max_id(chat_id)?;
            let iter = self.client.iter_messages(chat).offset_id(before_message_id);
            collect_message_page(
                &self.user_name_cache,
                iter,
                chat_id,
                outbox_read_max_id,
                limit,
                "older",
            )
            .await
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn mark_chat_read(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            self.client.mark_as_read(chat).await?;
            Ok(())
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn mark_chat_read_through(
        &self,
        chat_id: i64,
        max_message_id: i32,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            match bounded_chat_read_request(chat.pack(), max_message_id)? {
                BoundedChatReadRequest::Channel(request) => {
                    self.client.invoke(&request).await?;
                }
                BoundedChatReadRequest::Messages(request) => {
                    self.client.invoke(&request).await?;
                }
            }
            Ok(())
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn mark_thread_read(
        &self,
        chat_id: i64,
        topic_id: i32,
        max_message_id: i32,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            self.client
                .invoke(&tl::functions::messages::ReadDiscussion {
                    peer: chat.pack().to_input_peer(),
                    msg_id: topic_id,
                    read_max_id: max_message_id,
                })
                .await?;
            Ok(())
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn send_typing_action(
        &self,
        chat_id: i64,
        topic_id: Option<i32>,
    ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
        async move {
            let chat = self.cached_chat(chat_id)?;
            self.client
                .invoke(&tl::functions::messages::SetTyping {
                    peer: chat.pack().to_input_peer(),
                    top_msg_id: topic_id,
                    action: tl::enums::SendMessageAction::SendMessageTypingAction,
                })
                .await?;
            Ok(())
        }
    }

    async fn get_chats(&self, folder_id: Option<i32>, limit: usize) -> Result<Vec<Chat>> {
        Ok(self.load_chats(folder_id, limit).await?.chats)
    }

    async fn get_reconciliation_chats(
        &self,
        folder_id: Option<i32>,
        limit: usize,
    ) -> Result<ReconciliationChatList> {
        self.load_chats(folder_id, limit).await
    }

    #[allow(clippy::manual_async_fn)]
    fn get_folders(&self) -> impl std::future::Future<Output = Result<Vec<Folder>>> + Send + '_ {
        async move {
            let mut iter = self.client.iter_dialogs();
            let mut all_unread_count = 0usize;
            let mut folder_unread_counts = HashMap::new();
            let mut folder_names = HashMap::new();

            while let Some(dialog) = iter.next().await? {
                self.cache_dialog_read_state(&dialog)?;
                let unread_count = dialog_unread_count(&dialog);
                all_unread_count += unread_count;
                if let Some((folder_id, folder_name)) = dialog_folder_metadata(&dialog) {
                    folder_names.entry(folder_id).or_insert(folder_name);
                    *folder_unread_counts.entry(folder_id).or_default() += unread_count;
                } else if let Some(folder_id) = dialog_folder_id(&dialog) {
                    *folder_unread_counts.entry(folder_id).or_default() += unread_count;
                }
            }

            let dialog_filters = match self
                .client
                .invoke(&tl::functions::messages::GetDialogFilters {})
                .await
            {
                Ok(tl::enums::messages::DialogFilters::Filters(dialog_filters)) => {
                    dialog_filters.filters
                }
                Err(_) => Vec::new(),
            };
            self.cache_dialog_filters(&dialog_filters)?;

            Ok(folders_from_dialog_filters(
                dialog_filters,
                all_unread_count,
                folder_unread_counts,
                folder_names,
            ))
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn subscribe_updates(
        &mut self,
    ) -> impl std::future::Future<Output = Result<mpsc::UnboundedReceiver<Update>>> + Send + '_
    {
        async move {
            let (tx, rx) = mpsc::unbounded_channel();
            let client = self.client.clone();
            let user_name_cache = Arc::clone(&self.user_name_cache);
            let outbox_read_max_id_cache = Arc::clone(&self.outbox_read_max_id_cache);

            tokio::spawn(async move {
                'update_loop: loop {
                    match client.next_update().await {
                        Ok(update) => {
                            let updates = match update {
                                grammers_client::Update::NewMessage(msg) => {
                                    let age_seconds =
                                        (Utc::now() - msg.date()).num_seconds().max(0);
                                    diagnostics::event(
                                        "telegram_update_new_message",
                                        format!(
                                            "chat_id={} message_id={} outgoing={} age_secs={age_seconds}",
                                            msg.chat().id(),
                                            msg.id(),
                                            msg.outgoing()
                                        ),
                                    );
                                    cache_sender_name_from_message(&user_name_cache, &msg);
                                    vec![Update::NewMessage(convert_message(msg, None))]
                                }
                                grammers_client::Update::MessageEdited(msg) => {
                                    cache_sender_name_from_message(&user_name_cache, &msg);
                                    vec![Update::EditMessage {
                                        chat_id: msg.chat().id(),
                                        message_id: msg.id(),
                                        new_content: msg.text().to_string(),
                                    }]
                                }
                                grammers_client::Update::MessageDeleted(deletion) => {
                                    delete_message_updates(
                                        deletion.channel_id(),
                                        deletion.messages(),
                                    )
                                }
                                grammers_client::Update::Raw(raw) => {
                                    if let Some(Update::ReadOutgoingMessages {
                                        chat_id,
                                        max_message_id,
                                    }) = read_outbox_update_from_raw(&raw)
                                    {
                                        if let Err(error) = cache_outbox_read_max_id(
                                            &outbox_read_max_id_cache,
                                            chat_id,
                                            max_message_id,
                                        ) {
                                            diagnostics::event(
                                                "read_outbox_cache_error",
                                                format!("chat_id={chat_id} error={error}"),
                                            );
                                        }
                                        diagnostics::event(
                                            "telegram_update_read_outbox",
                                            format!(
                                                "chat_id={chat_id} max_message_id={max_message_id}"
                                            ),
                                        );
                                        vec![Update::ReadOutgoingMessages {
                                            chat_id,
                                            max_message_id,
                                        }]
                                    } else if let Some(update) =
                                        typing_status_update_from_raw_with_user_names(
                                            &raw,
                                            |user_id| cached_user_name(&user_name_cache, user_id),
                                        )
                                    {
                                        vec![update]
                                    } else {
                                        Vec::new()
                                    }
                                }
                                _ => Vec::new(),
                            };

                            for update in updates {
                                if tx.send(update).is_err() {
                                    break 'update_loop;
                                }
                            }
                        }
                        Err(e) => {
                            if tx.send(Update::Error(update_error_message(e))).is_err() {
                                break 'update_loop;
                            }
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
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
    use super::{
        BoundedChatReadRequest, CHAT_CACHE_LOCK_FAILED, CHAT_NOT_FOUND_IN_CACHE_PREFIX,
        DialogPageParts, UPDATE_ERROR_PREFIX, bounded_chat_read_request,
        cached_media_thumbnail_path, canonical_session_identity, chat_cache_lock_failed_message,
        chat_matches_filter_categories, chat_not_found_in_cache_message, complete_private_download,
        create_private_download_dir_with, delete_message_updates, delete_update_chat_id,
        dialog_chats_from_page_parts, dumbgram_init_params, folders_from_dialog_filters,
        input_peers_contain_chat, media_cache_dir, message_sender_name,
        message_status_for_read_state, message_thread_topic_id, publish_downloaded_thumbnail,
        read_outbox_update_from_raw, thread_topic_from_tl, typing_status_update_from_raw,
        typing_status_update_from_raw_with_user_names, update_error_message,
    };
    use crate::telegram::types::{
        MessageStatus, OWN_SENDER_NAME, UNKNOWN_DELETE_UPDATE_CHAT_ID, UNKNOWN_SENDER_NAME, Update,
    };
    use grammers_client::grammers_tl_types as tl;
    use grammers_session::{PackedChat, PackedType};
    use std::{
        collections::HashMap,
        ops::ControlFlow,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn bounded_chat_read_requests_use_exact_nonzero_max_for_messages_and_channels() {
        let user = PackedChat {
            ty: PackedType::User,
            id: 1,
            access_hash: Some(11),
        };
        let channel = PackedChat {
            ty: PackedType::Megagroup,
            id: 2,
            access_hash: Some(22),
        };

        match bounded_chat_read_request(user, 77).expect("user read should be bounded") {
            BoundedChatReadRequest::Messages(request) => assert_eq!(request.max_id, 77),
            BoundedChatReadRequest::Channel(_) => panic!("user read must use messages.readHistory"),
        }
        match bounded_chat_read_request(channel, 88).expect("channel read should be bounded") {
            BoundedChatReadRequest::Channel(request) => assert_eq!(request.max_id, 88),
            BoundedChatReadRequest::Messages(_) => {
                panic!("megagroup read must use channels.readHistory")
            }
        }
        assert!(bounded_chat_read_request(user, 0).is_err());
    }

    #[test]
    fn deleted_exact_forum_topic_maps_to_absence() {
        assert!(thread_topic_from_tl(tl::types::ForumTopicDeleted { id: 99 }.into()).is_none());
    }

    fn private_test_dir(label: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should be after epoch")
            .as_nanos();
        let root = std::path::PathBuf::from(
            std::env::var_os("HOME").expect("test home directory should be available"),
        )
        .join(format!(".dumbgram-{label}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&root).expect("private test directory should be created");
        super::set_private_permissions(&root, 0o700)
            .expect("private test directory permissions should be set");
        root
    }

    fn relative_to_current(path: &std::path::Path) -> std::path::PathBuf {
        let current = std::env::current_dir()
            .expect("test current directory should be available")
            .canonicalize()
            .expect("test current directory should be canonicalized");
        let target = path
            .canonicalize()
            .expect("test target should be canonicalized");
        let current_components = current.components().collect::<Vec<_>>();
        let target_components = target.components().collect::<Vec<_>>();
        let shared = current_components
            .iter()
            .zip(&target_components)
            .take_while(|(left, right)| left == right)
            .count();
        let mut relative = std::path::PathBuf::new();
        for _ in shared..current_components.len() {
            relative.push("..");
        }
        for component in &target_components[shared..] {
            relative.push(component.as_os_str());
        }
        relative
    }

    #[test]
    fn media_cache_uses_canonical_session_identity() {
        let root = private_test_dir("cache-identity");
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        super::set_private_permissions(&first, 0o700).unwrap();
        super::set_private_permissions(&second, 0o700).unwrap();
        let first_session = first.join("session.dat");
        let second_session = second.join("session.dat");
        std::fs::write(&first_session, b"first").unwrap();
        std::fs::write(&second_session, b"second").unwrap();

        let first_cache = media_cache_dir(&first_session).unwrap();
        let second_cache = media_cache_dir(&second_session).unwrap();
        assert_ne!(first_cache, second_cache);
        assert_eq!(
            canonical_session_identity(&relative_to_current(&first_session)).unwrap(),
            first_session.canonicalize().unwrap()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let alias = root.join("session-alias.dat");
            symlink(&first_session, &alias).unwrap();
            assert_eq!(media_cache_dir(&alias).unwrap(), first_cache);
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn media_cache_preserves_non_utf8_identity_and_rejects_unsafe_or_symlink_roots() {
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = private_test_dir("cache-safety");
        let non_utf8 = root.join(std::ffi::OsString::from_vec(vec![b'a', 0xff]));
        std::fs::create_dir(&non_utf8).unwrap();
        super::set_private_permissions(&non_utf8, 0o700).unwrap();
        let session = non_utf8.join(std::ffi::OsString::from_vec(vec![b's', 0xfe]));
        let distinct_session = non_utf8.join(std::ffi::OsString::from_vec(vec![b's', 0xfd]));
        std::fs::write(&session, b"session").unwrap();
        std::fs::write(&distinct_session, b"session").unwrap();
        let cache = media_cache_dir(&session).unwrap();
        assert_eq!(cache.parent(), Some(non_utf8.as_path()));
        assert_ne!(cache, media_cache_dir(&distinct_session).unwrap());

        std::fs::remove_dir(&cache).unwrap();
        let target = root.join("cache-target");
        std::fs::create_dir(&target).unwrap();
        symlink(&target, &cache).unwrap();
        assert!(media_cache_dir(&session).is_err());
        std::fs::remove_file(&cache).unwrap();
        std::fs::write(&cache, b"not a directory").unwrap();
        assert!(media_cache_dir(&session).is_err());
        std::fs::remove_file(&cache).unwrap();

        let unsafe_parent = root.join("unsafe");
        std::fs::create_dir(&unsafe_parent).unwrap();
        std::fs::set_permissions(&unsafe_parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(media_cache_dir(&unsafe_parent.join("session.dat")).is_err());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_thumbnail_publication_retries_and_rejects_symlinks() {
        let root = private_test_dir("cache-publish");
        let session = root.join("session.dat");
        std::fs::write(&session, b"session").unwrap();
        let cache = media_cache_dir(&session).unwrap();
        let first_id = 7;
        let second_id = 8;
        std::fs::create_dir(cache.join(format!(".download-{}-{first_id}", std::process::id())))
            .unwrap();
        let mut ids = [first_id, second_id].into_iter();
        let temp_dir = create_private_download_dir_with(&cache, || ids.next().unwrap()).unwrap();
        assert!(temp_dir.ends_with(format!(".download-{}-{second_id}", std::process::id())));
        let temp_file = temp_dir.join("thumbnail");
        std::fs::write(&temp_file, b"private thumbnail").unwrap();
        let final_path = cache.join("chat-7-message-11-thumb.jpg");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(root.join("unrelated"), &final_path).unwrap();
        }

        publish_downloaded_thumbnail(&temp_file, &final_path).unwrap();
        assert_eq!(std::fs::read(&final_path).unwrap(), b"private thumbnail");
        assert_eq!(
            cached_media_thumbnail_path(&cache, 7, 11),
            Some(final_path.clone())
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
            std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(
                cached_media_thumbnail_path(&cache, 7, 11),
                Some(final_path.clone())
            );
            assert_eq!(std::fs::metadata(&cache).unwrap().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(&temp_dir).unwrap().mode() & 0o777, 0o700);
            assert_eq!(
                std::fs::metadata(&final_path).unwrap().mode() & 0o777,
                0o600
            );
            let symlink_entry = cache.join("chat-7-message-12-thumb.jpg");
            symlink(&final_path, &symlink_entry).unwrap();
            assert_eq!(cached_media_thumbnail_path(&cache, 7, 12), None);
        }

        super::cleanup_private_download(&temp_dir, &temp_file);
        assert!(!temp_dir.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_thumbnail_failures_and_concurrent_publication_leave_no_temporary_artifacts() {
        let root = private_test_dir("cache-cleanup");
        let session = root.join("session.dat");
        std::fs::write(&session, b"session").unwrap();
        let cache = media_cache_dir(&session).unwrap();

        for (id, error_kind) in [
            (20, std::io::ErrorKind::ConnectionAborted),
            (21, std::io::ErrorKind::PermissionDenied),
        ] {
            let temp_dir = create_private_download_dir_with(&cache, || id).unwrap();
            let temp_file = temp_dir.join("thumbnail");
            std::fs::write(&temp_file, b"incomplete").unwrap();
            let final_path = cache.join(format!("failure-{id}"));
            let result = complete_private_download(
                &temp_dir,
                &temp_file,
                &final_path,
                Err(std::io::Error::new(error_kind, "injected failure")),
            );
            assert_eq!(result.unwrap_err().kind(), error_kind);
            assert!(!temp_dir.exists());
            assert!(!temp_file.exists());
        }

        let validation_dir = create_private_download_dir_with(&cache, || 22).unwrap();
        let validation_file = validation_dir.join("thumbnail");
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("missing"), &validation_file).unwrap();
        #[cfg(not(unix))]
        std::fs::create_dir(&validation_file).unwrap();
        assert!(
            complete_private_download(
                &validation_dir,
                &validation_file,
                &cache.join("validation-failure"),
                Ok(())
            )
            .is_err()
        );
        assert!(!validation_dir.exists());

        let rename_dir = create_private_download_dir_with(&cache, || 23).unwrap();
        let rename_file = rename_dir.join("thumbnail");
        std::fs::write(&rename_file, b"rename").unwrap();
        let rename_target = cache.join("rename-failure");
        std::fs::create_dir(&rename_target).unwrap();
        assert!(
            complete_private_download(&rename_dir, &rename_file, &rename_target, Ok(())).is_err()
        );
        assert!(!rename_dir.exists());

        let final_path = cache.join("concurrent");
        let publications = [b"first".as_slice(), b"second".as_slice()]
            .into_iter()
            .enumerate()
            .map(|(offset, content)| {
                let temp_dir =
                    create_private_download_dir_with(&cache, || 30 + offset as u64).unwrap();
                let temp_file = temp_dir.join("thumbnail");
                std::fs::write(&temp_file, content).unwrap();
                (temp_dir, temp_file)
            })
            .collect::<Vec<_>>();
        let threads = publications
            .into_iter()
            .map(|(temp_dir, temp_file)| {
                let final_path = final_path.clone();
                std::thread::spawn(move || {
                    complete_private_download(&temp_dir, &temp_file, &final_path, Ok(()))
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().expect("publication thread should not panic"))
            .collect::<Vec<_>>();
        assert!(results.iter().any(Result::is_ok));
        let metadata = std::fs::symlink_metadata(&final_path).unwrap();
        assert!(metadata.file_type().is_file());
        assert!(!metadata.file_type().is_symlink());
        assert!(std::fs::read_dir(&cache).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".download-")
        }));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn grammers_init_params_do_not_replay_offline_update_backlog() {
        let params = dumbgram_init_params();

        assert!(!params.catch_up);
        assert_eq!(params.update_queue_limit, Some(100));
        assert_eq!(params.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn grammers_init_params_retry_transport_reconnects_with_a_bound() {
        let policy = dumbgram_init_params().reconnection_policy;

        assert!(matches!(policy.should_retry(0), ControlFlow::Continue(_)));
        assert!(matches!(policy.should_retry(5), ControlFlow::Continue(_)));
        assert!(matches!(policy.should_retry(6), ControlFlow::Break(())));
    }

    #[test]
    fn outgoing_message_sender_label_uses_own_name_even_without_sender() {
        assert_eq!(message_sender_name(true, None, None, None), OWN_SENDER_NAME);
        assert_eq!(
            message_sender_name(false, Some("Alice".to_string()), None, None),
            "Alice"
        );
        assert_eq!(
            message_sender_name(
                false,
                None,
                Some("Channel Signature".to_string()),
                Some("News Channel".to_string())
            ),
            "Channel Signature"
        );
        assert_eq!(
            message_sender_name(false, None, None, Some("News Channel".to_string())),
            "News Channel"
        );
        assert_eq!(
            message_sender_name(false, None, None, None),
            UNKNOWN_SENDER_NAME
        );
    }

    #[test]
    fn outgoing_status_uses_dialog_outbox_read_max_id() {
        assert_eq!(
            message_status_for_read_state(true, 10, Some(10)),
            MessageStatus::Read
        );
        assert_eq!(
            message_status_for_read_state(true, 11, Some(10)),
            MessageStatus::Sent
        );
        assert_eq!(
            message_status_for_read_state(false, 10, Some(10)),
            MessageStatus::Delivered
        );
    }

    #[test]
    fn message_thread_topic_id_uses_forum_reply_top_id() {
        let reply_to = tl::enums::MessageReplyHeader::Header(tl::types::MessageReplyHeader {
            reply_to_scheduled: false,
            forum_topic: true,
            quote: false,
            reply_to_msg_id: Some(555),
            reply_to_peer_id: None,
            reply_from: None,
            reply_media: None,
            reply_to_top_id: Some(101),
            quote_text: None,
            quote_entities: None,
            quote_offset: None,
        });

        assert_eq!(message_thread_topic_id(Some(&reply_to), true), Some(101));
    }

    #[test]
    fn ordinary_supergroup_replies_do_not_become_thread_topics() {
        let reply_to = tl::enums::MessageReplyHeader::Header(tl::types::MessageReplyHeader {
            reply_to_scheduled: false,
            forum_topic: false,
            quote: false,
            reply_to_msg_id: Some(555),
            reply_to_peer_id: None,
            reply_from: None,
            reply_media: None,
            reply_to_top_id: Some(101),
            quote_text: None,
            quote_entities: None,
            quote_offset: None,
        });

        assert_eq!(message_thread_topic_id(Some(&reply_to), false), None);
    }

    #[test]
    fn headerless_forum_messages_belong_to_general_topic() {
        assert_eq!(message_thread_topic_id(None, true), Some(1));
    }

    #[test]
    fn raw_topic_typing_updates_keep_topic_id() {
        let update = typing_status_update_from_raw(&tl::enums::Update::ChannelUserTyping(
            tl::types::UpdateChannelUserTyping {
                channel_id: 99,
                top_msg_id: Some(101),
                from_id: tl::enums::Peer::User(tl::types::PeerUser { user_id: 42 }),
                action: tl::enums::SendMessageAction::SendMessageTypingAction,
            },
        ));

        match update {
            Some(Update::TypingStatus {
                chat_id,
                topic_id,
                user_name,
                is_typing,
            }) => {
                assert_eq!(chat_id, 99);
                assert_eq!(topic_id, Some(101));
                assert_eq!(user_name, "User 42");
                assert!(is_typing);
            }
            other => panic!("expected typing update, got {other:?}"),
        }
    }

    #[test]
    fn raw_typing_updates_use_cached_user_name_when_available() {
        let update = typing_status_update_from_raw_with_user_names(
            &tl::enums::Update::ChannelUserTyping(tl::types::UpdateChannelUserTyping {
                channel_id: 99,
                top_msg_id: Some(101),
                from_id: tl::enums::Peer::User(tl::types::PeerUser { user_id: 42 }),
                action: tl::enums::SendMessageAction::SendMessageTypingAction,
            }),
            |user_id| (user_id == 42).then(|| "Alice".to_string()),
        );

        match update {
            Some(Update::TypingStatus { user_name, .. }) => assert_eq!(user_name, "Alice"),
            other => panic!("expected typing update, got {other:?}"),
        }
    }

    #[test]
    fn raw_cancel_typing_updates_clear_topic_user() {
        let update = typing_status_update_from_raw(&tl::enums::Update::ChannelUserTyping(
            tl::types::UpdateChannelUserTyping {
                channel_id: 99,
                top_msg_id: Some(101),
                from_id: tl::enums::Peer::User(tl::types::PeerUser { user_id: 42 }),
                action: tl::enums::SendMessageAction::SendMessageCancelAction,
            },
        ));

        match update {
            Some(Update::TypingStatus {
                chat_id,
                topic_id,
                user_name,
                is_typing,
            }) => {
                assert_eq!(chat_id, 99);
                assert_eq!(topic_id, Some(101));
                assert_eq!(user_name, "User 42");
                assert!(!is_typing);
            }
            other => panic!("expected cancel typing update, got {other:?}"),
        }
    }

    #[test]
    fn raw_outbox_read_updates_become_status_updates() {
        let direct = read_outbox_update_from_raw(&tl::enums::Update::ReadHistoryOutbox(
            tl::types::UpdateReadHistoryOutbox {
                peer: tl::enums::Peer::User(tl::types::PeerUser { user_id: 42 }),
                max_id: 123,
                pts: 1,
                pts_count: 1,
            },
        ));
        assert!(matches!(
            direct,
            Some(Update::ReadOutgoingMessages {
                chat_id: 42,
                max_message_id: 123
            })
        ));

        let channel = read_outbox_update_from_raw(&tl::enums::Update::ReadChannelOutbox(
            tl::types::UpdateReadChannelOutbox {
                channel_id: 99,
                max_id: 77,
            },
        ));
        assert!(matches!(
            channel,
            Some(Update::ReadOutgoingMessages {
                chat_id: 99,
                max_message_id: 77
            })
        ));
    }

    fn dialog_filter(id: i32, title: &str) -> tl::enums::DialogFilter {
        tl::enums::DialogFilter::Filter(tl::types::DialogFilter {
            contacts: false,
            non_contacts: false,
            groups: false,
            broadcasts: false,
            bots: false,
            exclude_muted: false,
            exclude_read: false,
            exclude_archived: false,
            id,
            title: title.to_string(),
            emoticon: None,
            color: None,
            pinned_peers: Vec::new(),
            include_peers: Vec::new(),
            exclude_peers: Vec::new(),
        })
    }

    fn peer_chat(chat_id: i64) -> tl::enums::Peer {
        tl::enums::Peer::Chat(tl::types::PeerChat { chat_id })
    }

    fn peer_notify_settings() -> tl::enums::PeerNotifySettings {
        tl::enums::PeerNotifySettings::Settings(tl::types::PeerNotifySettings {
            show_previews: None,
            silent: None,
            mute_until: None,
            ios_sound: None,
            android_sound: None,
            other_sound: None,
            stories_muted: None,
            stories_hide_sender: None,
            stories_ios_sound: None,
            stories_android_sound: None,
            stories_other_sound: None,
        })
    }

    fn raw_chat(chat_id: i64, title: &str) -> tl::enums::Chat {
        tl::enums::Chat::Chat(tl::types::Chat {
            creator: false,
            left: false,
            deactivated: false,
            call_active: false,
            call_not_empty: false,
            noforwards: false,
            id: chat_id,
            title: title.to_string(),
            photo: tl::enums::ChatPhoto::Empty,
            participants_count: 2,
            date: 0,
            version: 1,
            migrated_to: None,
            admin_rights: None,
            default_banned_rights: None,
        })
    }

    fn raw_dialog(
        chat_id: i64,
        top_message: i32,
        unread_count: i32,
        folder_id: Option<i32>,
    ) -> tl::enums::Dialog {
        tl::enums::Dialog::Dialog(tl::types::Dialog {
            pinned: false,
            unread_mark: false,
            view_forum_as_messages: false,
            peer: peer_chat(chat_id),
            top_message,
            read_inbox_max_id: 0,
            read_outbox_max_id: 0,
            unread_count,
            unread_mentions_count: 0,
            unread_reactions_count: 0,
            notify_settings: peer_notify_settings(),
            pts: None,
            draft: None,
            folder_id,
            ttl_period: None,
        })
    }

    fn raw_message(chat_id: i64, message_id: i32, text: &str) -> tl::enums::Message {
        tl::enums::Message::Message(tl::types::Message {
            out: false,
            mentioned: false,
            media_unread: false,
            silent: false,
            post: false,
            from_scheduled: false,
            legacy: false,
            edit_hide: false,
            pinned: false,
            noforwards: false,
            invert_media: false,
            offline: false,
            id: message_id,
            from_id: None,
            from_boosts_applied: None,
            peer_id: peer_chat(chat_id),
            saved_peer_id: None,
            fwd_from: None,
            via_bot_id: None,
            via_business_bot_id: None,
            reply_to: None,
            date: 0,
            message: text.to_string(),
            media: None,
            reply_markup: None,
            entities: None,
            views: None,
            forwards: None,
            replies: None,
            edit_date: None,
            post_author: None,
            grouped_id: None,
            reactions: None,
            restriction_reason: None,
            ttl_period: None,
            quick_reply_shortcut_id: None,
            effect: None,
            factcheck: None,
        })
    }

    #[test]
    fn dialog_filter_include_peers_match_chats_without_server_folder_ids() {
        let chat = grammers_client::types::Chat::from_raw(raw_chat(42, "Work Room"));
        let mut filter = dialog_filter(2, "Work");
        let tl::enums::DialogFilter::Filter(filter_data) = &mut filter else {
            panic!("expected dialog filter");
        };
        filter_data
            .include_peers
            .push(tl::enums::InputPeer::Chat(tl::types::InputPeerChat {
                chat_id: 42,
            }));

        assert!(input_peers_contain_chat(&filter_data.include_peers, &chat));
    }

    #[test]
    fn dialog_filter_categories_match_group_and_channel_flags() {
        let group = grammers_client::types::Chat::from_raw(raw_chat(42, "Work Room"));
        let mut filter = tl::types::DialogFilter {
            contacts: false,
            non_contacts: false,
            groups: true,
            broadcasts: false,
            bots: false,
            exclude_muted: false,
            exclude_read: false,
            exclude_archived: false,
            id: 2,
            title: "Groups".to_string(),
            emoticon: None,
            color: None,
            pinned_peers: Vec::new(),
            include_peers: Vec::new(),
            exclude_peers: Vec::new(),
        };

        assert!(chat_matches_filter_categories(&filter, &group));
        filter.groups = false;
        assert!(!chat_matches_filter_categories(&filter, &group));
    }

    #[test]
    fn folder_scoped_dialog_chats_use_requested_folder_and_preview() {
        let chats = dialog_chats_from_page_parts(
            DialogPageParts {
                dialogs: vec![raw_dialog(42, 7, 3, None)],
                messages: vec![raw_message(42, 7, "folder top message")],
                chats: vec![raw_chat(42, "Work Room")],
                users: Vec::new(),
            },
            Some(2),
            50,
        );

        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].0.id, 42);
        assert_eq!(chats[0].0.name, "Work Room");
        assert_eq!(chats[0].0.folder_id, Some(2));
        assert_eq!(chats[0].0.unread_count, 3);
        assert_eq!(
            chats[0].0.last_message.as_deref(),
            Some("folder top message")
        );
        assert_eq!(chats[0].1.id(), 42);
    }

    #[test]
    fn folders_from_dialog_filters_uses_filter_names_and_unread_counts() {
        let folders = folders_from_dialog_filters(
            vec![dialog_filter(2, "Personal"), dialog_filter(3, "Work")],
            5,
            HashMap::from([(2, 3), (3, 2)]),
            HashMap::new(),
        );

        assert_eq!(folders.len(), 3);
        assert_eq!(folders[0].name, "All");
        assert_eq!(folders[0].unread_count, 5);
        assert_eq!(folders[1].id, 2);
        assert_eq!(folders[1].name, "Personal");
        assert_eq!(folders[1].unread_count, 3);
        assert_eq!(folders[2].id, 3);
        assert_eq!(folders[2].name, "Work");
        assert_eq!(folders[2].unread_count, 2);
    }

    #[test]
    fn folders_from_dialog_filters_keeps_unknown_folder_ids_accessible() {
        let folders = folders_from_dialog_filters(
            vec![tl::enums::DialogFilter::Default],
            4,
            HashMap::from([(9, 4)]),
            HashMap::new(),
        );

        assert_eq!(folders.len(), 2);
        assert_eq!(folders[1].id, 9);
        assert_eq!(folders[1].name, "Folder 9");
        assert_eq!(folders[1].unread_count, 4);
    }

    #[test]
    fn folders_from_dialog_filters_uses_dialog_folder_fallback_names_without_filters() {
        let folders = folders_from_dialog_filters(
            Vec::new(),
            4,
            HashMap::from([(1, 4)]),
            HashMap::from([(1, "Archived".to_string())]),
        );

        assert_eq!(folders.len(), 2);
        assert_eq!(folders[1].id, 1);
        assert_eq!(folders[1].name, "Archived");
        assert_eq!(folders[1].unread_count, 4);
    }

    #[test]
    fn chat_cache_miss_message_includes_chat_id() {
        assert_eq!(
            chat_not_found_in_cache_message(42),
            format!("{CHAT_NOT_FOUND_IN_CACHE_PREFIX}: 42")
        );
    }

    #[test]
    fn chat_cache_lock_error_uses_shared_prefix() {
        assert_eq!(
            chat_cache_lock_failed_message("poisoned"),
            format!("{CHAT_CACHE_LOCK_FAILED}: poisoned")
        );
    }

    #[test]
    fn update_error_uses_shared_prefix() {
        assert_eq!(
            update_error_message("disconnected"),
            format!("{UPDATE_ERROR_PREFIX}: disconnected")
        );
    }

    #[test]
    fn delete_update_uses_channel_id_when_available() {
        assert_eq!(delete_update_chat_id(Some(123)), 123);
    }

    #[test]
    fn delete_update_keeps_unknown_chat_wildcard_without_channel_id() {
        assert_eq!(delete_update_chat_id(None), UNKNOWN_DELETE_UPDATE_CHAT_ID);
    }

    #[test]
    fn delete_message_updates_include_every_message_id() {
        let updates = delete_message_updates(Some(123), &[7, 8]);

        assert_eq!(updates.len(), 2);
        assert_delete_update(&updates[0], 123, 7);
        assert_delete_update(&updates[1], 123, 8);
    }

    fn assert_delete_update(update: &Update, expected_chat_id: i64, expected_message_id: i32) {
        match update {
            Update::DeleteMessage {
                chat_id,
                message_id,
            } => {
                assert_eq!(*chat_id, expected_chat_id);
                assert_eq!(*message_id, expected_message_id);
            }
            _ => panic!("expected delete update, got {update:?}"),
        }
    }
}
