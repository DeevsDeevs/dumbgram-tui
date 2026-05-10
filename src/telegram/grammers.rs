use chrono::{DateTime, Utc};
use color_eyre::Result;
#[cfg(test)]
use grammers_client::types::ChatMap;
use grammers_client::{
    Client, Config, InitParams, InputMessage, grammers_tl_types as tl,
    types::{Downloadable, photo_sizes::PhotoSize},
};
use grammers_session::Session;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc;

use super::client::TelegramClient;
use super::types::{
    Chat, Folder, Message, MessageMedia, MessageMediaKind, MessageStatus, OWN_SENDER_NAME,
    UNKNOWN_DELETE_UPDATE_CHAT_ID, UNKNOWN_SENDER_NAME, Update, all_folder,
    message_display_preview,
};
use crate::diagnostics;

const CHAT_NOT_FOUND_IN_CACHE_PREFIX: &str = "Chat not found in cache";
const CHAT_CACHE_LOCK_FAILED: &str = "Chat cache lock failed";
const UPDATE_ERROR_PREFIX: &str = "Update error";

type ChatCache = HashMap<i64, grammers_client::types::Chat>;
type DialogFilterCache = HashMap<i32, tl::enums::DialogFilter>;

#[derive(Clone)]
pub struct GrammersClient {
    client: Client,
    chat_cache: Arc<Mutex<ChatCache>>,
    dialog_filter_cache: Arc<Mutex<DialogFilterCache>>,
    session_path: String,
    media_cache_dir: PathBuf,
}

impl GrammersClient {
    pub async fn new(api_id: i32, api_hash: String, session_path: &str) -> Result<Self> {
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
            dialog_filter_cache: Arc::new(Mutex::new(HashMap::new())),
            session_path: session_path.to_string(),
            media_cache_dir: media_cache_dir(session_path),
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
}

fn dumbgram_init_params() -> InitParams {
    InitParams {
        device_model: "Dumbgram TUI".to_string(),
        system_version: env!("CARGO_PKG_VERSION").to_string(),
        app_version: "1.0.0".to_string(),
        // Startup and folder/chat loads already fetch the recent visible history. Replaying
        // offline update backlog makes hours-old messages appear as new live messages.
        catch_up: false,
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

fn message_sender_name(is_outgoing: bool, sender_name: Option<String>) -> String {
    if is_outgoing {
        OWN_SENDER_NAME.to_string()
    } else {
        sender_name.unwrap_or_else(|| UNKNOWN_SENDER_NAME.to_string())
    }
}

fn convert_message(msg: grammers_client::types::Message) -> Message {
    let is_outgoing = msg.outgoing();
    let media = message_media(msg.media().as_ref());
    Message {
        id: msg.id(),
        chat_id: msg.chat().id(),
        sender_name: message_sender_name(is_outgoing, msg.sender().map(|s| s.name().to_string())),
        content: msg.text().to_string(),
        timestamp: msg.date(),
        is_own: is_outgoing,
        is_edited: msg.edit_date().is_some(),
        reply_to_content: None,
        media,
        status: if is_outgoing {
            MessageStatus::Sent
        } else {
            MessageStatus::Delivered
        },
        can_edit: is_outgoing && is_within_edit_window(msg.date()),
        can_delete: is_outgoing,
        error: None,
    }
}

fn media_cache_dir(session_path: &str) -> PathBuf {
    let session_stem = Path::new(session_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("session");
    std::env::temp_dir()
        .join("dumbgram-tui-media")
        .join(session_stem)
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

async fn message_media_with_local_preview(
    client: &Client,
    cache_dir: &Path,
    chat_id: i64,
    message_id: i32,
    media: Option<&grammers_client::types::Media>,
) -> Option<MessageMedia> {
    let mut message_media = message_media(media);
    if let (Some(message_media), Some(media)) = (message_media.as_mut(), media)
        && matches!(
            message_media.kind,
            MessageMediaKind::Photo | MessageMediaKind::Image
        )
    {
        match download_media_thumbnail(client, cache_dir, chat_id, message_id, media).await {
            Ok(Some(path)) => *message_media = message_media.clone().with_local_path(path),
            Ok(None) => {}
            Err(error) => diagnostics::event(
                "media_preview_download_error",
                format!("chat_id={chat_id} message_id={message_id} error={error}"),
            ),
        }
    }
    message_media
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

    std::fs::create_dir_all(cache_dir)?;
    let path = cache_dir.join(format!("chat-{chat_id}-message-{message_id}-thumb.jpg"));
    if path.exists() {
        return Ok(Some(path));
    }

    client
        .download_media(&Downloadable::PhotoSize(thumbnail), &path)
        .await?;
    Ok(Some(path))
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

async fn convert_message_with_media_preview(
    client: &Client,
    media_cache_dir: &Path,
    msg: grammers_client::types::Message,
) -> Message {
    let is_outgoing = msg.outgoing();
    let media = msg.media();
    let message_media = message_media_with_local_preview(
        client,
        media_cache_dir,
        msg.chat().id(),
        msg.id(),
        media.as_ref(),
    )
    .await;

    Message {
        id: msg.id(),
        chat_id: msg.chat().id(),
        sender_name: message_sender_name(is_outgoing, msg.sender().map(|s| s.name().to_string())),
        content: msg.text().to_string(),
        timestamp: msg.date(),
        is_own: is_outgoing,
        is_edited: msg.edit_date().is_some(),
        reply_to_content: None,
        media: message_media,
        status: if is_outgoing {
            MessageStatus::Sent
        } else {
            MessageStatus::Delivered
        },
        can_edit: is_outgoing && is_within_edit_window(msg.date()),
        can_delete: is_outgoing,
        error: None,
    }
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
    client: &Client,
    media_cache_dir: &Path,
    mut iter: grammers_client::client::messages::MessageIter,
    chat_id: i64,
    limit: usize,
    direction: &str,
) -> Result<Vec<Message>> {
    let mut messages = Vec::new();

    while let Some(msg) = iter.next().await? {
        messages.push(convert_message_with_media_preview(client, media_cache_dir, msg).await);
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

impl TelegramClient for GrammersClient {
    async fn connect(&mut self) -> Result<()> {
        Ok(())
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
            Ok(convert_message(msg))
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
            Ok(convert_message(msg))
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
            let iter = self.client.iter_messages(chat);
            collect_message_page(
                &self.client,
                &self.media_cache_dir,
                iter,
                chat_id,
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
            let iter = self.client.iter_messages(chat).offset_id(before_message_id);
            collect_message_page(
                &self.client,
                &self.media_cache_dir,
                iter,
                chat_id,
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

    async fn get_chats(&self, folder_id: Option<i32>, limit: usize) -> Result<Vec<Chat>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let dialog_filter = if let Some(folder_id) = folder_id {
            self.cached_dialog_filter(folder_id)?
        } else {
            None
        };
        let mut iter = self.client.iter_dialogs();
        let mut chats = Vec::new();

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

        Ok(chats)
    }

    #[allow(clippy::manual_async_fn)]
    fn get_folders(&self) -> impl std::future::Future<Output = Result<Vec<Folder>>> + Send + '_ {
        async move {
            let mut iter = self.client.iter_dialogs();
            let mut all_unread_count = 0usize;
            let mut folder_unread_counts = HashMap::new();
            let mut folder_names = HashMap::new();

            while let Some(dialog) = iter.next().await? {
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
                                    if !msg.outgoing() {
                                        vec![Update::NewMessage(convert_message(msg))]
                                    } else {
                                        Vec::new()
                                    }
                                }
                                grammers_client::Update::MessageEdited(msg) => {
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
        CHAT_CACHE_LOCK_FAILED, CHAT_NOT_FOUND_IN_CACHE_PREFIX, DialogPageParts,
        UPDATE_ERROR_PREFIX, chat_cache_lock_failed_message, chat_matches_filter_categories,
        chat_not_found_in_cache_message, delete_message_updates, delete_update_chat_id,
        dialog_chats_from_page_parts, dumbgram_init_params, folders_from_dialog_filters,
        input_peers_contain_chat, message_sender_name, update_error_message,
    };
    use crate::telegram::types::{
        OWN_SENDER_NAME, UNKNOWN_DELETE_UPDATE_CHAT_ID, UNKNOWN_SENDER_NAME, Update,
    };
    use grammers_client::grammers_tl_types as tl;
    use std::collections::HashMap;

    #[test]
    fn grammers_init_params_do_not_replay_offline_update_backlog() {
        let params = dumbgram_init_params();

        assert!(!params.catch_up);
        assert_eq!(params.update_queue_limit, Some(100));
    }

    #[test]
    fn outgoing_message_sender_label_uses_own_name_even_without_sender() {
        assert_eq!(message_sender_name(true, None), OWN_SENDER_NAME);
        assert_eq!(
            message_sender_name(false, Some("Alice".to_string())),
            "Alice"
        );
        assert_eq!(message_sender_name(false, None), UNKNOWN_SENDER_NAME);
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
