use crate::diagnostics;
use crate::state::{
    AppState, DeleteConfirmation, MessageSubmitAction, NO_CHAT_SELECTED_ERROR,
    ReconciliationContext, ReconciliationSnapshot,
};
use crate::telegram::{
    DownloadedMedia, TelegramClient,
    types::{ALL_FOLDER_ID, Chat, Folder, Message, MessageMediaKind, ThreadTopic},
};
use color_eyre::Result;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

static TEMP_ID_COUNTER: AtomicI32 = AtomicI32::new(-1);
pub(crate) const CHAT_LIST_PAGE_SIZE: usize = 50;
const MESSAGE_HISTORY_PAGE_SIZE: usize = 20;
const CHAT_THREAD_TOPIC_PAGE_SIZE: usize = 50;
#[cfg(not(test))]
const MESSAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const MESSAGE_LOAD_TIMEOUT: Duration = Duration::from_millis(10);
#[cfg(not(test))]
const CHAT_LIST_LOAD_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const CHAT_LIST_LOAD_TIMEOUT: Duration = Duration::from_millis(10);
#[cfg(not(test))]
const MARK_CHAT_READ_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const MARK_CHAT_READ_TIMEOUT: Duration = Duration::from_millis(10);
#[cfg(not(test))]
const SEND_TYPING_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const SEND_TYPING_TIMEOUT: Duration = Duration::from_millis(10);
#[cfg(not(test))]
pub(crate) const RECONCILIATION_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
pub(crate) const RECONCILIATION_TIMEOUT: Duration = Duration::from_millis(25);
pub(crate) const NO_OLDER_MESSAGES_STATUS: &str = "No older messages";
pub(crate) const LOAD_MESSAGES_TIMED_OUT_STATUS: &str = "Load messages timed out";
pub(crate) const LOAD_OLDER_MESSAGES_TIMED_OUT_STATUS: &str = "Load older messages timed out";
pub(crate) const LOAD_CHATS_TIMED_OUT_STATUS: &str = "Load chats timed out";
pub(crate) const LOAD_OLDER_MESSAGES_FAILED_PREFIX: &str = "Load older messages failed";
pub(crate) const DOWNLOAD_MEDIA_FAILED_PREFIX: &str = "Download media failed";

fn load_older_messages_failed_error(error: impl std::fmt::Display) -> String {
    format!("{LOAD_OLDER_MESSAGES_FAILED_PREFIX}: {error}")
}

fn download_media_failed_error(error: impl std::fmt::Display) -> String {
    format!("{DOWNLOAD_MEDIA_FAILED_PREFIX}: {error}")
}

pub fn default_download_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("Downloads"))
}

pub async fn download_message_media_result<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    message_id: i32,
    media_kind: MessageMediaKind,
    destination_dir: PathBuf,
) -> std::result::Result<DownloadedMedia, String> {
    diagnostics::event(
        "media_download_start",
        format!(
            "chat_id={chat_id} message_id={message_id} kind={} destination=downloads",
            media_kind.diagnostic_label()
        ),
    );
    let started = Instant::now();
    match client
        .download_message_media(chat_id, message_id, destination_dir)
        .await
    {
        Ok(downloaded) => {
            diagnostics::event(
                "media_download_finish",
                format!(
                    "chat_id={chat_id} message_id={message_id} kind={} bytes={} elapsed_ms={} destination=downloads",
                    media_kind.diagnostic_label(),
                    downloaded.bytes,
                    started.elapsed().as_millis()
                ),
            );
            Ok(downloaded)
        }
        Err(error) => {
            diagnostics::event(
                "media_download_error",
                format!(
                    "chat_id={chat_id} message_id={message_id} kind={} elapsed_ms={} error=true",
                    media_kind.diagnostic_label(),
                    started.elapsed().as_millis()
                ),
            );
            Err(download_media_failed_error(error))
        }
    }
}

pub struct PendingSend {
    pub(crate) temp_id: i32,
    pub(crate) chat_id: i64,
    pub(crate) thread_top_message_id: Option<i32>,
    pub(crate) content: String,
}

pub fn begin_confirm_delete(state: &mut AppState) -> Option<DeleteConfirmation> {
    let confirmation = state.delete_confirmation()?;
    state.cancel_delete_confirmation();
    Some(confirmation)
}

pub async fn delete_message_result<C: TelegramClient>(
    client: &C,
    confirmation: DeleteConfirmation,
) -> std::result::Result<(), String> {
    match client
        .delete_message(confirmation.chat_id, confirmation.message_id)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn apply_delete_message_result(
    state: &mut AppState,
    confirmation: DeleteConfirmation,
    result: std::result::Result<(), String>,
) {
    match result {
        Ok(_) => state.apply_delete_success(confirmation),
        Err(error) => state.apply_delete_failure(error),
    }
}

pub async fn confirm_delete<C: TelegramClient>(state: &mut AppState, client: &mut C) -> Result<()> {
    let Some(confirmation) = begin_confirm_delete(state) else {
        return Ok(());
    };

    let result = delete_message_result(client, confirmation).await;
    apply_delete_message_result(state, confirmation, result);

    Ok(())
}

pub async fn mark_chat_read_best_effort<C: TelegramClient>(client: &C, chat_id: i64) {
    diagnostics::event("mark_chat_read_start", format!("chat_id={chat_id}"));
    let started = Instant::now();
    match tokio::time::timeout(MARK_CHAT_READ_TIMEOUT, client.mark_chat_read(chat_id)).await {
        Ok(Ok(())) => diagnostics::event(
            "mark_chat_read_finish",
            format!(
                "chat_id={chat_id} elapsed_ms={}",
                started.elapsed().as_millis()
            ),
        ),
        Ok(Err(error)) => diagnostics::event(
            "mark_chat_read_error",
            format!(
                "chat_id={chat_id} elapsed_ms={} error={error}",
                started.elapsed().as_millis()
            ),
        ),
        Err(_) => diagnostics::event(
            "mark_chat_read_timeout",
            format!(
                "chat_id={chat_id} elapsed_ms={} timeout_ms={}",
                started.elapsed().as_millis(),
                MARK_CHAT_READ_TIMEOUT.as_millis()
            ),
        ),
    }
}

pub async fn mark_thread_read_best_effort<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    topic_id: i32,
    max_message_id: i32,
) {
    diagnostics::event(
        "mark_thread_read_start",
        format!("chat_id={chat_id} topic_id={topic_id} max_message_id={max_message_id}"),
    );
    let started = Instant::now();
    match tokio::time::timeout(
        MARK_CHAT_READ_TIMEOUT,
        client.mark_thread_read(chat_id, topic_id, max_message_id),
    )
    .await
    {
        Ok(Ok(())) => diagnostics::event(
            "mark_thread_read_finish",
            format!(
                "chat_id={chat_id} topic_id={topic_id} max_message_id={max_message_id} elapsed_ms={}",
                started.elapsed().as_millis()
            ),
        ),
        Ok(Err(error)) => diagnostics::event(
            "mark_thread_read_error",
            format!(
                "chat_id={chat_id} topic_id={topic_id} max_message_id={max_message_id} elapsed_ms={} error={error}",
                started.elapsed().as_millis()
            ),
        ),
        Err(_) => diagnostics::event(
            "mark_thread_read_timeout",
            format!(
                "chat_id={chat_id} topic_id={topic_id} max_message_id={max_message_id} elapsed_ms={} timeout_ms={}",
                started.elapsed().as_millis(),
                MARK_CHAT_READ_TIMEOUT.as_millis()
            ),
        ),
    }
}

pub async fn send_typing_action_best_effort<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    topic_id: Option<i32>,
) {
    diagnostics::event(
        "typing_action_start",
        format!("chat_id={chat_id} topic_id={topic_id:?}"),
    );
    let started = Instant::now();
    match tokio::time::timeout(
        SEND_TYPING_TIMEOUT,
        client.send_typing_action(chat_id, topic_id),
    )
    .await
    {
        Ok(Ok(())) => diagnostics::event(
            "typing_action_finish",
            format!(
                "chat_id={chat_id} topic_id={topic_id:?} elapsed_ms={}",
                started.elapsed().as_millis()
            ),
        ),
        Ok(Err(error)) => diagnostics::event(
            "typing_action_error",
            format!(
                "chat_id={chat_id} topic_id={topic_id:?} elapsed_ms={} error={error}",
                started.elapsed().as_millis()
            ),
        ),
        Err(_) => diagnostics::event(
            "typing_action_timeout",
            format!(
                "chat_id={chat_id} topic_id={topic_id:?} elapsed_ms={} timeout_ms={}",
                started.elapsed().as_millis(),
                SEND_TYPING_TIMEOUT.as_millis()
            ),
        ),
    }
}

pub async fn fetch_latest_chat_messages<C: TelegramClient>(
    client: &C,
    chat_id: i64,
) -> std::result::Result<Vec<Message>, String> {
    diagnostics::event(
        "messages_load_start",
        format!("chat_id={chat_id} limit={MESSAGE_HISTORY_PAGE_SIZE}"),
    );
    let started = Instant::now();
    let result = tokio::time::timeout(
        MESSAGE_LOAD_TIMEOUT,
        client.get_messages(chat_id, MESSAGE_HISTORY_PAGE_SIZE),
    )
    .await;
    let result = match result {
        Ok(result) => result,
        Err(_) => {
            diagnostics::event(
                "messages_load_timeout",
                format!(
                    "chat_id={chat_id} elapsed_ms={} timeout_ms={}",
                    started.elapsed().as_millis(),
                    MESSAGE_LOAD_TIMEOUT.as_millis()
                ),
            );
            return Err(LOAD_MESSAGES_TIMED_OUT_STATUS.to_string());
        }
    };
    match result {
        Ok(messages) => {
            diagnostics::event(
                "messages_load_finish",
                format!(
                    "chat_id={chat_id} count={} max_chars={} elapsed_ms={}",
                    messages.len(),
                    messages
                        .iter()
                        .map(|message| message.content.chars().count())
                        .max()
                        .unwrap_or(0),
                    started.elapsed().as_millis()
                ),
            );
            Ok(messages)
        }
        Err(error) => {
            diagnostics::event(
                "messages_load_error",
                format!(
                    "chat_id={chat_id} elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            );
            Err(error.to_string())
        }
    }
}

pub fn begin_open_chat_at(state: &mut AppState, chat_index: usize) -> Option<i64> {
    if chat_index >= state.chats.len() {
        return None;
    }

    let old_index = state.selected_chat_index;
    if old_index != chat_index {
        state.leave_selected_chat();
    }
    state.select_chat(chat_index);
    if old_index != state.selected_chat_index {
        state.clear_loaded_chat_messages();
        state.begin_conversation_load();
        return state.selected_chat_id();
    }

    None
}

pub async fn fetch_chat_thread_topics<C: TelegramClient>(
    client: &C,
    chat_id: i64,
) -> std::result::Result<Vec<ThreadTopic>, String> {
    diagnostics::event(
        "thread_topics_load_start",
        format!("chat_id={chat_id} limit={CHAT_THREAD_TOPIC_PAGE_SIZE}"),
    );
    let started = Instant::now();
    match client
        .get_thread_topics(chat_id, CHAT_THREAD_TOPIC_PAGE_SIZE)
        .await
    {
        Ok(thread_topics) => {
            diagnostics::event(
                "thread_topics_load_finish",
                format!(
                    "chat_id={chat_id} count={} elapsed_ms={}",
                    thread_topics.len(),
                    started.elapsed().as_millis()
                ),
            );
            Ok(thread_topics)
        }
        Err(error) => {
            diagnostics::event(
                "thread_topics_load_error",
                format!(
                    "chat_id={chat_id} elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            );
            Err(error.to_string())
        }
    }
}

pub async fn fetch_thread_topic_messages<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    topic_id: i32,
) -> std::result::Result<Vec<Message>, String> {
    diagnostics::event(
        "thread_messages_load_start",
        format!("chat_id={chat_id} topic_id={topic_id} limit={MESSAGE_HISTORY_PAGE_SIZE}"),
    );
    let started = Instant::now();
    match tokio::time::timeout(
        MESSAGE_LOAD_TIMEOUT,
        client.get_thread_messages(chat_id, topic_id, MESSAGE_HISTORY_PAGE_SIZE),
    )
    .await
    {
        Err(_) => {
            diagnostics::event(
                "thread_messages_load_timeout",
                format!(
                    "chat_id={chat_id} topic_id={topic_id} elapsed_ms={} timeout_ms={}",
                    started.elapsed().as_millis(),
                    MESSAGE_LOAD_TIMEOUT.as_millis()
                ),
            );
            Err(LOAD_MESSAGES_TIMED_OUT_STATUS.to_string())
        }
        Ok(Ok(messages)) => {
            diagnostics::event(
                "thread_messages_load_finish",
                format!(
                    "chat_id={chat_id} topic_id={topic_id} count={} elapsed_ms={}",
                    messages.len(),
                    started.elapsed().as_millis()
                ),
            );
            Ok(messages)
        }
        Ok(Err(error)) => {
            diagnostics::event(
                "thread_messages_load_error",
                format!(
                    "chat_id={chat_id} topic_id={topic_id} elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            );
            Err(error.to_string())
        }
    }
}

pub async fn load_selected_thread_topic_messages<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    let Some(chat_id) = state.selected_chat_id() else {
        return Ok(());
    };
    let Some(topic_id) = state.selected_thread_topic().map(|topic| topic.id) else {
        return Ok(());
    };

    state.begin_conversation_load();
    match fetch_thread_topic_messages(client, chat_id, topic_id).await {
        Ok(messages) => {
            let max_message_id = messages.iter().map(|message| message.id).max();
            state.apply_loaded_selected_chat_messages(messages);
            if let Some(max_message_id) = max_message_id {
                mark_thread_read_best_effort(client, chat_id, topic_id, max_message_id).await;
            }
        }
        Err(error) => {
            state.mark_conversation_load_failed();
            state.set_error(error);
        }
    }

    Ok(())
}

pub async fn load_selected_chat_messages<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    if let Some(chat_id) = state.selected_chat_id() {
        state.clear_loaded_chat_messages();
        state.begin_conversation_load();
        match fetch_latest_chat_messages(client, chat_id).await {
            Ok(messages) => {
                state.apply_loaded_selected_chat_messages(messages);
                if let Ok(thread_topics) = fetch_chat_thread_topics(client, chat_id).await {
                    state.apply_loaded_selected_chat_thread_topics(thread_topics);
                }
                mark_chat_read_best_effort(client, chat_id).await;
            }
            Err(error) => {
                state.mark_conversation_load_failed();
                state.set_error(error);
            }
        }
    } else {
        state.clear_loaded_chat_messages();
    }

    Ok(())
}

async fn fetch_older_messages_with<F>(
    client_call: F,
    chat_id: i64,
    topic_id: Option<i32>,
    before_message_id: i32,
) -> std::result::Result<Vec<Message>, String>
where
    F: std::future::Future<Output = color_eyre::Result<Vec<Message>>>,
{
    let scope = topic_id
        .map(|topic_id| format!("chat_id={chat_id} topic_id={topic_id}"))
        .unwrap_or_else(|| format!("chat_id={chat_id}"));
    diagnostics::event(
        "older_messages_load_start",
        format!("{scope} before_message_id={before_message_id} limit={MESSAGE_HISTORY_PAGE_SIZE}"),
    );
    let started = Instant::now();
    match tokio::time::timeout(MESSAGE_LOAD_TIMEOUT, client_call).await {
        Err(_) => {
            diagnostics::event(
                "older_messages_load_timeout",
                format!(
                    "{scope} elapsed_ms={} timeout_ms={}",
                    started.elapsed().as_millis(),
                    MESSAGE_LOAD_TIMEOUT.as_millis()
                ),
            );
            Err(LOAD_OLDER_MESSAGES_TIMED_OUT_STATUS.to_string())
        }
        Ok(Ok(messages)) => {
            diagnostics::event(
                "older_messages_load_finish",
                format!(
                    "{scope} count={} max_chars={} elapsed_ms={}",
                    messages.len(),
                    messages
                        .iter()
                        .map(|message| message.content.chars().count())
                        .max()
                        .unwrap_or(0),
                    started.elapsed().as_millis()
                ),
            );
            Ok(messages)
        }
        Ok(Err(error)) => {
            diagnostics::event(
                "older_messages_load_error",
                format!(
                    "{scope} elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            );
            Err(load_older_messages_failed_error(error))
        }
    }
}

pub async fn fetch_older_chat_messages<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    before_message_id: i32,
) -> std::result::Result<Vec<Message>, String> {
    fetch_older_messages_with(
        client.get_messages_before(chat_id, before_message_id, MESSAGE_HISTORY_PAGE_SIZE),
        chat_id,
        None,
        before_message_id,
    )
    .await
}

pub async fn fetch_older_thread_topic_messages<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    topic_id: i32,
    before_message_id: i32,
) -> std::result::Result<Vec<Message>, String> {
    fetch_older_messages_with(
        client.get_thread_messages_before(
            chat_id,
            topic_id,
            before_message_id,
            MESSAGE_HISTORY_PAGE_SIZE,
        ),
        chat_id,
        Some(topic_id),
        before_message_id,
    )
    .await
}

pub fn selected_older_messages_request(state: &mut AppState) -> Option<(i64, Option<i32>, i32)> {
    let Some(chat_id) = state.selected_chat_id() else {
        state.set_error(NO_CHAT_SELECTED_ERROR.to_string());
        return None;
    };
    let topic_id = state.selected_thread_topic().map(|topic| topic.id);
    let before_message_id = state.messages.first().map(|message| message.id)?;
    if state.selected_chat_older_history_exhausted() {
        state.set_status(NO_OLDER_MESSAGES_STATUS);
        return None;
    }

    Some((chat_id, topic_id, before_message_id))
}

pub fn apply_older_chat_messages_result(
    state: &mut AppState,
    result: std::result::Result<Vec<Message>, String>,
) -> usize {
    match result {
        Ok(messages) => {
            let added = state.prepend_loaded_selected_chat_messages(messages);
            if added == 0 {
                state.mark_selected_chat_older_history_exhausted();
                state.set_status(NO_OLDER_MESSAGES_STATUS);
            }
            added
        }
        Err(error) => {
            state.set_error(error);
            0
        }
    }
}

pub async fn load_older_selected_chat_messages<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<usize> {
    let Some((chat_id, topic_id, before_message_id)) = selected_older_messages_request(state)
    else {
        return Ok(0);
    };
    let result = if let Some(topic_id) = topic_id {
        fetch_older_thread_topic_messages(client, chat_id, topic_id, before_message_id).await
    } else {
        fetch_older_chat_messages(client, chat_id, before_message_id).await
    };
    Ok(apply_older_chat_messages_result(state, result))
}

pub struct InitialStateLoad {
    pub folders: Vec<Folder>,
    pub chats: Vec<Chat>,
    pub messages: std::result::Result<Vec<Message>, String>,
    pub thread_topics: Vec<ThreadTopic>,
}

fn folder_filter_id(folder: &Folder) -> Option<i32> {
    (folder.id != ALL_FOLDER_ID).then_some(folder.id)
}

pub async fn fetch_initial_state<C: TelegramClient>(
    client: &C,
) -> std::result::Result<InitialStateLoad, String> {
    diagnostics::event(
        "initial_load_start",
        "folders=true chats=true messages=true",
    );
    let started = Instant::now();
    let folders = match client.get_folders().await {
        Ok(folders) => folders,
        Err(error) => {
            diagnostics::event(
                "initial_load_error",
                format!(
                    "stage=folders elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            );
            return Err(error.to_string());
        }
    };

    let (chats, messages, thread_topics) = if let Some(folder) = folders.first() {
        let folder_id = folder_filter_id(folder);
        match fetch_folder_chats_and_selected_messages(client, folder_id).await {
            Ok(load) => (load.chats, load.messages, load.thread_topics),
            Err(error) => {
                diagnostics::event(
                    "initial_load_error",
                    format!(
                        "stage=chats elapsed_ms={} error={error}",
                        started.elapsed().as_millis()
                    ),
                );
                return Err(error);
            }
        }
    } else {
        (Vec::new(), Ok(Vec::new()), Vec::new())
    };

    diagnostics::event(
        "initial_load_finish",
        format!(
            "folders={} chats={} messages={} thread_topics={} elapsed_ms={}",
            folders.len(),
            chats.len(),
            messages.as_ref().map_or(0, Vec::len),
            thread_topics.len(),
            started.elapsed().as_millis()
        ),
    );

    Ok(InitialStateLoad {
        folders,
        chats,
        messages,
        thread_topics,
    })
}

pub async fn fetch_reconciliation_snapshot<C: TelegramClient + Sync>(
    client: &C,
    context: ReconciliationContext,
) -> std::result::Result<ReconciliationSnapshot, String> {
    diagnostics::event(
        "reconciliation_start",
        format!(
            "folder_id={:?} chat_id={:?} topic_id={:?}",
            context.folder_id, context.chat_id, context.topic_id
        ),
    );
    let started = Instant::now();
    let fetch = async {
        let folders = client
            .get_folders()
            .await
            .map_err(|error| error.to_string())?;
        let selected_folder = context
            .folder_id
            .and_then(|folder_id| folders.iter().find(|folder| folder.id == folder_id))
            .or_else(|| folders.first());
        let selected_folder_id = selected_folder.map(|folder| folder.id);
        let folder_filter_id =
            selected_folder.and_then(|folder| (folder.id != ALL_FOLDER_ID).then_some(folder.id));
        let chat_list = client
            .get_reconciliation_chats(folder_filter_id, CHAT_LIST_PAGE_SIZE)
            .await
            .map_err(|error| error.to_string())?;
        let chats = chat_list.chats;
        let selected_chat = context
            .chat_id
            .and_then(|chat_id| chats.iter().find(|chat| chat.id == chat_id))
            .or_else(|| chats.first());
        let selected_chat_id = selected_chat.map(|chat| chat.id);

        let (thread_topics, selected_topic_id, messages) = match selected_chat_id {
            Some(chat_id) => {
                let thread_topics = fetch_chat_thread_topics(client, chat_id).await?;
                let selected_topic_id = context
                    .topic_id
                    .filter(|topic_id| thread_topics.iter().any(|topic| topic.id == *topic_id));
                let messages = if let Some(topic_id) = selected_topic_id {
                    fetch_thread_topic_messages(client, chat_id, topic_id).await?
                } else {
                    fetch_latest_chat_messages(client, chat_id).await?
                };
                (thread_topics, selected_topic_id, messages)
            }
            None => (Vec::new(), None, Vec::new()),
        };

        Ok(ReconciliationSnapshot {
            folders,
            selected_folder_id,
            chats,
            chat_last_message_ids: chat_list.last_message_ids,
            selected_chat_id,
            thread_topics,
            selected_topic_id,
            messages,
        })
    };

    match tokio::time::timeout(RECONCILIATION_TIMEOUT, fetch).await {
        Ok(result) => {
            diagnostics::event(
                "reconciliation_finish",
                format!(
                    "success={} elapsed_ms={}",
                    result.is_ok(),
                    started.elapsed().as_millis()
                ),
            );
            result
        }
        Err(_) => {
            diagnostics::event(
                "reconciliation_timeout",
                format!(
                    "elapsed_ms={} timeout_ms={}",
                    started.elapsed().as_millis(),
                    RECONCILIATION_TIMEOUT.as_millis()
                ),
            );
            Err("Telegram state refresh timed out".to_string())
        }
    }
}

pub fn apply_initial_state_load_result(
    state: &mut AppState,
    result: std::result::Result<InitialStateLoad, String>,
) {
    state.folders.clear();
    state.chats.clear();
    state.reset_chat_selection();
    state.clear_loaded_chat_messages();

    match result {
        Ok(load) => {
            state.folders = load.folders;
            state.ensure_selected_folder_visible();
            state.chats = load.chats;
            state.cache_selected_folder_chats();
            state.reset_chat_selection();
            match load.messages {
                Ok(messages) if state.chats.is_empty() => {
                    debug_assert!(messages.is_empty());
                    state.clear_loaded_chat_messages();
                }
                Ok(messages) => state.apply_loaded_selected_chat_messages(messages),
                Err(error) => {
                    state.mark_conversation_load_failed();
                    state.set_error(error);
                }
            }
            state.apply_loaded_selected_chat_thread_topics(load.thread_topics);
        }
        Err(error) => {
            state.mark_conversation_load_failed();
            state.set_error(error);
        }
    }
}

pub async fn load_initial_state<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    let result = fetch_initial_state(client)
        .await
        .map_err(color_eyre::eyre::Report::msg)?;
    apply_initial_state_load_result(state, Ok(result));
    Ok(())
}

pub struct FolderChatLoad {
    pub chats: Vec<Chat>,
    pub messages: std::result::Result<Vec<Message>, String>,
    pub thread_topics: Vec<ThreadTopic>,
}

pub async fn fetch_folder_chats_and_selected_messages<C: TelegramClient>(
    client: &C,
    folder_id: Option<i32>,
) -> std::result::Result<FolderChatLoad, String> {
    diagnostics::event(
        "chat_list_load_start",
        format!("folder_id={folder_id:?} limit={CHAT_LIST_PAGE_SIZE}"),
    );
    let started = Instant::now();
    let result = tokio::time::timeout(
        CHAT_LIST_LOAD_TIMEOUT,
        client.get_chats(folder_id, CHAT_LIST_PAGE_SIZE),
    )
    .await;
    let chats = match result {
        Err(_) => {
            diagnostics::event(
                "chat_list_load_timeout",
                format!(
                    "folder_id={folder_id:?} elapsed_ms={} timeout_ms={}",
                    started.elapsed().as_millis(),
                    CHAT_LIST_LOAD_TIMEOUT.as_millis()
                ),
            );
            return Err(LOAD_CHATS_TIMED_OUT_STATUS.to_string());
        }
        Ok(Ok(chats)) => chats,
        Ok(Err(error)) => {
            diagnostics::event(
                "chat_list_load_error",
                format!(
                    "folder_id={folder_id:?} elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            );
            return Err(error.to_string());
        }
    };
    diagnostics::event(
        "chat_list_load_finish",
        format!(
            "folder_id={folder_id:?} count={} elapsed_ms={}",
            chats.len(),
            started.elapsed().as_millis()
        ),
    );
    let (messages, thread_topics) = match chats.first() {
        Some(chat) => {
            let messages = fetch_latest_chat_messages(client, chat.id).await;
            let thread_topics = fetch_chat_thread_topics(client, chat.id)
                .await
                .unwrap_or_default();
            (messages, thread_topics)
        }
        None => (Ok(Vec::new()), Vec::new()),
    };
    Ok(FolderChatLoad {
        chats,
        messages,
        thread_topics,
    })
}

pub fn begin_selected_folder_reload(state: &mut AppState) -> Option<(usize, Option<i32>)> {
    if state.folders.is_empty() {
        return None;
    }

    state.leave_selected_chat();
    let folder_id = state.selected_folder_filter_id();
    if !state.restore_cached_folder_chats(folder_id) {
        state.chats.clear();
        state.reset_chat_selection();
    }
    state.clear_loaded_chat_messages();
    state.begin_conversation_load();
    Some((state.selected_folder_index, folder_id))
}

pub fn apply_folder_chat_load_result(
    state: &mut AppState,
    result: std::result::Result<FolderChatLoad, String>,
) {
    match result {
        Ok(load) => {
            state.chats = load.chats;
            state.cache_selected_folder_chats();
            state.reset_chat_selection();
            match load.messages {
                Ok(messages) if state.chats.is_empty() => {
                    debug_assert!(messages.is_empty());
                    state.clear_loaded_chat_messages();
                }
                Ok(messages) => state.apply_loaded_selected_chat_messages(messages),
                Err(error) => {
                    state.mark_conversation_load_failed();
                    state.set_error(error);
                }
            }
            state.apply_loaded_selected_chat_thread_topics(load.thread_topics);
        }
        Err(error) => {
            state.mark_conversation_load_failed();
            state.set_error(error);
        }
    }
}

#[cfg(test)]
pub async fn reload_selected_folder_chats<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    let Some((_, folder_id)) = begin_selected_folder_reload(state) else {
        return Ok(());
    };
    let result = fetch_folder_chats_and_selected_messages(client, folder_id).await;
    apply_folder_chat_load_result(state, result);
    Ok(())
}

pub fn begin_open_folder_at(
    state: &mut AppState,
    folder_index: usize,
) -> Option<(usize, Option<i32>)> {
    let old_index = state.selected_folder_index;
    state.cache_folder_chats_at(old_index);
    state.select_folder(folder_index);
    if old_index != state.selected_folder_index {
        begin_selected_folder_reload(state)
    } else {
        None
    }
}

#[cfg(test)]
pub async fn submit_message<C: TelegramClient>(state: &mut AppState, client: &mut C) -> Result<()> {
    let Some(action) = state.prepare_message_submit() else {
        return Ok(());
    };

    execute_message_submit_action(state, client, action).await
}

pub async fn execute_message_submit_action<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
    action: MessageSubmitAction,
) -> Result<()> {
    match action {
        MessageSubmitAction::Edit {
            chat_id,
            message_id,
            content,
        } => {
            let result = edit_message_result(client, chat_id, message_id, content.clone()).await;
            apply_edit_message_result(state, message_id, content, result);
        }
        MessageSubmitAction::Reply {
            chat_id,
            thread_top_message_id,
            message_id,
            content,
        } => {
            let result =
                reply_message_result(client, chat_id, thread_top_message_id, message_id, content)
                    .await;
            apply_reply_message_result(state, result);
        }
        MessageSubmitAction::Send {
            chat_id,
            thread_top_message_id,
            content,
        } => {
            let pending = begin_send_message(state, chat_id, thread_top_message_id, content);
            finish_send_message(state, client, pending).await?;
        }
    }

    Ok(())
}

pub async fn edit_message_result<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    message_id: i32,
    content: String,
) -> std::result::Result<(), String> {
    match client.edit_message(chat_id, message_id, content).await {
        Ok(_) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn apply_edit_message_result(
    state: &mut AppState,
    message_id: i32,
    content: String,
    result: std::result::Result<(), String>,
) {
    match result {
        Ok(_) => state.apply_edit_success(message_id, content),
        Err(error) => state.apply_edit_failure(error),
    }
}

pub async fn reply_message_result<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    thread_top_message_id: Option<i32>,
    message_id: i32,
    content: String,
) -> std::result::Result<Message, String> {
    let result = if let Some(topic_id) = thread_top_message_id {
        client
            .reply_to_message_in_thread(chat_id, topic_id, message_id, content)
            .await
    } else {
        client.reply_to_message(chat_id, message_id, content).await
    };
    match result {
        Ok(message) => Ok(message),
        Err(error) => Err(error.to_string()),
    }
}

pub fn apply_reply_message_result(
    state: &mut AppState,
    result: std::result::Result<Message, String>,
) {
    match result {
        Ok(message) => state.apply_reply_success(message),
        Err(error) => state.apply_reply_failure(error),
    }
}

pub fn begin_send_message(
    state: &mut AppState,
    chat_id: i64,
    thread_top_message_id: Option<i32>,
    content: String,
) -> PendingSend {
    let temp_id = TEMP_ID_COUNTER.fetch_sub(1, Ordering::SeqCst);
    state.apply_send_pending(temp_id, chat_id, thread_top_message_id, content.clone());
    PendingSend {
        temp_id,
        chat_id,
        thread_top_message_id,
        content,
    }
}

pub async fn send_message_result<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    thread_top_message_id: Option<i32>,
    content: String,
) -> std::result::Result<Message, String> {
    let result = if let Some(top_message_id) = thread_top_message_id {
        client
            .send_message_to_thread(chat_id, top_message_id, content)
            .await
    } else {
        client.send_message(chat_id, content).await
    };
    match result {
        Ok(sent_msg) => Ok(sent_msg),
        Err(error) => Err(error.to_string()),
    }
}

pub fn apply_send_message_result(
    state: &mut AppState,
    temp_id: i32,
    result: std::result::Result<Message, String>,
) {
    match result {
        Ok(sent_msg) => state.apply_send_success(temp_id, sent_msg),
        Err(error) => state.apply_send_failure(temp_id, error),
    }
}

pub async fn finish_send_message<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
    pending: PendingSend,
) -> Result<()> {
    let result = send_message_result(
        client,
        pending.chat_id,
        pending.thread_top_message_id,
        pending.content,
    )
    .await;
    apply_send_message_result(state, pending.temp_id, result);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        LOAD_CHATS_TIMED_OUT_STATUS, NO_OLDER_MESSAGES_STATUS, apply_initial_state_load_result,
        begin_open_chat_at, begin_open_folder_at, begin_send_message, confirm_delete,
        download_message_media_result, fetch_folder_chats_and_selected_messages,
        fetch_initial_state, fetch_latest_chat_messages, fetch_reconciliation_snapshot,
        finish_send_message, load_initial_state, load_older_messages_failed_error,
        load_older_selected_chat_messages, load_selected_chat_messages,
        load_selected_thread_topic_messages, reload_selected_folder_chats,
        send_typing_action_best_effort, submit_message,
    };
    use crate::state::{
        AppState, ConversationLoadStatus, DeleteConfirmation, MESSAGE_DELETED_STATUS,
        NO_CHAT_SELECTED_ERROR, ReconciliationContext,
    };
    use crate::telegram::types::{
        Chat, Folder, Message, MessageMediaKind, MessageStatus, OWN_SENDER_NAME, ThreadTopic,
        Update, all_folder,
    };
    use crate::telegram::{MockTelegramClient, TelegramClient};
    use chrono::Utc;
    use color_eyre::Result;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn unexpected_client_call(client: &str, action: &str) -> ! {
        panic!("{client} should not {action}")
    }

    fn action_should_succeed<T>(result: Result<T>) -> T {
        result.expect("test action should succeed")
    }

    fn test_download_dir(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "dumbgram-tui-test-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock should be after epoch")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn chat(id: i64, name: &str) -> Chat {
        Chat {
            id,
            name: name.to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }
    }

    fn message(id: i32, chat_id: i64, content: &str) -> Message {
        Message {
            id,
            chat_id,
            thread_topic_id: None,
            sender_name: OWN_SENDER_NAME.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_content: None,
            media: None,
            status: MessageStatus::Delivered,
            can_edit: true,
            can_delete: true,
            error: None,
        }
    }

    struct EmptyFoldersClient;

    impl TelegramClient for EmptyFoldersClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            Ok(Vec::new())
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            unexpected_client_call("empty-folder initial load", "fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            unexpected_client_call("empty-folder initial load", "fetch messages")
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            unexpected_client_call("empty-folder initial load", "fetch older messages")
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("empty-folder client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("empty-folder client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("empty-folder client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("empty-folder client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("empty-folder client", "subscribe to updates")
        }
    }

    struct FailingMessagesClient;

    impl TelegramClient for FailingMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            unexpected_client_call("failing-message client", "fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            unexpected_client_call("failing-message client", "fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            Err(color_eyre::eyre::eyre!("history unavailable"))
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            Err(color_eyre::eyre::eyre!("older history unavailable"))
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("failing-message client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("failing-message client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("failing-message client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("failing-message client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("failing-message client", "subscribe to updates")
        }
    }

    struct SlowMessagesClient;

    impl TelegramClient for SlowMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            unexpected_client_call("slow-message client", "fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            unexpected_client_call("slow-message client", "fetch chats")
        }

        async fn get_messages(&self, chat_id: i64, limit: usize) -> Result<Vec<Message>> {
            assert_eq!(chat_id, 42);
            assert_eq!(limit, super::MESSAGE_HISTORY_PAGE_SIZE);
            tokio::time::sleep(super::MESSAGE_LOAD_TIMEOUT + Duration::from_millis(5)).await;
            Ok(vec![message(1, 42, "too late")])
        }

        async fn get_messages_before(
            &self,
            chat_id: i64,
            before_message_id: i32,
            limit: usize,
        ) -> Result<Vec<Message>> {
            assert_eq!(chat_id, 42);
            assert_eq!(before_message_id, 10);
            assert_eq!(limit, super::MESSAGE_HISTORY_PAGE_SIZE);
            tokio::time::sleep(super::MESSAGE_LOAD_TIMEOUT + Duration::from_millis(5)).await;
            Ok(vec![message(1, 42, "too late")])
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("slow-message client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("slow-message client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("slow-message client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("slow-message client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("slow-message client", "subscribe to updates")
        }
    }

    struct MarkReadClient {
        marked_chat_ids: Mutex<Vec<i64>>,
    }

    impl TelegramClient for MarkReadClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            unexpected_client_call("mark-read client", "fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            unexpected_client_call("mark-read client", "fetch chats")
        }

        async fn get_messages(&self, chat_id: i64, limit: usize) -> Result<Vec<Message>> {
            assert_eq!(chat_id, 42);
            assert_eq!(limit, super::MESSAGE_HISTORY_PAGE_SIZE);
            Ok(vec![message(1, 42, "read me")])
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            unexpected_client_call("mark-read client", "fetch older messages")
        }

        async fn mark_chat_read(&self, chat_id: i64) -> Result<()> {
            self.marked_chat_ids
                .lock()
                .expect("marked chat ids lock should not be poisoned")
                .push(chat_id);
            Ok(())
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("mark-read client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("mark-read client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("mark-read client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("mark-read client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("mark-read client", "subscribe to updates")
        }
    }

    struct SlowChatsClient;

    impl TelegramClient for SlowChatsClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            unexpected_client_call("slow-chat client", "fetch folders")
        }

        async fn get_chats(&self, folder_id: Option<i32>, limit: usize) -> Result<Vec<Chat>> {
            assert_eq!(folder_id, Some(2));
            assert_eq!(limit, super::CHAT_LIST_PAGE_SIZE);
            tokio::time::sleep(super::CHAT_LIST_LOAD_TIMEOUT + Duration::from_millis(5)).await;
            Ok(vec![chat(42, "too late")])
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            unexpected_client_call("slow-chat client", "fetch messages after chat timeout")
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            unexpected_client_call("slow-chat client", "fetch older messages")
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("slow-chat client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("slow-chat client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("slow-chat client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("slow-chat client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("slow-chat client", "subscribe to updates")
        }
    }

    struct OlderMessagesClient;

    impl TelegramClient for OlderMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            unexpected_client_call("older-message client", "fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            unexpected_client_call("older-message client", "fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            unexpected_client_call("older-message client", "fetch latest messages")
        }

        async fn get_messages_before(
            &self,
            chat_id: i64,
            before_message_id: i32,
            limit: usize,
        ) -> Result<Vec<Message>> {
            assert_eq!(chat_id, 42);
            assert_eq!(before_message_id, 10);
            assert_eq!(limit, super::MESSAGE_HISTORY_PAGE_SIZE);
            Ok(vec![
                message(1, 42, "older one"),
                message(2, 42, "older two"),
            ])
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("older-message client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("older-message client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("older-message client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("older-message client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("older-message client", "subscribe to updates")
        }
    }

    struct ChatPageLimitClient {
        observed_limit: Mutex<Option<usize>>,
    }

    impl TelegramClient for ChatPageLimitClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            Ok(vec![all_folder(0)])
        }

        async fn get_chats(&self, _folder_id: Option<i32>, limit: usize) -> Result<Vec<Chat>> {
            *self
                .observed_limit
                .lock()
                .expect("observed limit lock should not be poisoned") = Some(limit);
            Ok(vec![Chat {
                id: 42,
                name: "Bounded".to_string(),
                last_message: None,
                unread_count: 0,
                is_group: false,
                folder_id: None,
            }])
        }

        #[allow(clippy::manual_async_fn)]
        fn get_messages(
            &self,
            chat_id: i64,
            limit: usize,
        ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
            async move {
                assert_eq!(chat_id, 42);
                assert_eq!(limit, super::MESSAGE_HISTORY_PAGE_SIZE);
                Ok(Vec::new())
            }
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            unexpected_client_call("chat-page-limit client", "fetch older messages")
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("chat-page-limit client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("chat-page-limit client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("chat-page-limit client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("chat-page-limit client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("chat-page-limit client", "subscribe to updates")
        }
    }

    struct NoOlderMessagesClient {
        calls: AtomicUsize,
    }

    impl TelegramClient for NoOlderMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            unexpected_client_call("no-older-message client", "fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            unexpected_client_call("no-older-message client", "fetch chats")
        }

        #[allow(clippy::manual_async_fn)]
        fn get_messages(
            &self,
            _chat_id: i64,
            _limit: usize,
        ) -> impl std::future::Future<Output = Result<Vec<Message>>> + Send + '_ {
            async move { unexpected_client_call("no-older-message client", "fetch latest messages") }
        }

        async fn get_messages_before(
            &self,
            chat_id: i64,
            before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            assert_eq!(chat_id, 42);
            assert_eq!(before_message_id, 10);
            self.calls.fetch_add(1, AtomicOrdering::Relaxed);
            Ok(Vec::new())
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("no-older-message client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("no-older-message client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("no-older-message client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("no-older-message client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("no-older-message client", "subscribe to updates")
        }
    }

    #[tokio::test]
    async fn initial_state_fetch_and_apply_are_split_boundary() {
        let client = MockTelegramClient::new();
        let load = fetch_initial_state(&client)
            .await
            .expect("mock initial state should fetch");
        assert_eq!(load.folders.len(), 3);
        assert_eq!(load.chats.len(), 4);
        assert_eq!(
            load.messages
                .as_ref()
                .expect("mock initial messages should fetch")
                .len(),
            3
        );

        let mut state = AppState::new();
        state.folders = vec![Folder {
            id: 99,
            name: "Stale".to_string(),
            unread_count: 0,
        }];
        state.chats = vec![chat(99, "Stale")];
        state.messages = vec![message(99, 99, "stale")];

        apply_initial_state_load_result(&mut state, Ok(load));

        assert_eq!(state.folders.len(), 3);
        assert_eq!(state.chats.len(), 4);
        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.selected_folder_index, 0);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Loaded
        );
    }

    #[test]
    fn failed_initial_state_result_clears_stale_lists() {
        let mut state = AppState::new();
        state.folders = vec![Folder {
            id: 99,
            name: "Stale".to_string(),
            unread_count: 0,
        }];
        state.chats = vec![chat(99, "Stale")];
        state.messages = vec![message(99, 99, "stale")];

        apply_initial_state_load_result(&mut state, Err("initial unavailable".to_string()));

        assert!(state.folders.is_empty());
        assert!(state.chats.is_empty());
        assert!(state.messages.is_empty());
        assert_eq!(state.error_message.as_deref(), Some("initial unavailable"));
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Failed
        );
    }

    #[test]
    fn opening_another_chat_clears_old_content_and_marks_loading() {
        let mut state = AppState::new();
        state.chats = vec![chat(1, "Alice"), chat(2, "Bob")];
        state.messages = vec![message(10, 1, "Alice message")];

        assert_eq!(begin_open_chat_at(&mut state, 1), Some(2));

        assert_eq!(state.selected_chat_id(), Some(2));
        assert!(state.messages.is_empty());
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Loading
        );
    }

    #[tokio::test]
    async fn load_initial_state_requests_bounded_chat_page() {
        let mut state = AppState::new();
        let mut client = ChatPageLimitClient {
            observed_limit: Mutex::new(None),
        };

        action_should_succeed(load_initial_state(&mut state, &mut client).await);

        assert_eq!(
            *client
                .observed_limit
                .lock()
                .expect("observed limit lock should not be poisoned"),
            Some(super::CHAT_LIST_PAGE_SIZE)
        );
        assert_eq!(state.chats.len(), 1);
    }

    #[tokio::test]
    async fn reconciliation_snapshot_resolves_stable_loaded_view_ids() {
        let client = MockTelegramClient::new();
        let snapshot = fetch_reconciliation_snapshot(
            &client,
            ReconciliationContext {
                folder_id: Some(0),
                chat_id: Some(2),
                topic_id: None,
                message_id: None,
            },
        )
        .await
        .expect("mock reconciliation should succeed");

        assert_eq!(snapshot.selected_folder_id, Some(0));
        assert_eq!(snapshot.selected_chat_id, Some(2));
        assert_eq!(
            snapshot.chat_last_message_ids.get(&2),
            snapshot.messages.last().map(|message| &message.id)
        );
        assert!(snapshot.messages.iter().all(|message| message.chat_id == 2));
    }

    #[tokio::test]
    async fn load_initial_state_loads_folders_chats_and_selected_chat_messages() {
        let mut state = AppState::new();
        let mut client = MockTelegramClient::new();

        action_should_succeed(load_initial_state(&mut state, &mut client).await);

        assert_eq!(state.folders.len(), 3);
        assert_eq!(state.chats.len(), 4);
        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.selected_folder_index, 0);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.messages[0].chat_id, 1);
        assert_eq!(state.selected_message_index, 2);
        assert_eq!(state.chats[0].unread_count, 0);
    }

    #[tokio::test]
    async fn load_initial_state_clears_stale_chats_when_no_folders_are_loaded() {
        let mut state = AppState::new();
        state.folders = vec![Folder {
            id: 1,
            name: "Stale".to_string(),
            unread_count: 1,
        }];
        state.selected_folder_index = 9;
        state.folder_scroll_offset = 3;
        state.chats = vec![Chat {
            id: 99,
            name: "Stale Chat".to_string(),
            last_message: Some("stale preview".to_string()),
            unread_count: 2,
            is_group: false,
            folder_id: None,
        }];
        state.selected_chat_index = 4;
        state.chat_scroll_offset = 2;
        state.messages = vec![message(99, 99, "stale message")];
        state.selected_message_index = 1;
        state.message_scroll_offset = 1;
        state.input_buffer = "stale draft".to_string();
        let mut client = EmptyFoldersClient;

        action_should_succeed(load_initial_state(&mut state, &mut client).await);

        assert!(state.folders.is_empty());
        assert!(state.chats.is_empty());
        assert!(state.messages.is_empty());
        assert_eq!(state.selected_folder_index, 0);
        assert_eq!(state.folder_scroll_offset, 0);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.chat_scroll_offset, 0);
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.message_scroll_offset, 0);
        assert_eq!(state.input_buffer, "");
    }

    #[tokio::test]
    async fn load_selected_chat_messages_fetches_messages_and_updates_loaded_state() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 1,
            name: "Alice".to_string(),
            last_message: None,
            unread_count: 2,
            is_group: false,
            folder_id: None,
        }];
        state.input_buffer = "draft".to_string();
        state.save_current_draft();
        state.input_buffer = "stale".to_string();
        let mut client = MockTelegramClient::new();

        action_should_succeed(load_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.messages[0].chat_id, 1);
        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(state.input_buffer, "draft");
        assert_eq!(state.selected_message_index, 2);
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Loaded
        );
    }

    #[tokio::test]
    async fn message_history_publishes_without_loading_media_previews() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 4,
            name: "Release Channel".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: true,
            folder_id: None,
        }];
        let mut client = MockTelegramClient::new();
        let observer = client.clone();

        action_should_succeed(load_selected_chat_messages(&mut state, &mut client).await);

        assert!(!state.messages.is_empty());
        assert!(state.messages[0].media.is_some());
        assert!(
            state.messages[0]
                .media
                .as_ref()
                .and_then(|media| media.local_path.as_ref())
                .is_none()
        );
        assert_eq!(observer.preview_load_count(), 0);
    }

    #[tokio::test]
    async fn load_selected_thread_topic_messages_replaces_messages_with_topic_history() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 3,
            name: "Work Team".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: true,
            folder_id: None,
        }];
        let mut client = MockTelegramClient::new();
        action_should_succeed(load_selected_chat_messages(&mut state, &mut client).await);
        state.select_next_thread_topic();

        action_should_succeed(load_selected_thread_topic_messages(&mut state, &mut client).await);

        assert_eq!(state.selected_thread_topic().unwrap().title, "Deployments");
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 102);
        assert!(state.messages[0].content.contains("Deployments topic"));
        assert_eq!(state.thread_topics.len(), 2);
    }

    struct TypingRecordingClient {
        typed: Mutex<Option<(i64, Option<i32>)>>,
    }

    impl TelegramClient for TypingRecordingClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            unexpected_client_call("typing client", "fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            unexpected_client_call("typing client", "fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            unexpected_client_call("typing client", "fetch messages")
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            unexpected_client_call("typing client", "fetch older messages")
        }

        fn send_typing_action(
            &self,
            chat_id: i64,
            topic_id: Option<i32>,
        ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
            async move {
                *self.typed.lock().unwrap() = Some((chat_id, topic_id));
                Ok(())
            }
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("typing client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("typing client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("typing client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("typing client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("typing client", "subscribe to updates")
        }
    }

    #[tokio::test]
    async fn send_typing_action_best_effort_preserves_topic_scope() {
        let client = TypingRecordingClient {
            typed: Mutex::new(None),
        };

        send_typing_action_best_effort(&client, 3, Some(101)).await;

        assert_eq!(*client.typed.lock().unwrap(), Some((3, Some(101))));
    }

    struct ThreadReadRecordingClient {
        marked: Mutex<Option<(i64, i32, i32)>>,
    }

    impl TelegramClient for ThreadReadRecordingClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            unexpected_client_call("thread-read client", "fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>, _limit: usize) -> Result<Vec<Chat>> {
            unexpected_client_call("thread-read client", "fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            unexpected_client_call("thread-read client", "fetch messages")
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            unexpected_client_call("thread-read client", "fetch older messages")
        }

        async fn get_thread_messages(
            &self,
            chat_id: i64,
            topic_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            let mut first = message(10, chat_id, "older topic message");
            first.thread_topic_id = Some(topic_id);
            let mut latest = message(30, chat_id, "latest topic message");
            latest.thread_topic_id = Some(topic_id);
            Ok(vec![first, latest])
        }

        fn mark_thread_read(
            &self,
            chat_id: i64,
            topic_id: i32,
            max_message_id: i32,
        ) -> impl std::future::Future<Output = Result<()>> + Send + '_ {
            async move {
                *self.marked.lock().unwrap() = Some((chat_id, topic_id, max_message_id));
                Ok(())
            }
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            unexpected_client_call("thread-read client", "send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            unexpected_client_call("thread-read client", "edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            unexpected_client_call("thread-read client", "reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            unexpected_client_call("thread-read client", "delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            unexpected_client_call("thread-read client", "subscribe to updates")
        }
    }

    #[tokio::test]
    async fn load_selected_thread_topic_messages_marks_thread_read_to_latest_loaded_message() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 3,
            name: "Work Team".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: true,
            folder_id: None,
        }];
        state.apply_loaded_selected_chat_thread_topics(vec![ThreadTopic {
            id: 101,
            title: "General".to_string(),
            top_message_id: 101,
            unread_count: 2,
            is_closed: false,
            is_pinned: false,
        }]);
        let mut client = ThreadReadRecordingClient {
            marked: Mutex::new(None),
        };

        action_should_succeed(load_selected_thread_topic_messages(&mut state, &mut client).await);

        assert_eq!(*client.marked.lock().unwrap(), Some((3, 101, 30)));
        assert_eq!(state.thread_topics[0].unread_count, 0);
    }

    #[tokio::test]
    async fn load_selected_chat_messages_loads_thread_topics_for_threaded_group() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 3,
            name: "Work Team".to_string(),
            last_message: None,
            unread_count: 1,
            is_group: true,
            folder_id: Some(3),
        }];
        let mut client = MockTelegramClient::new();

        action_should_succeed(load_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(state.thread_topics.len(), 2);
        assert_eq!(state.thread_topics[0].title, "General");
        assert_eq!(state.thread_topics[0].id, 101);
        assert_eq!(state.thread_topics[0].top_message_id, 1001);
    }

    #[tokio::test]
    async fn load_older_selected_chat_messages_reports_missing_selected_chat() {
        let mut state = AppState::new();
        let mut client = OlderMessagesClient;

        let loaded =
            action_should_succeed(load_older_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(loaded, 0);
        assert_eq!(state.error_message.as_deref(), Some(NO_CHAT_SELECTED_ERROR));
    }

    #[tokio::test]
    async fn load_older_selected_chat_messages_prepends_and_preserves_anchor() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 42,
            name: "Selected".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        state.messages = vec![
            message(10, 42, "current first"),
            message(20, 42, "current last"),
        ];
        state.selected_message_index = 0;
        state.message_scroll_offset = 0;
        let mut client = OlderMessagesClient;

        let loaded =
            action_should_succeed(load_older_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(loaded, 2);
        assert_eq!(
            state
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 10, 20]
        );
        assert_eq!(state.selected_message_index, 2);
        assert_eq!(state.message_scroll_offset, 2);
        assert_eq!(state.messages[state.selected_message_index].id, 10);
    }

    #[tokio::test]
    async fn load_older_selected_thread_topic_messages_stays_in_topic() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 3,
            name: "Work Team".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: true,
            folder_id: None,
        }];
        state.apply_loaded_selected_chat_thread_topics(vec![ThreadTopic {
            id: 101,
            title: "General".to_string(),
            top_message_id: 1001,
            unread_count: 0,
            is_closed: false,
            is_pinned: false,
        }]);
        state.messages = vec![message(103, 3, "current topic first")];
        let mut client = MockTelegramClient::new();

        let loaded =
            action_should_succeed(load_older_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(loaded, 1);
        assert_eq!(state.messages[0].id, 101);
        assert!(state.messages[0].content.contains("General topic"));
        assert_eq!(state.messages[1].id, 103);
    }

    #[tokio::test]
    async fn load_older_selected_chat_messages_caches_exhausted_history() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 42,
            name: "Selected".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        state.messages = vec![message(10, 42, "current first")];
        let mut client = NoOlderMessagesClient {
            calls: AtomicUsize::new(0),
        };

        let first_loaded =
            action_should_succeed(load_older_selected_chat_messages(&mut state, &mut client).await);
        let second_loaded =
            action_should_succeed(load_older_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(first_loaded, 0);
        assert_eq!(second_loaded, 0);
        assert_eq!(client.calls.load(AtomicOrdering::Relaxed), 1);
        assert!(state.selected_chat_older_history_exhausted());
        assert_eq!(
            state.status_message.as_deref(),
            Some(NO_OLDER_MESSAGES_STATUS)
        );
    }

    #[test]
    fn message_history_page_size_is_conservative_for_real_account_loads() {
        assert_eq!(super::MESSAGE_HISTORY_PAGE_SIZE, 20);
    }

    #[tokio::test]
    async fn download_message_media_result_saves_mock_media_to_requested_directory() {
        let client = MockTelegramClient::new();
        let dir = test_download_dir("media-download");

        let downloaded =
            download_message_media_result(&client, 1, 3, MessageMediaKind::Photo, dir.clone())
                .await
                .expect("mock media should download");

        assert!(downloaded.path.starts_with(&dir));
        assert!(downloaded.bytes > 0);
        assert!(downloaded.path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn load_older_messages_failed_error_uses_shared_prefix() {
        assert_eq!(
            load_older_messages_failed_error("older history unavailable"),
            "Load older messages failed: older history unavailable"
        );
    }

    #[tokio::test]
    async fn load_older_selected_chat_messages_reports_fetch_failure_without_bubbling() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 42,
            name: "Selected".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        state.messages = vec![message(10, 42, "current first")];
        let mut client = FailingMessagesClient;

        let loaded =
            action_should_succeed(load_older_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(loaded, 0);
        assert_eq!(state.messages.len(), 1);
        assert!(!state.selected_chat_older_history_exhausted());
        assert_eq!(
            state.error_message.as_deref(),
            Some(load_older_messages_failed_error("older history unavailable").as_str())
        );
    }

    #[tokio::test]
    async fn fetch_latest_chat_messages_does_not_mark_chat_read_before_apply() {
        let client = MarkReadClient {
            marked_chat_ids: Mutex::new(Vec::new()),
        };

        let messages = fetch_latest_chat_messages(&client, 42)
            .await
            .expect("message load should succeed");

        assert_eq!(messages.len(), 1);
        assert!(
            client
                .marked_chat_ids
                .lock()
                .expect("marked chat ids lock should not be poisoned")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn load_selected_chat_messages_marks_chat_read_after_apply() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 42,
            name: "Selected".to_string(),
            last_message: None,
            unread_count: 3,
            is_group: false,
            folder_id: None,
        }];
        let mut client = MarkReadClient {
            marked_chat_ids: Mutex::new(Vec::new()),
        };

        action_should_succeed(load_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(
            client
                .marked_chat_ids
                .lock()
                .expect("marked chat ids lock should not be poisoned")
                .as_slice(),
            &[42]
        );
    }

    #[tokio::test]
    async fn fetch_folder_chats_times_out_without_loading_messages() {
        let client = SlowChatsClient;

        let result = fetch_folder_chats_and_selected_messages(&client, Some(2)).await;

        assert!(matches!(result, Err(error) if error == LOAD_CHATS_TIMED_OUT_STATUS));
    }

    #[tokio::test]
    async fn load_selected_chat_messages_times_out_without_bubbling() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 42,
            name: "Selected".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        state.messages = vec![message(99, 99, "stale message")];
        let mut client = SlowMessagesClient;

        action_should_succeed(load_selected_chat_messages(&mut state, &mut client).await);

        assert!(state.messages.is_empty());
        assert_eq!(
            state.error_message.as_deref(),
            Some(super::LOAD_MESSAGES_TIMED_OUT_STATUS)
        );
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Failed
        );
    }

    #[tokio::test]
    async fn load_older_selected_chat_messages_times_out_without_mutating_history() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 42,
            name: "Selected".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        state.messages = vec![message(10, 42, "current first")];
        let mut client = SlowMessagesClient;

        let loaded =
            action_should_succeed(load_older_selected_chat_messages(&mut state, &mut client).await);

        assert_eq!(loaded, 0);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "current first");
        assert!(!state.selected_chat_older_history_exhausted());
        assert_eq!(
            state.error_message.as_deref(),
            Some(super::LOAD_OLDER_MESSAGES_TIMED_OUT_STATUS)
        );
    }

    #[tokio::test]
    async fn load_selected_chat_messages_clears_stale_messages_before_failed_fetch() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 42,
            name: "Selected".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        state.messages = vec![message(99, 99, "stale message")];
        state.selected_message_index = 1;
        state.message_scroll_offset = 1;
        state.input_buffer = "stale input".to_string();
        let mut client = FailingMessagesClient;

        let result = load_selected_chat_messages(&mut state, &mut client).await;

        action_should_succeed(result);
        assert!(state.messages.is_empty());
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.message_scroll_offset, 0);
        assert_eq!(state.input_buffer, "");
        assert_eq!(state.error_message.as_deref(), Some("history unavailable"));
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Failed
        );
    }

    #[test]
    fn begin_open_folder_at_restores_cached_chats_while_refreshing() {
        let mut state = AppState::new();
        state.folders = vec![
            all_folder(0),
            Folder {
                id: 2,
                name: "Personal".to_string(),
                unread_count: 0,
            },
            Folder {
                id: 5,
                name: "Work".to_string(),
                unread_count: 0,
            },
        ];
        state.selected_folder_index = 1;
        state.chats = vec![Chat {
            id: 10,
            name: "Personal cached".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: Some(2),
        }];
        state.cache_selected_folder_chats();
        state.selected_folder_index = 2;
        state.chats = vec![Chat {
            id: 20,
            name: "Work loaded".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: Some(5),
        }];
        state.messages = vec![message(20, 20, "stale work message")];

        assert_eq!(begin_open_folder_at(&mut state, 1), Some((1, Some(2))));

        assert_eq!(state.selected_folder_index, 1);
        assert_eq!(state.chats.len(), 1);
        assert_eq!(state.chats[0].id, 10);
        assert!(state.messages.is_empty());
        assert_eq!(state.selected_chat_index, 0);
    }

    #[tokio::test]
    async fn reload_selected_folder_chats_saves_draft_and_loads_selected_folder() {
        let mut state = AppState::new();
        state.folders = vec![
            all_folder(5),
            crate::telegram::types::Folder {
                id: 2,
                name: "Personal".to_string(),
                unread_count: 3,
            },
        ];
        state.selected_folder_index = 1;
        state.chats = vec![Chat {
            id: 1,
            name: "Old Alice".to_string(),
            last_message: None,
            unread_count: 2,
            is_group: false,
            folder_id: Some(2),
        }];
        state.input_buffer = "alice draft".to_string();
        let mut client = MockTelegramClient::new();

        action_should_succeed(reload_selected_folder_chats(&mut state, &mut client).await);

        assert_eq!(state.chats.len(), 2);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.chats[0].id, 1);
        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.input_buffer, "alice draft");
    }

    #[tokio::test]
    async fn confirm_delete_deletes_confirmed_message() {
        let mut state = AppState::new();
        state.messages = vec![message(1, 10, "keep"), message(2, 10, "delete")];
        state.selected_message_index = 1;
        state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 10,
            message_id: 2,
        });
        let mut client = MockTelegramClient::new();

        action_should_succeed(confirm_delete(&mut state, &mut client).await);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 1);
        assert_eq!(state.selected_message_index, 0);
        assert!(state.delete_confirmation().is_none());
        assert_eq!(
            state.status_message.as_deref(),
            Some(MESSAGE_DELETED_STATUS)
        );
    }

    #[tokio::test]
    async fn split_send_action_exposes_pending_message_before_network_finish() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 10,
            name: "Alice".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        let mut client = MockTelegramClient::new();

        let pending = begin_send_message(&mut state, 10, None, "hello".to_string());

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "hello");
        assert_eq!(state.messages[0].status, MessageStatus::Sending);
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.chats[0].last_message.as_deref(), Some("hello"));

        action_should_succeed(finish_send_message(&mut state, &mut client, pending).await);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 999);
        assert_eq!(state.messages[0].status, MessageStatus::Sent);
        assert!(state.status_message.is_none());
    }

    #[tokio::test]
    async fn finish_send_message_uses_selected_thread_topic_when_present() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 3,
            name: "Work Team".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: true,
            folder_id: None,
        }];
        let mut client = MockTelegramClient::new();

        let pending = begin_send_message(&mut state, 3, Some(102), "ship it".to_string());
        action_should_succeed(finish_send_message(&mut state, &mut client, pending).await);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 1102);
        assert_eq!(state.messages[0].content, "ship it");
        assert_eq!(
            state.messages[0].reply_to_content.as_deref(),
            Some("topic 102")
        );
        assert_eq!(state.messages[0].status, MessageStatus::Sent);
    }

    #[tokio::test]
    async fn submit_message_sends_plain_input_and_selects_sent_message() {
        let mut state = AppState::new();
        state.chats = vec![Chat {
            id: 10,
            name: "Alice".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        state.input_buffer = "hello".to_string();
        let mut client = MockTelegramClient::new();

        action_should_succeed(submit_message(&mut state, &mut client).await);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 999);
        assert_eq!(state.messages[0].content, "hello");
        assert_eq!(state.messages[0].status, MessageStatus::Sent);
        assert_eq!(state.selected_message_index, 0);
        assert!(state.status_message.is_none());
        assert!(state.input_buffer.is_empty());
    }
}
