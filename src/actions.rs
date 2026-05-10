use crate::state::{AppState, MessageSubmitAction};
use crate::telegram::TelegramClient;
use color_eyre::Result;
use std::sync::atomic::{AtomicI32, Ordering};

static TEMP_ID_COUNTER: AtomicI32 = AtomicI32::new(-1);
const MESSAGE_HISTORY_PAGE_SIZE: usize = 50;

pub struct PendingSend {
    temp_id: i32,
    chat_id: i64,
    content: String,
}

pub async fn confirm_delete<C: TelegramClient>(state: &mut AppState, client: &mut C) -> Result<()> {
    let Some(confirmation) = state.delete_confirmation else {
        return Ok(());
    };

    match client
        .delete_message(confirmation.chat_id, confirmation.message_id)
        .await
    {
        Ok(_) => state.apply_delete_success(confirmation),
        Err(e) => state.apply_delete_failure(e.to_string()),
    }

    Ok(())
}

pub async fn load_selected_chat_messages<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    if let Some(chat_id) = state.selected_chat_id() {
        state.clear_loaded_chat_messages();
        let messages = client
            .get_messages(chat_id, MESSAGE_HISTORY_PAGE_SIZE)
            .await?;
        state.apply_loaded_selected_chat_messages(messages);
    } else {
        state.clear_loaded_chat_messages();
    }

    Ok(())
}

pub async fn load_older_selected_chat_messages<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<usize> {
    let Some(chat_id) = state.selected_chat_id() else {
        state.set_error("No chat selected".to_string());
        return Ok(0);
    };
    let Some(before_message_id) = state.messages.first().map(|message| message.id) else {
        return Ok(0);
    };
    if state.selected_chat_older_history_exhausted() {
        state.set_status("No older messages");
        return Ok(0);
    }

    let older_messages = match client
        .get_messages_before(chat_id, before_message_id, MESSAGE_HISTORY_PAGE_SIZE)
        .await
    {
        Ok(messages) => messages,
        Err(error) => {
            state.set_error(format!("Load older messages failed: {}", error));
            return Ok(0);
        }
    };
    let added = state.prepend_loaded_selected_chat_messages(older_messages);
    if added == 0 {
        state.mark_selected_chat_older_history_exhausted();
        state.set_status("No older messages");
    }
    Ok(added)
}

pub async fn load_initial_state<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    state.folders = client.get_folders().await?;
    state.ensure_selected_folder_visible();
    state.chats.clear();
    state.reset_chat_selection();
    state.clear_loaded_chat_messages();

    if !state.folders.is_empty() {
        state.chats = client.get_chats(state.selected_folder_filter_id()).await?;
        state.reset_chat_selection();
        load_selected_chat_messages(state, client).await?;
    }

    Ok(())
}

pub async fn reload_selected_folder_chats<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    state.leave_selected_chat();
    state.chats = client.get_chats(state.selected_folder_filter_id()).await?;
    state.reset_chat_selection();
    load_selected_chat_messages(state, client).await
}

pub async fn open_folder_at<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
    folder_index: usize,
) -> Result<()> {
    let old_index = state.selected_folder_index;
    state.select_folder(folder_index);
    if old_index != state.selected_folder_index && !state.folders.is_empty() {
        reload_selected_folder_chats(state, client).await?;
    }

    Ok(())
}

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

    let old_index = state.selected_chat_index;
    if old_index != chat_index {
        state.leave_selected_chat();
    }
    state.select_chat(chat_index);
    if old_index != state.selected_chat_index {
        load_selected_chat_messages(state, client).await?;
    }

    Ok(())
}

pub async fn open_next_chat<C: TelegramClient>(state: &mut AppState, client: &mut C) -> Result<()> {
    if state.chats.is_empty() {
        return Ok(());
    }

    let next_index = (state.selected_chat_index + 1) % state.chats.len();
    open_chat_at(state, client, next_index).await
}

pub async fn open_previous_chat_wrapping<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
) -> Result<()> {
    if state.chats.is_empty() {
        return Ok(());
    }

    let previous_index = if state.selected_chat_index == 0 {
        state.chats.len() - 1
    } else {
        state.selected_chat_index - 1
    };
    open_chat_at(state, client, previous_index).await
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
        } => match client
            .edit_message(chat_id, message_id, content.clone())
            .await
        {
            Ok(_) => state.apply_edit_success(message_id, content),
            Err(e) => state.apply_edit_failure(e.to_string()),
        },
        MessageSubmitAction::Reply {
            chat_id,
            message_id,
            content,
        } => match client.reply_to_message(chat_id, message_id, content).await {
            Ok(new_msg) => state.apply_reply_success(new_msg),
            Err(e) => state.apply_reply_failure(e.to_string()),
        },
        MessageSubmitAction::Send { chat_id, content } => {
            let pending = begin_send_message(state, chat_id, content);
            finish_send_message(state, client, pending).await?;
        }
    }

    Ok(())
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

pub async fn finish_send_message<C: TelegramClient>(
    state: &mut AppState,
    client: &mut C,
    pending: PendingSend,
) -> Result<()> {
    match client.send_message(pending.chat_id, pending.content).await {
        Ok(sent_msg) => state.apply_send_success(pending.temp_id, sent_msg),
        Err(e) => state.apply_send_failure(pending.temp_id, e.to_string()),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        begin_send_message, confirm_delete, finish_send_message, load_initial_state,
        load_older_selected_chat_messages, load_selected_chat_messages, open_chat_at,
        open_folder_at, open_next_folder, open_previous_folder, reload_selected_folder_chats,
        submit_message,
    };
    use crate::state::{AppState, DeleteConfirmation};
    use crate::telegram::types::{Chat, Folder, Message, MessageStatus, Update};
    use crate::telegram::{MockTelegramClient, TelegramClient};
    use chrono::Utc;
    use color_eyre::Result;
    use std::cell::Cell;
    use tokio::sync::mpsc;

    fn message(id: i32, chat_id: i64, content: &str) -> Message {
        Message {
            id,
            chat_id,
            sender_name: "You".to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_content: None,
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

        async fn get_chats(&self, _folder_id: Option<i32>) -> Result<Vec<Chat>> {
            panic!("empty-folder initial load should not fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            panic!("empty-folder initial load should not fetch messages")
        }

        async fn get_messages_before(
            &self,
            _chat_id: i64,
            _before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            panic!("empty-folder initial load should not fetch older messages")
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            panic!("empty-folder client should not send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            panic!("empty-folder client should not edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            panic!("empty-folder client should not reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            panic!("empty-folder client should not delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            panic!("empty-folder client should not subscribe to updates")
        }
    }

    struct FailingMessagesClient;

    impl TelegramClient for FailingMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            panic!("failing-message client should not fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>) -> Result<Vec<Chat>> {
            panic!("failing-message client should not fetch chats")
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
            panic!("failing-message client should not send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            panic!("failing-message client should not edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            panic!("failing-message client should not reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            panic!("failing-message client should not delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            panic!("failing-message client should not subscribe to updates")
        }
    }

    struct OlderMessagesClient;

    impl TelegramClient for OlderMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            panic!("older-message client should not fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>) -> Result<Vec<Chat>> {
            panic!("older-message client should not fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            panic!("older-message client should not fetch latest messages")
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
            panic!("older-message client should not send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            panic!("older-message client should not edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            panic!("older-message client should not reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            panic!("older-message client should not delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            panic!("older-message client should not subscribe to updates")
        }
    }

    struct NoOlderMessagesClient {
        calls: Cell<usize>,
    }

    impl TelegramClient for NoOlderMessagesClient {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }

        async fn get_folders(&self) -> Result<Vec<Folder>> {
            panic!("no-older-message client should not fetch folders")
        }

        async fn get_chats(&self, _folder_id: Option<i32>) -> Result<Vec<Chat>> {
            panic!("no-older-message client should not fetch chats")
        }

        async fn get_messages(&self, _chat_id: i64, _limit: usize) -> Result<Vec<Message>> {
            panic!("no-older-message client should not fetch latest messages")
        }

        async fn get_messages_before(
            &self,
            chat_id: i64,
            before_message_id: i32,
            _limit: usize,
        ) -> Result<Vec<Message>> {
            assert_eq!(chat_id, 42);
            assert_eq!(before_message_id, 10);
            self.calls.set(self.calls.get() + 1);
            Ok(Vec::new())
        }

        async fn send_message(&self, _chat_id: i64, _content: String) -> Result<Message> {
            panic!("no-older-message client should not send messages")
        }

        async fn edit_message(
            &self,
            _chat_id: i64,
            _message_id: i32,
            _content: String,
        ) -> Result<()> {
            panic!("no-older-message client should not edit messages")
        }

        async fn reply_to_message(
            &self,
            _chat_id: i64,
            _reply_to: i32,
            _content: String,
        ) -> Result<Message> {
            panic!("no-older-message client should not reply to messages")
        }

        async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<()> {
            panic!("no-older-message client should not delete messages")
        }

        async fn subscribe_updates(&mut self) -> Result<mpsc::UnboundedReceiver<Update>> {
            panic!("no-older-message client should not subscribe to updates")
        }
    }

    #[tokio::test]
    async fn load_initial_state_loads_folders_chats_and_selected_chat_messages() {
        let mut state = AppState::new();
        let mut client = MockTelegramClient::new();

        load_initial_state(&mut state, &mut client).await.unwrap();

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

        load_initial_state(&mut state, &mut client).await.unwrap();

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

        load_selected_chat_messages(&mut state, &mut client)
            .await
            .unwrap();

        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.messages[0].chat_id, 1);
        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(state.input_buffer, "draft");
        assert_eq!(state.selected_message_index, 2);
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

        let loaded = load_older_selected_chat_messages(&mut state, &mut client)
            .await
            .unwrap();

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
            calls: Cell::new(0),
        };

        let first_loaded = load_older_selected_chat_messages(&mut state, &mut client)
            .await
            .unwrap();
        let second_loaded = load_older_selected_chat_messages(&mut state, &mut client)
            .await
            .unwrap();

        assert_eq!(first_loaded, 0);
        assert_eq!(second_loaded, 0);
        assert_eq!(client.calls.get(), 1);
        assert!(state.selected_chat_older_history_exhausted());
        assert_eq!(state.status_message.as_deref(), Some("No older messages"));
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

        let loaded = load_older_selected_chat_messages(&mut state, &mut client)
            .await
            .unwrap();

        assert_eq!(loaded, 0);
        assert_eq!(state.messages.len(), 1);
        assert!(!state.selected_chat_older_history_exhausted());
        assert_eq!(
            state.error_message.as_deref(),
            Some("Load older messages failed: older history unavailable")
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

        assert!(result.is_err());
        assert!(state.messages.is_empty());
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.message_scroll_offset, 0);
        assert_eq!(state.input_buffer, "");
    }

    #[tokio::test]
    async fn open_folder_at_loads_target_folder_when_selection_changes() {
        let mut state = AppState::new();
        let mut client = MockTelegramClient::new();
        load_initial_state(&mut state, &mut client).await.unwrap();
        state.input_buffer = "alice draft".to_string();

        open_folder_at(&mut state, &mut client, 1).await.unwrap();

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

    #[tokio::test]
    async fn open_folder_at_does_not_reload_when_selection_is_unchanged() {
        let mut state = AppState::new();
        state.folders = vec![crate::telegram::types::Folder {
            id: 1,
            name: "All".to_string(),
            unread_count: 0,
        }];
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

        open_folder_at(&mut state, &mut client, 0).await.unwrap();

        assert_eq!(state.chats.len(), 1);
        assert_eq!(state.chats[0].id, 99);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 99);
    }

    #[tokio::test]
    async fn open_next_and_previous_folder_wrap_and_load_chats() {
        let mut state = AppState::new();
        let mut client = MockTelegramClient::new();
        load_initial_state(&mut state, &mut client).await.unwrap();

        open_previous_folder(&mut state, &mut client).await.unwrap();

        assert_eq!(state.selected_folder_index, 2);
        assert_eq!(state.chats.len(), 2);
        assert!(state.chats.iter().all(|chat| chat.folder_id == Some(3)));

        open_next_folder(&mut state, &mut client).await.unwrap();

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

        open_chat_at(&mut state, &mut client, 1).await.unwrap();

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
    async fn reload_selected_folder_chats_saves_draft_and_loads_selected_folder() {
        let mut state = AppState::new();
        state.folders = vec![
            crate::telegram::types::Folder {
                id: 1,
                name: "All".to_string(),
                unread_count: 5,
            },
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

        reload_selected_folder_chats(&mut state, &mut client)
            .await
            .unwrap();

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

        confirm_delete(&mut state, &mut client).await.unwrap();

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 1);
        assert_eq!(state.selected_message_index, 0);
        assert!(state.delete_confirmation.is_none());
        assert_eq!(state.status_message.as_deref(), Some("Message deleted"));
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

        finish_send_message(&mut state, &mut client, pending)
            .await
            .unwrap();

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 999);
        assert_eq!(state.messages[0].status, MessageStatus::Sent);
        assert_eq!(state.status_message.as_deref(), Some("Message sent"));
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

        submit_message(&mut state, &mut client).await.unwrap();

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 999);
        assert_eq!(state.messages[0].content, "hello");
        assert_eq!(state.messages[0].status, MessageStatus::Sent);
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.status_message.as_deref(), Some("Message sent"));
        assert!(state.input_buffer.is_empty());
    }
}
