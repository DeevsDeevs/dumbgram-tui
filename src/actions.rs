use crate::diagnostics;
use crate::state::{AppState, DeleteConfirmation, MessageSubmitAction, NO_CHAT_SELECTED_ERROR};
use crate::telegram::{
    DownloadedMedia, TelegramClient,
    types::{ALL_FOLDER_ID, Chat, Folder, Message, MessageMediaKind},
};
use color_eyre::Result;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

static TEMP_ID_COUNTER: AtomicI32 = AtomicI32::new(-1);
pub(crate) const CHAT_LIST_PAGE_SIZE: usize = 50;
const MESSAGE_HISTORY_PAGE_SIZE: usize = 20;
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
    pub(crate) content: String,
}

pub fn begin_confirm_delete(state: &mut AppState) -> Option<DeleteConfirmation> {
    let confirmation = state.delete_confirmation?;
    state.delete_confirmation = None;
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
        return state.selected_chat_id();
    }

    None
}

pub async fn load_selected_chat_messages<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    if let Some(chat_id) = state.selected_chat_id() {
        state.clear_loaded_chat_messages();
        match fetch_latest_chat_messages(client, chat_id).await {
            Ok(messages) => {
                state.apply_loaded_selected_chat_messages(messages);
                mark_chat_read_best_effort(client, chat_id).await;
            }
            Err(error) => state.set_error(error),
        }
    } else {
        state.clear_loaded_chat_messages();
    }

    Ok(())
}

pub async fn fetch_older_chat_messages<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    before_message_id: i32,
) -> std::result::Result<Vec<Message>, String> {
    diagnostics::event(
        "older_messages_load_start",
        format!(
            "chat_id={chat_id} before_message_id={before_message_id} limit={MESSAGE_HISTORY_PAGE_SIZE}"
        ),
    );
    let started = Instant::now();
    match tokio::time::timeout(
        MESSAGE_LOAD_TIMEOUT,
        client.get_messages_before(chat_id, before_message_id, MESSAGE_HISTORY_PAGE_SIZE),
    )
    .await
    {
        Err(_) => {
            diagnostics::event(
                "older_messages_load_timeout",
                format!(
                    "chat_id={chat_id} elapsed_ms={} timeout_ms={}",
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
        Ok(Err(error)) => {
            diagnostics::event(
                "older_messages_load_error",
                format!(
                    "chat_id={chat_id} elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            );
            Err(load_older_messages_failed_error(error))
        }
    }
}

pub fn selected_older_messages_request(state: &mut AppState) -> Option<(i64, i32)> {
    let Some(chat_id) = state.selected_chat_id() else {
        state.set_error(NO_CHAT_SELECTED_ERROR.to_string());
        return None;
    };
    let before_message_id = state.messages.first().map(|message| message.id)?;
    if state.selected_chat_older_history_exhausted() {
        state.set_status(NO_OLDER_MESSAGES_STATUS);
        return None;
    }

    Some((chat_id, before_message_id))
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
    let Some((chat_id, before_message_id)) = selected_older_messages_request(state) else {
        return Ok(0);
    };
    let result = fetch_older_chat_messages(client, chat_id, before_message_id).await;
    Ok(apply_older_chat_messages_result(state, result))
}

pub struct InitialStateLoad {
    pub folders: Vec<Folder>,
    pub chats: Vec<Chat>,
    pub messages: std::result::Result<Vec<Message>, String>,
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

    let (chats, messages) = if let Some(folder) = folders.first() {
        let folder_id = folder_filter_id(folder);
        match fetch_folder_chats_and_selected_messages(client, folder_id).await {
            Ok(load) => (load.chats, load.messages),
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
        (Vec::new(), Ok(Vec::new()))
    };

    diagnostics::event(
        "initial_load_finish",
        format!(
            "folders={} chats={} messages={} elapsed_ms={}",
            folders.len(),
            chats.len(),
            messages.as_ref().map_or(0, Vec::len),
            started.elapsed().as_millis()
        ),
    );

    Ok(InitialStateLoad {
        folders,
        chats,
        messages,
    })
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
                Err(error) => state.set_error(error),
            }
        }
        Err(error) => state.set_error(error),
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
    let messages = match chats.first() {
        Some(chat) => fetch_latest_chat_messages(client, chat.id).await,
        None => Ok(Vec::new()),
    };
    Ok(FolderChatLoad { chats, messages })
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
                Err(error) => state.set_error(error),
            }
        }
        Err(error) => state.set_error(error),
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

pub async fn open_folder_at<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
    folder_index: usize,
) -> Result<()> {
    if let Some((_, folder_id)) = begin_open_folder_at(state, folder_index) {
        let result = fetch_folder_chats_and_selected_messages(client, folder_id).await;
        apply_folder_chat_load_result(state, result);
    }

    Ok(())
}

#[cfg(test)]
pub async fn open_next_folder<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    let old_index = state.selected_folder_index;
    state.select_next_folder();
    state.ensure_selected_folder_visible();
    if old_index != state.selected_folder_index && !state.folders.is_empty() {
        reload_selected_folder_chats(state, client).await?;
    }

    Ok(())
}

#[cfg(test)]
pub async fn open_previous_folder<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    let old_index = state.selected_folder_index;
    state.select_prev_folder();
    state.ensure_selected_folder_visible();
    if old_index != state.selected_folder_index && !state.folders.is_empty() {
        reload_selected_folder_chats(state, client).await?;
    }

    Ok(())
}

pub async fn open_chat_at<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
    chat_index: usize,
) -> Result<()> {
    if chat_index >= state.chats.len() {
        return Ok(());
    }

    if begin_open_chat_at(state, chat_index).is_some() {
        load_selected_chat_messages(state, client).await?;
    }

    Ok(())
}

#[cfg(test)]
pub async fn open_next_chat<C: TelegramClient>(state: &mut AppState, client: &mut C) -> Result<()> {
    if state.selected_chat_index + 1 >= state.chats.len() {
        return Ok(());
    }

    open_chat_at(state, client, state.selected_chat_index + 1).await
}

#[cfg(test)]
pub async fn open_previous_chat<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    if state.selected_chat_index == 0 || state.chats.is_empty() {
        return Ok(());
    }

    open_chat_at(state, client, state.selected_chat_index - 1).await
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
            message_id,
            content,
        } => {
            let result = reply_message_result(client, chat_id, message_id, content).await;
            apply_reply_message_result(state, result);
        }
        MessageSubmitAction::Send { chat_id, content } => {
            let pending = begin_send_message(state, chat_id, content);
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
    message_id: i32,
    content: String,
) -> std::result::Result<Message, String> {
    match client.reply_to_message(chat_id, message_id, content).await {
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

pub fn begin_send_message(state: &mut AppState, chat_id: i64, content: String) -> PendingSend {
    let temp_id = TEMP_ID_COUNTER.fetch_sub(1, Ordering::SeqCst);
    state.apply_send_pending(temp_id, chat_id, content.clone());
    PendingSend {
        temp_id,
        chat_id,
        content,
    }
}

pub async fn send_message_result<C: TelegramClient>(
    client: &C,
    chat_id: i64,
    content: String,
) -> std::result::Result<Message, String> {
    match client.send_message(chat_id, content).await {
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
    let result = send_message_result(client, pending.chat_id, pending.content).await;
    apply_send_message_result(state, pending.temp_id, result);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        LOAD_CHATS_TIMED_OUT_STATUS, NO_OLDER_MESSAGES_STATUS, apply_initial_state_load_result,
        begin_open_folder_at, begin_send_message, confirm_delete, download_message_media_result,
        fetch_folder_chats_and_selected_messages, fetch_initial_state, fetch_latest_chat_messages,
        finish_send_message, load_initial_state, load_older_messages_failed_error,
        load_older_selected_chat_messages, load_selected_chat_messages, open_chat_at,
        open_folder_at, open_next_chat, open_next_folder, open_previous_chat, open_previous_folder,
        reload_selected_folder_chats, submit_message,
    };
    use crate::state::{
        AppState, DeleteConfirmation, MESSAGE_DELETED_STATUS, NO_CHAT_SELECTED_ERROR,
    };
    use crate::telegram::types::{
        Chat, Folder, Message, MessageMediaKind, MessageStatus, OWN_SENDER_NAME, Update, all_folder,
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
    }

    #[tokio::test]
    async fn open_folder_at_loads_target_folder_when_selection_changes() {
        let mut state = AppState::new();
        let mut client = MockTelegramClient::new();
        action_should_succeed(load_initial_state(&mut state, &mut client).await);
        state.input_buffer = "alice draft".to_string();

        action_should_succeed(open_folder_at(&mut state, &mut client, 1).await);

        assert_eq!(state.selected_folder_index, 1);
        assert_eq!(state.chats.len(), 2);
        assert!(state.chats.iter().all(|chat| chat.folder_id == Some(2)));
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.messages.len(), 3);
        assert_eq!(
            state.chat_drafts.get(&1).map(String::as_str),
            Some("alice draft")
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
    async fn open_folder_at_does_not_reload_when_selection_is_unchanged() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![Chat {
            id: 99,
            name: "Local Only".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        state.messages = vec![message(99, 99, "keep local messages")];
        let mut client = MockTelegramClient::new();

        action_should_succeed(open_folder_at(&mut state, &mut client, 0).await);

        assert_eq!(state.chats.len(), 1);
        assert_eq!(state.chats[0].id, 99);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 99);
    }

    #[tokio::test]
    async fn open_next_and_previous_folder_wrap_and_load_chats() {
        let mut state = AppState::new();
        let mut client = MockTelegramClient::new();
        action_should_succeed(load_initial_state(&mut state, &mut client).await);

        action_should_succeed(open_previous_folder(&mut state, &mut client).await);

        assert_eq!(state.selected_folder_index, 2);
        assert_eq!(state.chats.len(), 2);
        assert!(state.chats.iter().all(|chat| chat.folder_id == Some(3)));

        action_should_succeed(open_next_folder(&mut state, &mut client).await);

        assert_eq!(state.selected_folder_index, 0);
        assert_eq!(state.chats.len(), 4);
    }

    #[tokio::test]
    async fn open_chat_at_saves_current_draft_and_loads_target_chat() {
        let mut state = AppState::new();
        state.chats = vec![
            Chat {
                id: 1,
                name: "Alice".to_string(),
                last_message: None,
                unread_count: 0,
                is_group: false,
                folder_id: None,
            },
            Chat {
                id: 2,
                name: "Bob".to_string(),
                last_message: None,
                unread_count: 4,
                is_group: false,
                folder_id: None,
            },
        ];
        state.input_buffer = "alice draft".to_string();
        let mut client = MockTelegramClient::new();

        action_should_succeed(open_chat_at(&mut state, &mut client, 1).await);

        assert_eq!(state.selected_chat_index, 1);
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[0].chat_id, 2);
        assert_eq!(state.chats[1].unread_count, 0);
        assert_eq!(
            state.chat_drafts.get(&1).map(String::as_str),
            Some("alice draft")
        );
        assert_eq!(state.input_buffer, "");
    }

    #[tokio::test]
    async fn open_next_and_previous_chat_stop_at_list_boundaries() {
        let mut state = AppState::new();
        state.chats = vec![
            Chat {
                id: 1,
                name: "Alice".to_string(),
                last_message: None,
                unread_count: 0,
                is_group: false,
                folder_id: None,
            },
            Chat {
                id: 2,
                name: "Bob".to_string(),
                last_message: None,
                unread_count: 0,
                is_group: false,
                folder_id: None,
            },
        ];
        state.messages = vec![message(10, 1, "keep")];
        let mut client = MockTelegramClient::new();

        action_should_succeed(open_previous_chat(&mut state, &mut client).await);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.messages[0].content, "keep");

        state.selected_chat_index = 1;
        action_should_succeed(open_next_chat(&mut state, &mut client).await);
        assert_eq!(state.selected_chat_index, 1);
        assert_eq!(state.messages[0].content, "keep");
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
        state.delete_confirmation = Some(DeleteConfirmation {
            chat_id: 10,
            message_id: 2,
        });
        let mut client = MockTelegramClient::new();

        action_should_succeed(confirm_delete(&mut state, &mut client).await);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 1);
        assert_eq!(state.selected_message_index, 0);
        assert!(state.delete_confirmation.is_none());
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

        let pending = begin_send_message(&mut state, 10, "hello".to_string());

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
