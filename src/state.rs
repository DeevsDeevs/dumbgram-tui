use crate::diagnostics;
use crate::telegram::types::{
    Chat, Folder, Message, MessageMediaKind, MessageStatus, OWN_SENDER_NAME, SenderIdentity,
    ThreadTopic, UNKNOWN_DELETE_UPDATE_CHAT_ID, Update, is_all_folder, message_display_content,
    message_display_preview,
};
use crate::text::{display_width, wrap_display_lines_limited};
use chrono::Utc;
use ratatui::layout::Rect;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const FOLDER_LEFT_SCROLL_INDICATOR: &str = "◀ ";
pub(crate) const FOLDER_SEPARATOR: &str = " │ ";
pub(crate) const FOLDER_RIGHT_SCROLL_INDICATOR: &str = " ▶";
pub(crate) const FOLDER_LABEL_HORIZONTAL_PADDING: &str = "  ";
pub(crate) const THREAD_TOPIC_LABEL_HORIZONTAL_PADDING: &str = "  ";
pub(crate) const NO_CHAT_SELECTED_ERROR: &str = "No chat selected";
pub(crate) const MESSAGE_EDITED_STATUS: &str = "Message edited";
pub(crate) const REPLY_SENT_STATUS: &str = "Reply sent";
pub(crate) const MESSAGE_DELETED_STATUS: &str = "Message deleted";
pub(crate) const EDIT_FAILED_PREFIX: &str = "Edit failed";
pub(crate) const REPLY_FAILED_PREFIX: &str = "Reply failed";
pub(crate) const SEND_FAILED_PREFIX: &str = "Send failed";
pub(crate) const DELETE_FAILED_PREFIX: &str = "Delete failed";
pub(crate) const MARK_READ_FAILED_PREFIX: &str = "Mark read failed";
pub(crate) const CANNOT_EDIT_MESSAGE_ERROR: &str = "Cannot edit this message";
pub(crate) const CANNOT_REPLY_UNSENT_MESSAGE_ERROR: &str = "Cannot reply to unsent message";
pub(crate) const CANNOT_DELETE_MESSAGE_ERROR: &str = "Cannot delete this message";
pub(crate) const FAILED_SEND_DISMISSED_STATUS: &str = "Failed send dismissed";
pub(crate) const REMOTE_EDIT_WHILE_EDITING_STATUS: &str = "Message updated remotely while editing";
pub(crate) const DEFAULT_SPLIT_RATIO: f32 = 0.3;
pub(crate) const SPLIT_RATIO_STEP: f32 = 0.05;
pub(crate) const SPLIT_DRAG_THRESHOLD_COLUMNS: u16 = 1;
pub(crate) const MIN_SPLIT_RATIO: f32 = 0.1;
pub(crate) const MAX_SPLIT_RATIO: f32 = 0.9;
pub(crate) const NOTIFICATION_TIMEOUT_SECONDS: u64 = 5;
pub(crate) const NOTIFICATION_LIFETIME: Duration =
    Duration::from_secs(NOTIFICATION_TIMEOUT_SECONDS + 1);
pub(crate) const PANEL_BORDER_RESERVED_COLUMNS: u16 = 2;
pub(crate) const PANEL_BORDER_RESERVED_ROWS: u16 = 2;
pub(crate) const CHAT_LIST_ITEM_HEIGHT: u16 = 2;
pub(crate) const FOLDER_VIEWPORT_RESERVED_COLUMNS: u16 = 4;
pub(crate) const MESSAGE_ROW_HEIGHT: usize = 1;
pub(crate) const REPLY_MESSAGE_ROW_HEIGHT: usize = 2;
pub(crate) const TYPING_ACTION_COOLDOWN: Duration = Duration::from_secs(4);
pub(crate) const TYPING_ACTIVITY_LIFETIME: Duration = Duration::from_secs(5);
pub(crate) const MAX_REMOTE_MESSAGES: usize = 500;

fn last_index(item_count: usize) -> usize {
    item_count.saturating_sub(1)
}

#[cfg(test)]
pub(crate) fn message_visible_row_height_for_width(message: &Message, text_width: usize) -> usize {
    message_visible_row_height_for_width_capped(message, text_width, usize::MAX)
}

pub(crate) fn message_reply_preview_visible(message: &Message, max_height: usize) -> bool {
    message.reply_to_content.is_some() && max_height > 1
}

pub(crate) fn message_visible_row_height_for_width_capped(
    message: &Message,
    text_width: usize,
    max_height: usize,
) -> usize {
    if max_height == 0 {
        return 0;
    }

    let sender = format!("{}: ", message.sender_name);
    let metadata_width = message_metadata_display_width(message);
    let first_content_width = text_width
        .saturating_sub(display_width(&sender) + metadata_width)
        .max(1);
    let continuation_width = text_width.saturating_sub(display_width(&sender)).max(1);
    let display_content = message_display_content(message.media.as_ref(), &message.content);
    let mut rows = wrap_display_lines_limited(
        &display_content,
        first_content_width,
        continuation_width,
        max_height,
    )
    .len()
    .max(MESSAGE_ROW_HEIGHT);
    if message_reply_preview_visible(message, max_height) && rows < max_height {
        rows += REPLY_MESSAGE_ROW_HEIGHT - MESSAGE_ROW_HEIGHT;
    }
    rows
}

#[cfg(test)]
fn message_visible_row_height(message: &Message) -> usize {
    message_visible_row_height_for_width(message, usize::MAX / 2)
}

fn message_metadata_display_width(message: &Message) -> usize {
    let mut width = 6;
    if message.is_edited {
        width += display_width(" · edited");
    }
    if message.is_own {
        width += display_width(match message.status {
            MessageStatus::Sending => " · sending",
            MessageStatus::Sent | MessageStatus::Delivered => " · ✓",
            MessageStatus::Read => " · ✓✓",
            MessageStatus::Failed => " · failed",
        });
    }
    if let Some(error) = &message.error {
        width += display_width(" · error: ") + display_width(error);
    }
    width
}

fn chat_name_starts_with(name: &str, prefix: char) -> bool {
    let Some(first) = name.trim_start().chars().next() else {
        return false;
    };

    first.to_lowercase().to_string() == prefix.to_lowercase().to_string()
}

fn action_failed_error(prefix: &str, error: impl std::fmt::Display) -> String {
    format!("{prefix}: {error}")
}

pub(crate) fn edit_failed_error(error: impl std::fmt::Display) -> String {
    action_failed_error(EDIT_FAILED_PREFIX, error)
}

pub(crate) fn reply_failed_error(error: impl std::fmt::Display) -> String {
    action_failed_error(REPLY_FAILED_PREFIX, error)
}

pub(crate) fn send_failed_error(error: impl std::fmt::Display) -> String {
    action_failed_error(SEND_FAILED_PREFIX, error)
}

pub(crate) fn delete_failed_error(error: impl std::fmt::Display) -> String {
    action_failed_error(DELETE_FAILED_PREFIX, error)
}

pub(crate) fn mark_read_failed_error(error: impl std::fmt::Display) -> String {
    action_failed_error(MARK_READ_FAILED_PREFIX, error)
}

fn delete_update_matches_chat(update_chat_id: i64, chat_id: i64) -> bool {
    update_chat_id == UNKNOWN_DELETE_UPDATE_CHAT_ID || update_chat_id == chat_id
}

fn chat_matches_search(chat_name: &str, query: &str) -> bool {
    let query = normalize_chat_search_text(query);
    if query.is_empty() {
        return true;
    }

    let name = normalize_chat_search_text(chat_name);
    name.contains(&query) || is_subsequence(&query, &name)
}

fn normalize_chat_search_text(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|needle_char| {
        chars
            .by_ref()
            .any(|haystack_char| haystack_char == needle_char)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    Folders,
    Chats,
    Messages,
    Input,
}

impl FocusedPanel {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Folders => "Folders",
            Self::Chats => "Chats",
            Self::Messages => "Messages",
            Self::Input => "Input",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationLoadStatus {
    Idle,
    Loading,
    Loaded,
    Empty,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum OlderHistoryKey {
    Chat(i64),
    Topic { chat_id: i64, topic_id: i32 },
}

#[derive(Debug)]
struct MessageWindowAnchors {
    selected: Option<(i64, i32)>,
    viewport_top: Option<(i64, i32)>,
    protected: HashSet<(i64, i32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ConversationScope {
    chat_id: i64,
    topic_id: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ManualMarkReadOwner {
    request_id: u64,
    reconcile_after_request_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TypingActivityKey {
    chat_id: i64,
    topic_id: Option<i32>,
    sender_identity: SenderIdentity,
}

#[derive(Debug)]
struct TypingActivity {
    display_name: String,
    expires_at: tokio::time::Instant,
}

#[derive(Debug, Default)]
struct FailedSubmissionRecovery {
    submissions: BTreeMap<u64, String>,
    base: Option<String>,
}

impl FailedSubmissionRecovery {
    fn prefix(&self) -> String {
        self.submissions
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn capture_base(&mut self, current: &str, incoming: Option<&str>) {
        let prefix = self.prefix();
        self.base = Some(if prefix.is_empty() {
            if incoming == Some(current) {
                String::new()
            } else {
                current.to_string()
            }
        } else if current == prefix {
            String::new()
        } else if let Some(base) = current.strip_prefix(&format!("{prefix}\n\n")) {
            base.to_string()
        } else {
            current.to_string()
        });
    }

    fn merged(&self) -> String {
        let prefix = self.prefix();
        match self.base.as_deref().filter(|base| !base.is_empty()) {
            Some(base) if !prefix.is_empty() => format!("{prefix}\n\n{base}"),
            Some(base) => base.to_string(),
            None => prefix,
        }
    }
}

#[derive(Debug)]
struct PendingGapSubmit {
    action: MessageSubmitAction,
    request_id: u64,
    chat_id: i64,
    topic_id: Option<i32>,
    ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteConfirmation {
    pub chat_id: i64,
    pub message_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuTarget {
    Chat { chat_id: i64 },
    Message { chat_id: i64, message_id: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    OpenChat,
    MarkChatRead,
    CopyChatName,
    ReplyMessage,
    EditMessage,
    CopyMessageText,
    OpenMessageLink,
    SaveMessageMedia,
    OpenDownloadedMedia,
    DeleteMessage,
    DismissFailedSend,
}

impl ContextMenuAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenChat => "Open",
            Self::MarkChatRead => "Mark read",
            Self::CopyChatName => "Copy chat name",
            Self::ReplyMessage => "Reply",
            Self::EditMessage => "Edit",
            Self::CopyMessageText => "Copy text",
            Self::OpenMessageLink => "Open link",
            Self::SaveMessageMedia => "Save media",
            Self::OpenDownloadedMedia => "Open saved media",
            Self::DeleteMessage => "Delete",
            Self::DismissFailedSend => "Dismiss failed send",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuState {
    pub target: ContextMenuTarget,
    pub column: u16,
    pub row: u16,
    pub highlighted: usize,
    pub actions: Vec<ContextMenuAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalState {
    DeleteConfirmation(DeleteConfirmation),
    ContextMenu(ContextMenuState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageSubmitAction {
    Edit {
        chat_id: i64,
        message_id: i32,
        content: String,
    },
    Reply {
        chat_id: i64,
        thread_top_message_id: Option<i32>,
        message_id: i32,
        content: String,
    },
    Send {
        chat_id: i64,
        thread_top_message_id: Option<i32>,
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedMediaReference {
    pub chat_id: i64,
    pub message_id: i32,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconciliationContext {
    pub folder_id: Option<i32>,
    pub chat_id: Option<i64>,
    pub topic_id: Option<i32>,
    pub message_id: Option<i32>,
}

#[derive(Debug)]
pub struct ReconciliationSnapshot {
    pub folders: Vec<Folder>,
    pub selected_folder_id: Option<i32>,
    pub chats: Vec<Chat>,
    pub chat_last_message_ids: HashMap<i64, i32>,
    pub selected_chat_id: Option<i64>,
    pub thread_topics: Vec<ThreadTopic>,
    pub selected_topic_id: Option<i32>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationApply {
    Applied { conversation_replaced: bool },
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentedIncomingMessage {
    pub chat_id: i64,
    pub topic_id: Option<i32>,
    pub message_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrependMessagesResult {
    pub added: usize,
    pub had_unique_messages: bool,
}

pub struct AppState {
    pub folders: Vec<Folder>,
    pub chats: Vec<Chat>,
    pub messages: Vec<Message>,
    pub thread_topics: Vec<ThreadTopic>,
    pub selected_thread_topic_index: usize,
    pub thread_topic_scroll_offset: usize,
    pub selected_folder_index: usize,
    pub selected_chat_index: usize,
    pub chat_scroll_offset: usize,
    pub chat_search_query: Option<String>,
    pub chat_search_scroll_offset: usize,
    chat_search_selected_index: usize,
    pub selected_message_index: usize,
    pub message_scroll_offset: usize,
    pub focused_panel: FocusedPanel,
    pub input_buffer: String,
    pub input_cursor: usize,
    pub input_scroll_offset: usize,
    conversation_drafts: HashMap<ConversationScope, String>,
    cached_folder_chats: HashMap<Option<i32>, Vec<Chat>>,
    older_history_exhausted: HashSet<OlderHistoryKey>,
    newer_history_gap: bool,
    newer_history_generation: u64,
    pending_gap_submit: Option<PendingGapSubmit>,
    reply_submission_request_id: Option<u64>,
    edit_submission_request_id: Option<u64>,
    pending_mutation_scopes: HashMap<u64, ConversationScope>,
    failed_submission_recovery: HashMap<ConversationScope, FailedSubmissionRecovery>,
    pending_delete_submissions: HashMap<u64, DeleteConfirmation>,
    manual_mark_read_owners: HashMap<i64, ManualMarkReadOwner>,
    pub split_ratio: f32,
    pub split_drag_active: bool,
    split_drag_origin: Option<u16>,
    pub show_help_bar: bool,
    pub screen_area: Rect,
    pub folders_area: Rect,
    pub chats_area: Rect,
    pub messages_area: Rect,
    pub thread_topics_area: Rect,
    pub terminal_image_area: Rect,
    pub input_area: Rect,
    pub folder_scroll_offset: usize,
    pub editing_message_id: Option<i32>,
    pub replying_to_message_id: Option<i32>,
    pub error_message: Option<String>,
    pub error_timestamp: Option<tokio::time::Instant>,
    pub mutation_outcome_unknown: bool,
    pub status_message: Option<String>,
    pub status_timestamp: Option<tokio::time::Instant>,
    pub last_downloaded_media: Option<DownloadedMediaReference>,
    pub modal: Option<ModalState>,
    typing_activity: BTreeMap<TypingActivityKey, TypingActivity>,
    terminal_focused: bool,
    pub conversation_load_status: ConversationLoadStatus,
    last_typing_action_context: Option<(i64, Option<i32>)>,
    last_typing_action_at: Option<Instant>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            folders: Vec::new(),
            chats: Vec::new(),
            messages: Vec::new(),
            thread_topics: Vec::new(),
            selected_thread_topic_index: 0,
            thread_topic_scroll_offset: 0,
            selected_folder_index: 0,
            selected_chat_index: 0,
            chat_scroll_offset: 0,
            chat_search_query: None,
            chat_search_scroll_offset: 0,
            chat_search_selected_index: 0,
            selected_message_index: 0,
            message_scroll_offset: 0,
            focused_panel: FocusedPanel::Folders,
            input_buffer: String::new(),
            input_cursor: 0,
            input_scroll_offset: 0,
            conversation_drafts: HashMap::new(),
            cached_folder_chats: HashMap::new(),
            older_history_exhausted: HashSet::new(),
            newer_history_gap: false,
            newer_history_generation: 0,
            pending_gap_submit: None,
            reply_submission_request_id: None,
            edit_submission_request_id: None,
            pending_mutation_scopes: HashMap::new(),
            failed_submission_recovery: HashMap::new(),
            pending_delete_submissions: HashMap::new(),
            manual_mark_read_owners: HashMap::new(),
            split_ratio: DEFAULT_SPLIT_RATIO,
            split_drag_active: false,
            split_drag_origin: None,
            show_help_bar: true,
            screen_area: Rect::default(),
            folders_area: Rect::default(),
            chats_area: Rect::default(),
            messages_area: Rect::default(),
            thread_topics_area: Rect::default(),
            terminal_image_area: Rect::default(),
            input_area: Rect::default(),
            folder_scroll_offset: 0,
            editing_message_id: None,
            replying_to_message_id: None,
            error_message: None,
            error_timestamp: None,
            mutation_outcome_unknown: false,
            status_message: None,
            status_timestamp: None,
            last_downloaded_media: None,
            modal: None,
            typing_activity: BTreeMap::new(),
            terminal_focused: true,
            conversation_load_status: ConversationLoadStatus::Idle,
            last_typing_action_context: None,
            last_typing_action_at: None,
        }
    }

    pub fn toggle_help_bar(&mut self) {
        self.show_help_bar = !self.show_help_bar;
    }

    pub fn delete_confirmation(&self) -> Option<DeleteConfirmation> {
        match self.modal.as_ref() {
            Some(ModalState::DeleteConfirmation(confirmation)) => Some(*confirmation),
            _ => None,
        }
    }

    pub fn context_menu(&self) -> Option<&ContextMenuState> {
        match self.modal.as_ref() {
            Some(ModalState::ContextMenu(menu)) => Some(menu),
            _ => None,
        }
    }

    pub fn context_menu_mut(&mut self) -> Option<&mut ContextMenuState> {
        match self.modal.as_mut() {
            Some(ModalState::ContextMenu(menu)) => Some(menu),
            _ => None,
        }
    }

    pub fn set_delete_confirmation(&mut self, confirmation: DeleteConfirmation) {
        self.modal = Some(ModalState::DeleteConfirmation(confirmation));
    }

    pub fn close_context_menu(&mut self) {
        if matches!(self.modal.as_ref(), Some(ModalState::ContextMenu(_))) {
            self.modal = None;
        }
    }

    pub fn context_actions_for_target(&self, target: ContextMenuTarget) -> Vec<ContextMenuAction> {
        match target {
            ContextMenuTarget::Chat { chat_id } => {
                let Some(chat) = self.chats.iter().find(|chat| chat.id == chat_id) else {
                    return Vec::new();
                };
                let mut actions = vec![ContextMenuAction::OpenChat];
                if chat.unread_count > 0 && !self.manual_mark_read_pending(chat_id) {
                    actions.push(ContextMenuAction::MarkChatRead);
                }
                actions.push(ContextMenuAction::CopyChatName);
                actions
            }
            ContextMenuTarget::Message {
                chat_id,
                message_id,
            } => {
                let Some(message) = self
                    .messages
                    .iter()
                    .find(|message| message.chat_id == chat_id && message.id == message_id)
                else {
                    return Vec::new();
                };
                let actionable = Self::is_remote_actionable_message(message);
                let compose_available =
                    !self.reply_submission_pending() && !self.edit_submission_pending();
                let mut actions = Vec::new();
                if actionable && compose_available {
                    actions.push(ContextMenuAction::ReplyMessage);
                }
                if message.is_own && message.can_edit && actionable && compose_available {
                    actions.push(ContextMenuAction::EditMessage);
                }
                if !message.content.trim().is_empty() {
                    actions.push(ContextMenuAction::CopyMessageText);
                }
                if crate::links::first_url(&message.content).is_some() {
                    actions.push(ContextMenuAction::OpenMessageLink);
                }
                if message
                    .media
                    .as_ref()
                    .is_some_and(|media| media.kind.is_downloadable())
                {
                    actions.push(ContextMenuAction::SaveMessageMedia);
                }
                if self
                    .last_downloaded_media
                    .as_ref()
                    .is_some_and(|downloaded| {
                        downloaded.chat_id == chat_id && downloaded.message_id == message_id
                    })
                {
                    actions.push(ContextMenuAction::OpenDownloadedMedia);
                }
                if message.status == MessageStatus::Failed {
                    actions.push(ContextMenuAction::DismissFailedSend);
                } else if message.is_own
                    && message.can_delete
                    && actionable
                    && !self.delete_submission_pending_for(chat_id, message_id)
                {
                    actions.push(ContextMenuAction::DeleteMessage);
                }
                actions
            }
        }
    }

    pub fn open_context_menu(&mut self, target: ContextMenuTarget, column: u16, row: u16) -> bool {
        if matches!(self.modal.as_ref(), Some(ModalState::DeleteConfirmation(_))) {
            return false;
        }
        let actions = self.context_actions_for_target(target);
        if actions.is_empty() {
            return false;
        }
        self.modal = Some(ModalState::ContextMenu(ContextMenuState {
            target,
            column,
            row,
            highlighted: 0,
            actions,
        }));
        true
    }

    pub fn context_menu_rect(&self) -> Option<Rect> {
        let menu = self.context_menu()?;
        let width = menu
            .actions
            .iter()
            .map(|action| display_width(action.label()))
            .max()
            .unwrap_or(0) as u16
            + 4;
        let width = width.max(14).min(self.screen_area.width.max(1));
        let height = (menu.actions.len() as u16 + 2).min(self.screen_area.height.max(1));
        let column = menu
            .column
            .min(self.screen_area.x + self.screen_area.width.saturating_sub(width));
        let row = menu
            .row
            .min(self.screen_area.y + self.screen_area.height.saturating_sub(height));
        Some(Rect::new(column, row, width, height))
    }

    pub fn context_menu_item_at(&self, column: u16, row: u16) -> Option<usize> {
        let area = self.context_menu_rect()?;
        let inner = Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(1),
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        if !inner.contains(ratatui::layout::Position::new(column, row)) {
            return None;
        }
        let index = row.saturating_sub(inner.y) as usize;
        (index < self.context_menu()?.actions.len()).then_some(index)
    }

    pub fn hover_context_menu_at(&mut self, column: u16, row: u16) {
        let hovered = self.context_menu_item_at(column, row);
        if let (Some(index), Some(menu)) = (hovered, self.context_menu_mut()) {
            menu.highlighted = index;
        }
    }

    pub fn move_context_menu_highlight(&mut self, delta: isize) {
        let Some(menu) = self.context_menu_mut() else {
            return;
        };
        let last = menu.actions.len().saturating_sub(1);
        menu.highlighted = if delta < 0 {
            menu.highlighted.saturating_sub(delta.unsigned_abs())
        } else {
            menu.highlighted.saturating_add(delta as usize).min(last)
        };
    }

    pub fn take_context_menu_action(
        &mut self,
        index: usize,
    ) -> Option<(ContextMenuTarget, ContextMenuAction)> {
        let menu = self.context_menu()?;
        let result = Some((menu.target, *menu.actions.get(index)?));
        self.modal = None;
        result
    }

    pub fn take_highlighted_context_menu_action(
        &mut self,
    ) -> Option<(ContextMenuTarget, ContextMenuAction)> {
        let highlighted = self.context_menu()?.highlighted;
        self.take_context_menu_action(highlighted)
    }

    pub fn has_message_identity(&self, chat_id: i64, message_id: i32) -> bool {
        self.messages
            .iter()
            .any(|message| message.chat_id == chat_id && message.id == message_id)
    }

    pub fn select_message_by_identity(&mut self, chat_id: i64, message_id: i32) -> bool {
        if self.selected_chat_id() != Some(chat_id) {
            return false;
        }
        let Some(index) = self
            .messages
            .iter()
            .position(|message| message.chat_id == chat_id && message.id == message_id)
        else {
            return false;
        };
        self.selected_message_index = index;
        self.ensure_selected_message_visible();
        true
    }

    pub fn begin_delete_submission(
        &mut self,
        submission_id: u64,
        confirmation: DeleteConfirmation,
    ) -> bool {
        if self.delete_submission_pending_for(confirmation.chat_id, confirmation.message_id) {
            return false;
        }
        self.pending_delete_submissions
            .insert(submission_id, confirmation);
        true
    }

    pub fn delete_submission_pending_for(&self, chat_id: i64, message_id: i32) -> bool {
        self.pending_delete_submissions
            .values()
            .any(|confirmation| {
                confirmation.chat_id == chat_id && confirmation.message_id == message_id
            })
    }

    pub fn finish_delete_submission(
        &mut self,
        submission_id: u64,
        confirmation: DeleteConfirmation,
    ) -> bool {
        if self.pending_delete_submissions.get(&submission_id) != Some(&confirmation) {
            return false;
        }
        self.pending_delete_submissions.remove(&submission_id);
        true
    }

    pub fn finish_delete_submissions_for_update(&mut self, chat_id: i64, message_id: i32) {
        self.pending_delete_submissions.retain(|_, confirmation| {
            confirmation.message_id != message_id
                || !delete_update_matches_chat(chat_id, confirmation.chat_id)
        });
    }

    pub fn begin_manual_mark_read(&mut self, chat_id: i64, request_id: u64) -> bool {
        if self.manual_mark_read_pending(chat_id) {
            return false;
        }
        self.manual_mark_read_owners.insert(
            chat_id,
            ManualMarkReadOwner {
                request_id,
                reconcile_after_request_id: None,
            },
        );
        true
    }

    pub fn manual_mark_read_pending(&self, chat_id: i64) -> bool {
        self.manual_mark_read_owners.contains_key(&chat_id)
    }

    pub fn accept_manual_mark_read_success(
        &mut self,
        chat_id: i64,
        request_id: u64,
        reconcile_after_request_id: u64,
    ) -> bool {
        let Some(owner) = self.manual_mark_read_owners.get_mut(&chat_id) else {
            return false;
        };
        if owner.request_id != request_id || owner.reconcile_after_request_id.is_some() {
            return false;
        }
        owner.reconcile_after_request_id = Some(reconcile_after_request_id);
        true
    }

    pub fn finish_manual_mark_read_failure(&mut self, chat_id: i64, request_id: u64) -> bool {
        let matches = self
            .manual_mark_read_owners
            .get(&chat_id)
            .is_some_and(|owner| {
                owner.request_id == request_id && owner.reconcile_after_request_id.is_none()
            });
        if matches {
            self.manual_mark_read_owners.remove(&chat_id);
        }
        matches
    }

    pub fn finish_manual_mark_read_snapshot(
        &mut self,
        chat_id: i64,
        reconciliation_request_id: u64,
    ) -> bool {
        let matches = self
            .manual_mark_read_owners
            .get(&chat_id)
            .is_some_and(|owner| {
                owner
                    .reconcile_after_request_id
                    .is_some_and(|required| reconciliation_request_id >= required)
            });
        if matches {
            self.manual_mark_read_owners.remove(&chat_id);
        }
        matches
    }

    pub fn scroll_chats(&mut self, delta: isize) {
        let item_count = self.chat_display_indices().len();
        let max_offset = item_count.saturating_sub(self.chat_visible_capacity());
        let offset = if self.chat_search_active() {
            &mut self.chat_search_scroll_offset
        } else {
            &mut self.chat_scroll_offset
        };
        *offset = if delta < 0 {
            offset.saturating_sub(delta.unsigned_abs())
        } else {
            offset.saturating_add(delta as usize).min(max_offset)
        };
    }

    pub fn split_divider_contains(&self, column: u16, row: u16) -> bool {
        let left_border = self.chats_area.x + self.chats_area.width.saturating_sub(1);
        let right_border = self.messages_area.x;
        row >= self.chats_area.y
            && row < self.chats_area.y + self.chats_area.height
            && (column == left_border || column == right_border)
    }

    pub fn begin_split_drag(&mut self, column: u16) {
        self.split_drag_active = true;
        self.split_drag_origin = Some(column);
    }

    pub fn drag_split_to(&mut self, column: u16) {
        if self
            .split_drag_origin
            .is_none_or(|origin| origin.abs_diff(column) < SPLIT_DRAG_THRESHOLD_COLUMNS)
        {
            return;
        }
        let width = self.screen_area.width.max(1);
        let ratio = column.saturating_sub(self.screen_area.x) as f32 / width as f32;
        self.split_ratio = ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
    }

    pub fn end_split_drag(&mut self) {
        self.split_drag_active = false;
        self.split_drag_origin = None;
    }

    pub fn set_terminal_focused(&mut self, focused: bool) {
        self.terminal_focused = focused;
    }

    pub fn terminal_focused(&self) -> bool {
        self.terminal_focused
    }

    fn folder_label_display_width(folder: &Folder) -> usize {
        let unread_width = if folder.unread_count > 0 {
            display_width(&format!(" ({})", folder.unread_count))
        } else {
            0
        };

        display_width(&folder.name) + unread_width + display_width(FOLDER_LABEL_HORIZONTAL_PADDING)
    }

    fn thread_topic_label_display_width(topic: &ThreadTopic) -> usize {
        let unread_width = if topic.unread_count > 0 {
            display_width(&format!(" ({})", topic.unread_count))
        } else {
            0
        };

        display_width(&topic.title)
            + unread_width
            + display_width(THREAD_TOPIC_LABEL_HORIZONTAL_PADDING)
    }

    fn visible_folder_count_from_offset(&self, offset: usize) -> usize {
        if self.folders.is_empty() || offset >= self.folders.len() {
            return 0;
        }

        let available_width =
            self.folders_area
                .width
                .saturating_sub(FOLDER_VIEWPORT_RESERVED_COLUMNS) as usize;
        let mut visible_count = 0usize;
        let mut current_width = 0usize;

        if offset > 0 {
            current_width += display_width(FOLDER_LEFT_SCROLL_INDICATOR);
        }

        for (relative_idx, folder) in self.folders.iter().skip(offset).enumerate() {
            let separator_width = if visible_count == 0 {
                0
            } else {
                display_width(FOLDER_SEPARATOR)
            };
            let right_indicator_width = if offset + relative_idx + 1 < self.folders.len() {
                display_width(FOLDER_RIGHT_SCROLL_INDICATOR)
            } else {
                0
            };
            let folder_width = Self::folder_label_display_width(folder);

            if current_width + separator_width + folder_width + right_indicator_width
                > available_width
            {
                return visible_count;
            }
            visible_count += 1;
            current_width += separator_width + folder_width;
        }

        visible_count
    }

    pub fn get_visible_folders(&self) -> (Vec<&Folder>, bool, bool) {
        let visible_count = self.visible_folder_count_from_offset(self.folder_scroll_offset);
        if visible_count == 0 {
            return (Vec::new(), self.folder_scroll_offset > 0, false);
        }

        let visible_folders = self
            .folders
            .iter()
            .skip(self.folder_scroll_offset)
            .take(visible_count)
            .collect::<Vec<_>>();
        let has_left_scroll = self.folder_scroll_offset > 0;
        let has_right_scroll = self.folder_scroll_offset + visible_count < self.folders.len();
        (visible_folders, has_left_scroll, has_right_scroll)
    }

    pub fn folder_index_at_visible_column(&self, column: usize) -> Option<usize> {
        let (visible_folders, has_left_scroll, _) = self.get_visible_folders();
        if visible_folders.is_empty() {
            return None;
        }

        let mut current_column = 0usize;
        if has_left_scroll {
            let indicator_width = display_width(FOLDER_LEFT_SCROLL_INDICATOR);
            if column < current_column + indicator_width {
                return None;
            }
            current_column += indicator_width;
        }

        for (idx, folder) in visible_folders.iter().enumerate() {
            if idx > 0 {
                let separator_width = display_width(FOLDER_SEPARATOR);
                if column < current_column + separator_width {
                    return None;
                }
                current_column += separator_width;
            }

            let folder_width = Self::folder_label_display_width(folder);
            if column < current_column + folder_width {
                return Some(self.folder_scroll_offset + idx);
            }
            current_column += folder_width;
        }

        None
    }

    fn visible_thread_topic_count_from_offset(&self, offset: usize) -> usize {
        if self.thread_topics.is_empty() || offset >= self.thread_topics.len() {
            return 0;
        }

        let available_width =
            self.thread_topics_area
                .width
                .saturating_sub(FOLDER_VIEWPORT_RESERVED_COLUMNS) as usize;
        let mut visible_count = 0usize;
        let mut current_width = 0usize;

        if offset > 0 {
            current_width += display_width(FOLDER_LEFT_SCROLL_INDICATOR);
        }

        for (relative_idx, topic) in self.thread_topics.iter().skip(offset).enumerate() {
            let separator_width = if visible_count == 0 {
                0
            } else {
                display_width(FOLDER_SEPARATOR)
            };
            let right_indicator_width = if offset + relative_idx + 1 < self.thread_topics.len() {
                display_width(FOLDER_RIGHT_SCROLL_INDICATOR)
            } else {
                0
            };
            let topic_width = Self::thread_topic_label_display_width(topic);

            if current_width + separator_width + topic_width + right_indicator_width
                > available_width
            {
                return visible_count;
            }
            visible_count += 1;
            current_width += separator_width + topic_width;
        }

        visible_count
    }

    pub fn get_visible_thread_topics(&self) -> (Vec<&ThreadTopic>, bool, bool) {
        let visible_count =
            self.visible_thread_topic_count_from_offset(self.thread_topic_scroll_offset);
        if visible_count == 0 {
            return (Vec::new(), self.thread_topic_scroll_offset > 0, false);
        }

        let visible_topics = self
            .thread_topics
            .iter()
            .skip(self.thread_topic_scroll_offset)
            .take(visible_count)
            .collect::<Vec<_>>();
        let has_left_scroll = self.thread_topic_scroll_offset > 0;
        let has_right_scroll =
            self.thread_topic_scroll_offset + visible_count < self.thread_topics.len();
        (visible_topics, has_left_scroll, has_right_scroll)
    }

    pub fn thread_topic_index_at_visible_column(&self, column: usize) -> Option<usize> {
        let (visible_topics, has_left_scroll, _) = self.get_visible_thread_topics();
        if visible_topics.is_empty() {
            return None;
        }

        let mut current_column = 0usize;
        if has_left_scroll {
            let indicator_width = display_width(FOLDER_LEFT_SCROLL_INDICATOR);
            if column < current_column + indicator_width {
                return None;
            }
            current_column += indicator_width;
        }

        for (idx, topic) in visible_topics.iter().enumerate() {
            if idx > 0 {
                let separator_width = display_width(FOLDER_SEPARATOR);
                if column < current_column + separator_width {
                    return None;
                }
                current_column += separator_width;
            }

            let topic_width = Self::thread_topic_label_display_width(topic);
            if column < current_column + topic_width {
                return Some(self.thread_topic_scroll_offset + idx);
            }
            current_column += topic_width;
        }

        None
    }

    pub fn ensure_selected_thread_topic_visible(&mut self) {
        if self.thread_topics.is_empty() {
            self.selected_thread_topic_index = 0;
            self.thread_topic_scroll_offset = 0;
            return;
        }

        self.selected_thread_topic_index = self
            .selected_thread_topic_index
            .min(last_index(self.thread_topics.len()));
        self.thread_topic_scroll_offset = self
            .thread_topic_scroll_offset
            .min(last_index(self.thread_topics.len()));

        if self.selected_thread_topic_index < self.thread_topic_scroll_offset {
            self.thread_topic_scroll_offset = self.selected_thread_topic_index;
        }

        while self.thread_topic_scroll_offset > 0 {
            let candidate = self.thread_topic_scroll_offset - 1;
            let visible_count = self.visible_thread_topic_count_from_offset(candidate);
            let max_visible_index = candidate + last_index(visible_count);
            if visible_count > 0 && self.selected_thread_topic_index <= max_visible_index {
                self.thread_topic_scroll_offset = candidate;
            } else {
                break;
            }
        }

        let visible_count =
            self.visible_thread_topic_count_from_offset(self.thread_topic_scroll_offset);
        let max_visible_index = self.thread_topic_scroll_offset + last_index(visible_count);
        if visible_count == 0 || self.selected_thread_topic_index > max_visible_index {
            self.thread_topic_scroll_offset = self
                .selected_thread_topic_index
                .saturating_sub(last_index(visible_count));
        }
    }

    fn switch_thread_topic_to(&mut self, index: usize) {
        if self.thread_topics.is_empty() {
            self.selected_thread_topic_index = 0;
            self.thread_topic_scroll_offset = 0;
            return;
        }
        let index = index.min(last_index(self.thread_topics.len()));
        if index == self.selected_thread_topic_index {
            self.ensure_selected_thread_topic_visible();
            return;
        }
        if self.editing_message_id.is_some() || self.replying_to_message_id.is_some() {
            self.cancel_compose_mode();
        }
        self.save_current_draft();
        self.selected_thread_topic_index = index;
        self.ensure_selected_thread_topic_visible();
        self.restore_draft_for_selected_chat();
    }

    pub fn select_thread_topic_at(&mut self, index: usize) {
        self.switch_thread_topic_to(index);
    }

    pub fn ensure_selected_folder_visible(&mut self) {
        if self.folders.is_empty() {
            self.selected_folder_index = 0;
            self.folder_scroll_offset = 0;
            return;
        }

        self.selected_folder_index = self
            .selected_folder_index
            .min(last_index(self.folders.len()));
        self.folder_scroll_offset = self
            .folder_scroll_offset
            .min(last_index(self.folders.len()));

        if self.selected_folder_index < self.folder_scroll_offset {
            self.folder_scroll_offset = self.selected_folder_index;
        }

        while self.folder_scroll_offset > 0 {
            let candidate = self.folder_scroll_offset - 1;
            let visible_count = self.visible_folder_count_from_offset(candidate);
            let max_visible_index = candidate + last_index(visible_count);
            if visible_count > 0 && self.selected_folder_index <= max_visible_index {
                self.folder_scroll_offset = candidate;
            } else {
                break;
            }
        }

        let visible_count = self.visible_folder_count_from_offset(self.folder_scroll_offset);
        let max_visible_index = self.folder_scroll_offset + last_index(visible_count);
        if visible_count == 0 || self.selected_folder_index > max_visible_index {
            self.folder_scroll_offset = self
                .selected_folder_index
                .saturating_sub(last_index(visible_count));
        }
    }

    pub fn select_folder(&mut self, index: usize) {
        if index < self.folders.len() {
            self.selected_folder_index = index;
            self.ensure_selected_folder_visible();
        }
    }

    pub fn select_chat(&mut self, index: usize) {
        if index < self.chats.len() {
            self.selected_chat_index = index;
            self.ensure_selected_chat_visible();
        }
    }

    pub fn chat_search_active(&self) -> bool {
        self.chat_search_query.is_some()
    }

    pub fn chat_search_query(&self) -> &str {
        self.chat_search_query.as_deref().unwrap_or("")
    }

    pub fn begin_chat_search(&mut self) {
        self.chat_search_query = Some(String::new());
        self.chat_search_scroll_offset = 0;
        self.chat_search_selected_index =
            self.selected_chat_index.min(last_index(self.chats.len()));
        self.ensure_selected_chat_search_visible();
    }

    pub fn clear_chat_search(&mut self) {
        self.chat_search_query = None;
        self.chat_search_scroll_offset = 0;
        self.chat_search_selected_index = 0;
        self.ensure_selected_chat_visible();
    }

    pub fn push_chat_search_char(&mut self, ch: char) {
        if !self.chat_search_active() || ch.is_control() {
            return;
        }
        if let Some(query) = self.chat_search_query.as_mut() {
            query.push(ch);
        }
        self.select_first_chat_search_match();
    }

    pub fn pop_chat_search_char(&mut self) {
        if let Some(query) = self.chat_search_query.as_mut() {
            query.pop();
        }
        self.select_first_chat_search_match();
    }

    pub fn chat_display_indices(&self) -> Vec<usize> {
        if !self.chat_search_active() {
            return (0..self.chats.len()).collect();
        }

        self.chats
            .iter()
            .enumerate()
            .filter_map(|(index, chat)| {
                chat_matches_search(&chat.name, self.chat_search_query()).then_some(index)
            })
            .collect()
    }

    pub fn selected_chat_display_index(&self) -> Option<usize> {
        if self.chat_search_active() {
            let match_count = self.chat_display_indices().len();
            return (match_count > 0)
                .then_some(self.chat_search_selected_index.min(last_index(match_count)));
        }
        self.chats
            .get(self.selected_chat_index)
            .map(|_| self.selected_chat_index)
    }

    pub fn selected_chat_search_result_index(&self) -> Option<usize> {
        if !self.chat_search_active() {
            return None;
        }
        let indices = self.chat_display_indices();
        indices
            .get(
                self.chat_search_selected_index
                    .min(last_index(indices.len())),
            )
            .copied()
    }

    fn select_first_chat_search_match(&mut self) {
        self.chat_search_selected_index = 0;
        self.ensure_selected_chat_search_visible();
    }

    pub fn ensure_selected_chat_search_visible(&mut self) {
        if !self.chat_search_active() {
            self.ensure_selected_chat_visible();
            return;
        }

        let indices = self.chat_display_indices();
        if indices.is_empty() {
            self.chat_search_scroll_offset = 0;
            return;
        }

        let capacity = self.chat_visible_capacity();
        let max_scroll_offset = indices.len().saturating_sub(capacity);
        self.chat_search_selected_index = self
            .chat_search_selected_index
            .min(last_index(indices.len()));
        self.chat_search_scroll_offset = self.chat_search_scroll_offset.min(max_scroll_offset);
        if self.chat_search_selected_index < self.chat_search_scroll_offset {
            self.chat_search_scroll_offset = self.chat_search_selected_index;
        } else if self.chat_search_selected_index >= self.chat_search_scroll_offset + capacity {
            self.chat_search_scroll_offset = self.chat_search_selected_index + 1 - capacity;
        }
    }

    pub fn select_next_chat_search_match(&mut self) {
        let indices = self.chat_display_indices();
        if indices.is_empty() {
            return;
        }
        self.chat_search_selected_index = self
            .chat_search_selected_index
            .saturating_add(1)
            .min(indices.len() - 1);
        self.ensure_selected_chat_search_visible();
    }

    pub fn select_previous_chat_search_match(&mut self) {
        let indices = self.chat_display_indices();
        if indices.is_empty() {
            return;
        }
        self.chat_search_selected_index = self.chat_search_selected_index.saturating_sub(1);
        self.ensure_selected_chat_search_visible();
    }

    pub fn next_chat_index_starting_with(&self, prefix: char) -> Option<usize> {
        if self.chats.len() <= 1 || !prefix.is_alphanumeric() {
            return None;
        }

        let start = self.selected_chat_index.saturating_add(1);
        (start..self.chats.len())
            .chain(0..self.selected_chat_index.min(self.chats.len()))
            .find(|&index| chat_name_starts_with(&self.chats[index].name, prefix))
    }

    pub fn selected_chat_id(&self) -> Option<i64> {
        self.chats.get(self.selected_chat_index).map(|chat| chat.id)
    }

    pub fn selected_folder_filter_id(&self) -> Option<i32> {
        self.folders
            .get(self.selected_folder_index)
            .and_then(|folder| (!is_all_folder(folder)).then_some(folder.id))
    }

    fn folder_cache_key_at(&self, folder_index: usize) -> Option<Option<i32>> {
        self.folders
            .get(folder_index)
            .map(|folder| (!is_all_folder(folder)).then_some(folder.id))
    }

    pub fn cache_folder_chats_at(&mut self, folder_index: usize) {
        let Some(folder_id) = self.folder_cache_key_at(folder_index) else {
            return;
        };
        self.cached_folder_chats
            .insert(folder_id, self.chats.clone());
    }

    pub fn cache_selected_folder_chats(&mut self) {
        self.cache_folder_chats_at(self.selected_folder_index);
    }

    pub fn restore_cached_folder_chats(&mut self, folder_id: Option<i32>) -> bool {
        let Some(chats) = self.cached_folder_chats.get(&folder_id).cloned() else {
            return false;
        };
        self.chats = chats;
        self.reset_chat_selection();
        true
    }

    pub fn apply_loaded_selected_chat_messages(&mut self, messages: Vec<Message>) {
        self.apply_selected_chat_message_replacement(messages, false, true);
    }

    pub fn apply_refreshed_selected_chat_messages(&mut self, messages: Vec<Message>) {
        self.apply_selected_chat_message_replacement(messages, true, false);
    }

    fn apply_selected_chat_message_replacement(
        &mut self,
        messages: Vec<Message>,
        preserve_current_input: bool,
        preserve_newer_remote_rows: bool,
    ) {
        let current_input = preserve_current_input.then(|| {
            (
                self.input_buffer.clone(),
                self.input_cursor,
                self.input_scroll_offset,
            )
        });
        self.clear_selected_scope_older_history_exhausted();
        let selected_chat_id = self.selected_chat_id();
        let selected_topic_id = self.selected_thread_topic().map(|topic| topic.id);
        let local_rows = self
            .messages
            .iter()
            .filter(|message| {
                matches!(
                    message.status,
                    MessageStatus::Sending | MessageStatus::Failed
                ) && Some(message.chat_id) == selected_chat_id
                    && message.thread_topic_id == selected_topic_id
            })
            .cloned()
            .collect::<Vec<_>>();
        let fetched_max_message_id = messages.iter().map(|message| message.id).max();
        let newer_remote_rows = if preserve_newer_remote_rows {
            self.messages
                .iter()
                .filter(|message| {
                    Self::is_remote_message(message)
                        && Some(message.chat_id) == selected_chat_id
                        && message.thread_topic_id == selected_topic_id
                        && fetched_max_message_id.is_none_or(|max_id| message.id > max_id)
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.messages = messages;
        for message in newer_remote_rows {
            if !self
                .messages
                .iter()
                .any(|existing| existing.chat_id == message.chat_id && existing.id == message.id)
            {
                self.insert_remote_message_ordered(message);
            }
        }
        self.retain_newest_remote_messages();
        for local in local_rows {
            if !self.messages.iter().any(|message| message.id == local.id) {
                self.messages.push(local);
            }
        }
        self.newer_history_gap = false;
        self.newer_history_generation = self.newer_history_generation.saturating_add(1);
        if self.messages.is_empty() {
            self.conversation_load_status = ConversationLoadStatus::Empty;
            self.reset_message_selection();
        } else {
            self.conversation_load_status = ConversationLoadStatus::Loaded;
            self.select_last_message();
        }
        if self.terminal_focused {
            self.clear_selected_conversation_unread(selected_topic_id);
        }
        if selected_topic_id.is_none() {
            self.refresh_selected_chat_last_message_from_loaded_messages();
        }
        self.revalidate_reconciled_targets();
        self.restore_draft_for_selected_chat();
        if let Some((input_buffer, input_cursor, input_scroll_offset)) = current_input {
            self.input_buffer = input_buffer;
            self.input_cursor = input_cursor.min(self.input_buffer.len());
            self.input_scroll_offset = input_scroll_offset;
            self.save_current_draft();
        }
    }

    fn selected_older_history_key(&self) -> Option<OlderHistoryKey> {
        let chat_id = self.selected_chat_id()?;
        Some(
            self.selected_thread_topic()
                .map(|topic| OlderHistoryKey::Topic {
                    chat_id,
                    topic_id: topic.id,
                })
                .unwrap_or(OlderHistoryKey::Chat(chat_id)),
        )
    }

    pub fn selected_chat_older_history_exhausted(&self) -> bool {
        self.selected_older_history_key()
            .is_some_and(|key| self.older_history_exhausted.contains(&key))
    }

    pub fn mark_selected_chat_older_history_exhausted(&mut self) {
        if let Some(key) = self.selected_older_history_key() {
            self.older_history_exhausted.insert(key);
        }
    }

    fn clear_selected_scope_older_history_exhausted(&mut self) {
        if let Some(key) = self.selected_older_history_key() {
            self.older_history_exhausted.remove(&key);
        }
    }

    pub fn newer_history_gap(&self) -> bool {
        self.newer_history_gap
    }

    pub fn newer_history_generation(&self) -> u64 {
        self.newer_history_generation
    }

    pub fn queue_gap_submit(
        &mut self,
        action: MessageSubmitAction,
        request_id: u64,
        chat_id: i64,
        topic_id: Option<i32>,
    ) {
        self.pending_gap_submit = Some(PendingGapSubmit {
            action,
            request_id,
            chat_id,
            topic_id,
            ready: false,
        });
    }

    pub fn gap_submit_pending(&self) -> bool {
        self.pending_gap_submit.is_some()
    }

    pub fn cancel_gap_submit(&mut self) -> bool {
        self.pending_gap_submit.take().is_some()
    }

    pub fn reply_submission_pending(&self) -> bool {
        self.reply_submission_request_id.is_some()
    }

    pub fn begin_reply_submission(&mut self, request_id: u64) -> bool {
        if self.reply_submission_request_id.is_some() {
            false
        } else {
            self.reply_submission_request_id = Some(request_id);
            true
        }
    }

    pub fn reply_submission_matches(&self, request_id: u64) -> bool {
        self.reply_submission_request_id == Some(request_id)
    }

    pub fn finish_reply_submission(&mut self) {
        self.reply_submission_request_id = None;
    }

    pub fn begin_edit_submission(&mut self, request_id: u64) {
        self.edit_submission_request_id = Some(request_id);
    }

    pub fn edit_submission_pending(&self) -> bool {
        self.edit_submission_request_id.is_some()
    }

    pub fn edit_submission_matches(&self, request_id: u64) -> bool {
        self.edit_submission_request_id == Some(request_id)
    }

    pub fn finish_edit_submission(&mut self) {
        self.edit_submission_request_id = None;
    }

    fn selected_conversation_scope(&self) -> Option<ConversationScope> {
        Some(ConversationScope {
            chat_id: self.selected_chat_id()?,
            topic_id: self.selected_thread_topic().map(|topic| topic.id),
        })
    }

    pub fn register_mutation_submission(
        &mut self,
        submission_id: u64,
        chat_id: i64,
        topic_id: Option<i32>,
    ) {
        self.pending_mutation_scopes
            .insert(submission_id, ConversationScope { chat_id, topic_id });
    }

    pub fn finish_mutation_submission(&mut self, submission_id: u64) {
        self.pending_mutation_scopes.remove(&submission_id);
    }

    pub fn recover_failed_submission(
        &mut self,
        submission_id: u64,
        chat_id: i64,
        topic_id: Option<i32>,
        content: String,
    ) {
        self.record_failed_submission(submission_id, chat_id, topic_id, content, true);
    }

    pub fn record_failed_submission(
        &mut self,
        submission_id: u64,
        chat_id: i64,
        topic_id: Option<i32>,
        content: String,
        reveal_if_selected: bool,
    ) {
        let reported_scope = ConversationScope { chat_id, topic_id };
        let scope = self
            .pending_mutation_scopes
            .get(&submission_id)
            .copied()
            .unwrap_or(reported_scope);
        let selected = reveal_if_selected && self.selected_conversation_scope() == Some(scope);
        let current_input = selected.then(|| self.input_buffer.clone());
        let recovery = self.failed_submission_recovery.entry(scope).or_default();
        if let Some(current_input) = current_input.as_deref() {
            recovery.capture_base(current_input, Some(&content));
        }
        let equal_submission_id = recovery
            .submissions
            .iter()
            .find_map(|(&id, existing)| (existing == &content).then_some(id));
        match equal_submission_id {
            Some(existing_id) if submission_id < existing_id => {
                recovery.submissions.remove(&existing_id);
                recovery.submissions.insert(submission_id, content);
            }
            Some(_) => {}
            None => {
                recovery.submissions.insert(submission_id, content);
            }
        }
        if selected {
            self.input_buffer = recovery.merged();
            self.set_input_cursor_to_end();
            self.focused_panel = FocusedPanel::Input;
        }
        self.pending_mutation_scopes.remove(&submission_id);
    }

    #[cfg(test)]
    fn pending_mutation_submission_count(&self) -> usize {
        self.pending_mutation_scopes.len()
    }

    #[cfg(test)]
    fn conversation_draft(&self, chat_id: i64, topic_id: Option<i32>) -> Option<&str> {
        self.conversation_drafts
            .get(&ConversationScope { chat_id, topic_id })
            .map(String::as_str)
    }

    #[cfg(test)]
    fn insert_conversation_draft(
        &mut self,
        chat_id: i64,
        topic_id: Option<i32>,
        draft: impl Into<String>,
    ) {
        self.conversation_drafts
            .insert(ConversationScope { chat_id, topic_id }, draft.into());
    }

    pub fn mark_gap_submit_ready(&mut self, request_id: u64, chat_id: i64, topic_id: Option<i32>) {
        if let Some(pending) = self.pending_gap_submit.as_mut()
            && pending.request_id == request_id
            && pending.chat_id == chat_id
            && pending.topic_id == topic_id
        {
            pending.ready = true;
        }
    }

    pub fn take_ready_gap_submit(&mut self) -> Option<MessageSubmitAction> {
        self.pending_gap_submit
            .as_ref()
            .is_some_and(|pending| pending.ready)
            .then(|| self.pending_gap_submit.take().map(|pending| pending.action))
            .flatten()
    }

    pub fn cancel_gap_submit_for_request(&mut self, request_id: u64) {
        if self
            .pending_gap_submit
            .as_ref()
            .is_some_and(|pending| pending.request_id == request_id)
        {
            self.pending_gap_submit = None;
        }
    }

    fn record_newer_history_gap(&mut self) {
        self.newer_history_gap = true;
        self.newer_history_generation = self.newer_history_generation.saturating_add(1);
    }

    fn is_remote_message(message: &Message) -> bool {
        !matches!(
            message.status,
            MessageStatus::Sending | MessageStatus::Failed
        )
    }

    fn message_identity(message: &Message) -> (i64, i32) {
        (message.chat_id, message.id)
    }

    fn capture_message_window_anchors(&self) -> MessageWindowAnchors {
        let selected = self.selected_message().map(Self::message_identity);
        let viewport_top = self
            .messages
            .get(self.message_scroll_offset)
            .map(Self::message_identity);
        let mut protected = [selected, viewport_top]
            .into_iter()
            .flatten()
            .collect::<HashSet<_>>();
        if let Some(message_id) = self.editing_message_id.or(self.replying_to_message_id)
            && let Some(chat_id) = self.selected_chat_id()
        {
            protected.insert((chat_id, message_id));
        }
        if let Some(confirmation) = self.delete_confirmation() {
            protected.insert((confirmation.chat_id, confirmation.message_id));
        }
        if let Some(ContextMenuTarget::Message {
            chat_id,
            message_id,
        }) = self.context_menu().map(|menu| menu.target)
        {
            protected.insert((chat_id, message_id));
        }
        MessageWindowAnchors {
            selected,
            viewport_top,
            protected,
        }
    }

    fn restore_message_window_anchors(&mut self, anchors: &MessageWindowAnchors) {
        self.selected_message_index = anchors
            .selected
            .and_then(|identity| {
                self.messages
                    .iter()
                    .position(|message| Self::message_identity(message) == identity)
            })
            .unwrap_or_else(|| {
                self.selected_message_index
                    .min(last_index(self.messages.len()))
            });
        self.message_scroll_offset = anchors
            .viewport_top
            .and_then(|identity| {
                self.messages
                    .iter()
                    .position(|message| Self::message_identity(message) == identity)
            })
            .unwrap_or_else(|| {
                self.message_scroll_offset
                    .min(last_index(self.messages.len()))
            });
        if self.messages.is_empty() {
            self.reset_message_selection();
        }
    }

    fn remove_message_indices(&mut self, indices: &HashSet<usize>) {
        let mut index = 0;
        self.messages.retain(|_| {
            let keep = !indices.contains(&index);
            index += 1;
            keep
        });
    }

    fn remote_message_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| Self::is_remote_message(message))
            .count()
    }

    fn retain_newest_remote_messages(&mut self) {
        let excess = self
            .remote_message_count()
            .saturating_sub(MAX_REMOTE_MESSAGES);
        if excess == 0 {
            return;
        }
        let indices = self
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| Self::is_remote_message(message).then_some(index))
            .take(excess)
            .collect::<HashSet<_>>();
        self.remove_message_indices(&indices);
    }

    fn compact_remote_tail_growth(
        &mut self,
        anchors: &MessageWindowAnchors,
        newest_identity: (i64, i32),
    ) -> bool {
        let excess = self
            .remote_message_count()
            .saturating_sub(MAX_REMOTE_MESSAGES);
        if excess == 0 {
            return true;
        }

        let protected_start = anchors
            .protected
            .iter()
            .filter_map(|identity| {
                self.messages
                    .iter()
                    .position(|message| Self::message_identity(message) == *identity)
            })
            .min()
            .unwrap_or(last_index(self.messages.len()));
        let removable = self
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                (index < protected_start
                    && Self::is_remote_message(message)
                    && !anchors.protected.contains(&Self::message_identity(message)))
                .then_some(index)
            })
            .take(excess)
            .collect::<HashSet<_>>();
        if removable.len() < excess {
            if let Some(index) = self
                .messages
                .iter()
                .rposition(|message| Self::message_identity(message) == newest_identity)
            {
                self.messages.remove(index);
            }
            self.record_newer_history_gap();
            self.restore_message_window_anchors(anchors);
            return false;
        }

        self.remove_message_indices(&removable);
        self.clear_selected_scope_older_history_exhausted();
        self.restore_message_window_anchors(anchors);
        true
    }

    fn insert_remote_message_ordered(&mut self, message: Message) -> usize {
        let insert_at = self
            .messages
            .iter()
            .position(|existing| !Self::is_remote_message(existing) || existing.id > message.id)
            .unwrap_or(self.messages.len());
        self.messages.insert(insert_at, message);
        insert_at
    }

    fn replace_anchor_identity(
        anchors: &mut MessageWindowAnchors,
        previous: (i64, i32),
        replacement: (i64, i32),
    ) {
        if anchors.selected == Some(previous) {
            anchors.selected = Some(replacement);
        }
        if anchors.viewport_top == Some(previous) {
            anchors.viewport_top = Some(replacement);
        }
        if anchors.protected.remove(&previous) {
            anchors.protected.insert(replacement);
        }
    }

    fn upsert_confirmed_message(
        &mut self,
        message: Message,
        replace_identity: Option<(i64, i32)>,
    ) -> bool {
        let identity = Self::message_identity(&message);
        let mut anchors = self.capture_message_window_anchors();
        if let Some(previous) = replace_identity {
            Self::replace_anchor_identity(&mut anchors, previous, identity);
        }
        self.messages.retain(|existing| {
            let existing_identity = Self::message_identity(existing);
            existing_identity != identity && Some(existing_identity) != replace_identity
        });
        self.insert_remote_message_ordered(message);
        self.restore_message_window_anchors(&anchors);
        let retained = self.compact_remote_tail_growth(&anchors, identity);
        self.restore_message_window_anchors(&anchors);
        self.ensure_selected_message_visible();
        retained
    }

    fn append_remote_message_with_retention(&mut self, message: Message) -> bool {
        if self.newer_history_gap {
            self.record_newer_history_gap();
            return false;
        }
        let identity = Self::message_identity(&message);
        let anchors = self.capture_message_window_anchors();
        self.messages.push(message);
        self.compact_remote_tail_growth(&anchors, identity)
    }

    pub fn prepend_loaded_selected_chat_messages(
        &mut self,
        mut older_messages: Vec<Message>,
    ) -> PrependMessagesResult {
        older_messages.retain(|older| {
            !self
                .messages
                .iter()
                .any(|current| current.chat_id == older.chat_id && current.id == older.id)
        });
        if older_messages.is_empty() {
            return PrependMessagesResult {
                added: 0,
                had_unique_messages: false,
            };
        }

        let anchors = self.capture_message_window_anchors();
        let older_identities = older_messages
            .iter()
            .map(Self::message_identity)
            .collect::<HashSet<_>>();
        self.messages.splice(0..0, older_messages);
        let excess = self
            .remote_message_count()
            .saturating_sub(MAX_REMOTE_MESSAGES);
        let protected_end = anchors
            .protected
            .iter()
            .filter_map(|identity| {
                self.messages
                    .iter()
                    .position(|message| Self::message_identity(message) == *identity)
            })
            .max()
            .unwrap_or(0);
        let mut removable = self
            .messages
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, message)| {
                (index > protected_end
                    && Self::is_remote_message(message)
                    && !anchors.protected.contains(&Self::message_identity(message)))
                .then_some(index)
            })
            .take(excess)
            .collect::<HashSet<_>>();
        let removed_newer = !removable.is_empty();
        let remaining = excess.saturating_sub(removable.len());
        if remaining > 0 {
            removable.extend(
                self.messages
                    .iter()
                    .enumerate()
                    .filter_map(|(index, message)| {
                        older_identities
                            .contains(&Self::message_identity(message))
                            .then_some(index)
                    })
                    .take(remaining),
            );
        }
        self.remove_message_indices(&removable);
        if removed_newer {
            self.record_newer_history_gap();
        }
        self.restore_message_window_anchors(&anchors);
        let added = self
            .messages
            .iter()
            .filter(|message| older_identities.contains(&Self::message_identity(message)))
            .count();
        self.ensure_selected_message_visible();
        PrependMessagesResult {
            added,
            had_unique_messages: true,
        }
    }

    pub fn apply_loaded_selected_chat_thread_topics(&mut self, thread_topics: Vec<ThreadTopic>) {
        self.thread_topics = thread_topics;
        self.selected_thread_topic_index = self
            .selected_thread_topic_index
            .min(last_index(self.thread_topics.len()));
        self.ensure_selected_thread_topic_visible();
    }

    pub fn selected_thread_topic(&self) -> Option<&ThreadTopic> {
        self.thread_topics.get(self.selected_thread_topic_index)
    }

    fn reconcile_thread_topic_unread_for_message(&mut self, message: &Message, presented: bool) {
        let Some(topic_id) = message.thread_topic_id else {
            return;
        };
        let Some(topic_index) = self
            .thread_topics
            .iter()
            .position(|topic| topic.id == topic_id)
        else {
            return;
        };

        let topic = &mut self.thread_topics[topic_index];
        if topic_index == self.selected_thread_topic_index && presented {
            topic.unread_count = 0;
        } else if !message.is_own {
            topic.unread_count += 1;
        }
    }

    pub fn select_next_thread_topic(&mut self) {
        if self.thread_topics.is_empty() {
            self.switch_thread_topic_to(0);
        } else {
            self.switch_thread_topic_to(
                (self.selected_thread_topic_index + 1) % self.thread_topics.len(),
            );
        }
    }

    pub fn select_prev_thread_topic(&mut self) {
        if self.thread_topics.is_empty() {
            self.switch_thread_topic_to(0);
        } else {
            self.switch_thread_topic_to(if self.selected_thread_topic_index == 0 {
                last_index(self.thread_topics.len())
            } else {
                self.selected_thread_topic_index - 1
            });
        }
    }

    pub fn clear_loaded_chat_messages(&mut self) {
        self.messages.clear();
        self.newer_history_gap = false;
        self.pending_gap_submit = None;
        self.thread_topics.clear();
        self.selected_thread_topic_index = 0;
        self.thread_topic_scroll_offset = 0;
        self.reset_message_selection();
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.input_scroll_offset = 0;
        self.conversation_load_status = ConversationLoadStatus::Idle;
    }

    pub fn begin_conversation_load(&mut self) {
        self.messages.clear();
        self.newer_history_gap = false;
        self.pending_gap_submit = None;
        self.reset_message_selection();
        self.conversation_load_status = ConversationLoadStatus::Loading;
    }

    pub fn mark_conversation_load_failed(&mut self) {
        self.messages.clear();
        self.newer_history_gap = false;
        self.pending_gap_submit = None;
        self.reset_message_selection();
        self.conversation_load_status = ConversationLoadStatus::Failed;
    }

    pub(crate) fn typing_action_due(&mut self, chat_id: i64, topic_id: Option<i32>) -> bool {
        self.typing_action_due_at(chat_id, topic_id, Instant::now())
    }

    fn typing_action_due_at(&mut self, chat_id: i64, topic_id: Option<i32>, now: Instant) -> bool {
        let context = (chat_id, topic_id);
        let due = self.last_typing_action_context != Some(context)
            || self.last_typing_action_at.is_none_or(|sent_at| {
                now.saturating_duration_since(sent_at) >= TYPING_ACTION_COOLDOWN
            });
        if due {
            self.last_typing_action_context = Some(context);
            self.last_typing_action_at = Some(now);
        }
        due
    }

    pub(crate) fn reset_typing_action_cooldown(&mut self) {
        self.last_typing_action_context = None;
        self.last_typing_action_at = None;
    }

    fn input_visible_capacity(&self) -> usize {
        self.input_area
            .width
            .saturating_sub(PANEL_BORDER_RESERVED_COLUMNS) as usize
    }

    fn input_len(&self) -> usize {
        self.input_buffer.graphemes(true).count()
    }

    pub(crate) fn input_has_submit_text(&self) -> bool {
        !self.input_buffer.trim().is_empty()
    }

    pub fn input_cursor(&self) -> usize {
        self.input_cursor.min(self.input_len())
    }

    fn input_byte_index(&self, grapheme_index: usize) -> usize {
        self.input_buffer
            .grapheme_indices(true)
            .nth(grapheme_index)
            .map(|(idx, _)| idx)
            .unwrap_or(self.input_buffer.len())
    }

    fn clamp_input_cursor(&mut self) {
        self.input_cursor = self.input_cursor();
    }

    fn set_input_cursor_to_end(&mut self) {
        self.input_cursor = self.input_len();
        self.ensure_input_cursor_visible();
    }

    fn input_display_width_between(&self, start: usize, end: usize) -> usize {
        self.input_buffer
            .graphemes(true)
            .skip(start)
            .take(end.saturating_sub(start))
            .map(display_width)
            .sum()
    }

    pub fn effective_input_scroll_offset(&self) -> usize {
        let visible = self.input_visible_capacity();
        if visible == 0 {
            return 0;
        }

        let cursor = self.input_cursor();
        let grapheme_count = self.input_len();
        let max_cursor_column = last_index(visible);
        let mut offset = self.input_scroll_offset.min(grapheme_count);

        if self.input_display_width_between(0, grapheme_count) <= visible {
            return 0;
        }

        if cursor < offset {
            return cursor;
        }

        while offset < cursor
            && self.input_display_width_between(offset, cursor) > max_cursor_column
        {
            offset += 1;
        }

        offset
    }

    pub fn visible_input_text(&self) -> String {
        let visible = self.input_visible_capacity();
        if visible == 0 {
            return String::new();
        }

        let mut output = String::new();
        let mut width = 0;
        for grapheme in self
            .input_buffer
            .graphemes(true)
            .skip(self.effective_input_scroll_offset())
        {
            let grapheme_width = display_width(grapheme);
            if width + grapheme_width > visible {
                break;
            }
            output.push_str(grapheme);
            width += grapheme_width;
        }
        output
    }

    pub fn visible_input_cursor_column(&self) -> usize {
        let visible = self.input_visible_capacity();
        if visible == 0 {
            return 0;
        }

        self.input_display_width_between(self.effective_input_scroll_offset(), self.input_cursor())
            .min(last_index(visible))
    }

    pub fn ensure_input_cursor_visible(&mut self) {
        self.clamp_input_cursor();
        self.input_scroll_offset = self.effective_input_scroll_offset();
    }

    pub fn move_input_cursor_left(&mut self) {
        self.clamp_input_cursor();
        self.input_cursor = self.input_cursor.saturating_sub(1);
        self.ensure_input_cursor_visible();
    }

    pub fn move_input_cursor_right(&mut self) {
        self.clamp_input_cursor();
        self.input_cursor = (self.input_cursor + 1).min(self.input_len());
        self.ensure_input_cursor_visible();
    }

    pub fn move_input_cursor_to_start(&mut self) {
        self.input_cursor = 0;
        self.ensure_input_cursor_visible();
    }

    pub fn move_input_cursor_to_end(&mut self) {
        self.set_input_cursor_to_end();
    }

    pub fn move_input_cursor_to_visible_column(&mut self, column: usize) {
        let visible = self.input_visible_capacity();
        if visible == 0 {
            return;
        }

        let target_column = column.min(visible);
        let offset = self.effective_input_scroll_offset();
        let mut cursor = offset;
        let mut width = 0;

        for (index, grapheme) in self.input_buffer.graphemes(true).enumerate().skip(offset) {
            let grapheme_width = display_width(grapheme);
            if width + grapheme_width > target_column {
                break;
            }

            width += grapheme_width;
            cursor = index + 1;
        }

        self.input_cursor = cursor;
        self.ensure_input_cursor_visible();
    }

    pub fn insert_input_char(&mut self, c: char) {
        self.clamp_input_cursor();
        let byte_index = self.input_byte_index(self.input_cursor);
        self.input_buffer.insert(byte_index, c);
        self.input_cursor += 1;
        self.ensure_input_cursor_visible();
    }

    pub fn backspace_input_char(&mut self) {
        self.clamp_input_cursor();
        if self.input_cursor == 0 {
            return;
        }

        let start = self.input_byte_index(self.input_cursor - 1);
        let end = self.input_byte_index(self.input_cursor);
        self.input_buffer.replace_range(start..end, "");
        self.input_cursor -= 1;
        self.ensure_input_cursor_visible();
    }

    pub fn delete_input_char(&mut self) {
        self.clamp_input_cursor();
        if self.input_cursor >= self.input_len() {
            return;
        }

        let start = self.input_byte_index(self.input_cursor);
        let end = self.input_byte_index(self.input_cursor + 1);
        self.input_buffer.replace_range(start..end, "");
        self.ensure_input_cursor_visible();
    }

    pub fn delete_input_before_cursor(&mut self) {
        self.clamp_input_cursor();
        if self.input_cursor == 0 {
            return;
        }

        let end = self.input_byte_index(self.input_cursor);
        self.input_buffer.replace_range(..end, "");
        self.input_cursor = 0;
        self.ensure_input_cursor_visible();
    }

    pub fn delete_input_after_cursor(&mut self) {
        self.clamp_input_cursor();
        if self.input_cursor >= self.input_len() {
            return;
        }

        let start = self.input_byte_index(self.input_cursor);
        self.input_buffer.replace_range(start.., "");
        self.ensure_input_cursor_visible();
    }

    pub fn delete_input_previous_word(&mut self) {
        self.clamp_input_cursor();
        if self.input_cursor == 0 {
            return;
        }

        let end = self.input_byte_index(self.input_cursor);
        let before_cursor = &self.input_buffer[..end];
        let trimmed_end = before_cursor.trim_end_matches(char::is_whitespace).len();

        let start_byte = if trimmed_end == 0 {
            0
        } else {
            let trimmed = &before_cursor[..trimmed_end];
            trimmed
                .unicode_word_indices()
                .filter_map(|(idx, word)| (idx + word.len() == trimmed_end).then_some(idx))
                .next_back()
                .or_else(|| {
                    trimmed
                        .split_word_bound_indices()
                        .next_back()
                        .map(|(idx, _)| idx)
                })
                .unwrap_or(0)
        };

        self.input_buffer.replace_range(start_byte..end, "");
        self.input_cursor = self.input_buffer[..start_byte].graphemes(true).count();
        self.ensure_input_cursor_visible();
    }

    pub fn save_current_draft(&mut self) {
        if self.editing_message_id.is_some() || self.replying_to_message_id.is_some() {
            return;
        }

        let Some(scope) = self.selected_conversation_scope() else {
            return;
        };
        let draft = if let Some(recovery) = self.failed_submission_recovery.get_mut(&scope) {
            recovery.capture_base(&self.input_buffer, None);
            recovery.base.clone().unwrap_or_default()
        } else {
            self.input_buffer.clone()
        };
        if draft.is_empty() {
            self.conversation_drafts.remove(&scope);
        } else {
            self.conversation_drafts.insert(scope, draft);
        }
    }

    pub fn restore_draft_for_selected_chat(&mut self) {
        if self.editing_message_id.is_some() || self.replying_to_message_id.is_some() {
            return;
        }

        self.input_buffer = self
            .selected_conversation_scope()
            .and_then(|scope| self.conversation_drafts.get(&scope).cloned())
            .unwrap_or_default();
        if let Some(scope) = self.selected_conversation_scope()
            && let Some(recovery) = self.failed_submission_recovery.get_mut(&scope)
        {
            let matches_recovered_submission = recovery
                .submissions
                .values()
                .any(|content| content == &self.input_buffer);
            recovery.capture_base(
                &self.input_buffer,
                matches_recovered_submission.then_some(self.input_buffer.as_str()),
            );
            self.input_buffer = recovery.merged();
        }
        self.set_input_cursor_to_end();
    }

    pub fn discard_draft_for_selected_chat(&mut self) {
        if let Some(scope) = self.selected_conversation_scope() {
            self.conversation_drafts.remove(&scope);
            self.failed_submission_recovery.remove(&scope);
        }
    }

    pub fn leave_selected_chat(&mut self) {
        self.save_current_draft();

        if self.editing_message_id.is_some() || self.replying_to_message_id.is_some() {
            self.clear_input_mode();
        }
    }

    pub fn reconciliation_context(&self) -> ReconciliationContext {
        ReconciliationContext {
            folder_id: self
                .folders
                .get(self.selected_folder_index)
                .map(|folder| folder.id),
            chat_id: self.selected_chat_id(),
            topic_id: self.selected_thread_topic().map(|topic| topic.id),
            message_id: self.selected_message().map(|message| message.id),
        }
    }

    pub fn apply_reconciliation_snapshot(
        &mut self,
        context: ReconciliationContext,
        snapshot: ReconciliationSnapshot,
    ) -> ReconciliationApply {
        let current_context = self.reconciliation_context();
        if (
            current_context.folder_id,
            current_context.chat_id,
            current_context.topic_id,
        ) != (context.folder_id, context.chat_id, context.topic_id)
        {
            return ReconciliationApply::Stale;
        }

        let focused_panel = self.focused_panel;
        let selected_message_id = current_context.message_id;
        let reading_older_history = !self.messages.is_empty() && !self.selected_message_is_last();
        let local_rows = self
            .messages
            .iter()
            .filter(|message| {
                matches!(
                    message.status,
                    MessageStatus::Sending | MessageStatus::Failed
                )
            })
            .cloned()
            .collect::<Vec<_>>();

        let selected_chat_changed = context.chat_id != snapshot.selected_chat_id;
        let selected_scope_changed =
            selected_chat_changed || context.topic_id != snapshot.selected_topic_id;
        if selected_scope_changed {
            self.leave_selected_chat();
        }

        self.folders = snapshot.folders;
        self.selected_folder_index = snapshot
            .selected_folder_id
            .and_then(|folder_id| {
                self.folders
                    .iter()
                    .position(|folder| folder.id == folder_id)
            })
            .unwrap_or(0)
            .min(last_index(self.folders.len()));
        self.ensure_selected_folder_visible();

        self.chats = snapshot.chats;
        self.selected_chat_index = snapshot
            .selected_chat_id
            .and_then(|chat_id| self.chats.iter().position(|chat| chat.id == chat_id))
            .unwrap_or(0)
            .min(last_index(self.chats.len()));
        self.ensure_selected_chat_visible();
        self.cache_selected_folder_chats();

        let selected_message_in_snapshot = selected_message_id.is_some_and(|message_id| {
            snapshot.messages.iter().any(|message| {
                Some(message.chat_id) == snapshot.selected_chat_id && message.id == message_id
            })
        });
        let preserve_conversation = !selected_chat_changed
            && context.topic_id == snapshot.selected_topic_id
            && reading_older_history
            && !selected_message_in_snapshot;

        self.thread_topics = snapshot.thread_topics;
        self.selected_thread_topic_index = snapshot
            .selected_topic_id
            .and_then(|topic_id| {
                self.thread_topics
                    .iter()
                    .position(|topic| topic.id == topic_id)
            })
            .unwrap_or(0)
            .min(last_index(self.thread_topics.len()));
        self.ensure_selected_thread_topic_visible();

        if !preserve_conversation {
            self.messages = snapshot.messages;
            for local in local_rows {
                if Some(local.chat_id) == snapshot.selected_chat_id
                    && local.thread_topic_id == snapshot.selected_topic_id
                    && !self.messages.iter().any(|current| current.id == local.id)
                {
                    self.messages.push(local);
                }
            }
            self.retain_newest_remote_messages();
            self.newer_history_gap = false;
            self.newer_history_generation = self.newer_history_generation.saturating_add(1);
            self.pending_gap_submit = None;
            self.selected_message_index = selected_message_id
                .and_then(|message_id| {
                    self.messages
                        .iter()
                        .position(|message| message.id == message_id)
                })
                .unwrap_or_else(|| last_index(self.messages.len()));
            self.ensure_selected_message_visible();
            self.conversation_load_status = if self.messages.is_empty() {
                ConversationLoadStatus::Empty
            } else {
                ConversationLoadStatus::Loaded
            };
            self.clear_selected_scope_older_history_exhausted();
        }

        self.typing_activity.clear();
        self.revalidate_reconciled_targets();
        if selected_scope_changed {
            self.clear_input_mode();
            self.restore_draft_for_selected_chat();
        }
        self.focused_panel = if self.chats.is_empty()
            && matches!(focused_panel, FocusedPanel::Messages | FocusedPanel::Input)
        {
            FocusedPanel::Chats
        } else {
            focused_panel
        };
        if !preserve_conversation && self.terminal_focused {
            self.clear_selected_conversation_unread(snapshot.selected_topic_id);
        }

        ReconciliationApply::Applied {
            conversation_replaced: !preserve_conversation,
        }
    }

    fn clear_selected_conversation_unread(&mut self, selected_topic_id: Option<i32>) {
        let topic_cleared = selected_topic_id.and_then(|topic_id| {
            self.thread_topics
                .iter_mut()
                .find(|topic| topic.id == topic_id)
                .map(|topic| {
                    let unread_count = topic.unread_count;
                    topic.unread_count = 0;
                    unread_count
                })
        });
        let Some(selected_chat) = self.chats.get_mut(self.selected_chat_index) else {
            return;
        };
        let cleared = match selected_topic_id {
            Some(_) => topic_cleared.unwrap_or(0),
            None => selected_chat.unread_count,
        };
        if cleared == 0 {
            return;
        }
        selected_chat.unread_count = if selected_topic_id.is_some() {
            selected_chat.unread_count.saturating_sub(cleared)
        } else {
            0
        };
    }

    fn revalidate_reconciled_targets(&mut self) {
        let selected_chat_id = self.selected_chat_id();
        let message_exists = |message_id| {
            self.messages.iter().any(|message| {
                Some(message.chat_id) == selected_chat_id && message.id == message_id
            })
        };
        if self
            .delete_confirmation()
            .is_some_and(|confirmation| !message_exists(confirmation.message_id))
        {
            self.modal = None;
        }
        if self
            .editing_message_id
            .is_some_and(|id| !message_exists(id))
            || self
                .replying_to_message_id
                .is_some_and(|id| !message_exists(id))
        {
            self.cancel_compose_mode();
        }
        if let Some(target) = self.context_menu().map(|menu| menu.target)
            && self.context_actions_for_target(target).is_empty()
        {
            self.modal = None;
        }
    }

    pub fn apply_update(&mut self, update: Update) -> Option<PresentedIncomingMessage> {
        let mut presented_incoming = None;
        match update {
            Update::NewMessage(msg) => {
                let current_chat_id = self.chats.get(self.selected_chat_index).map(|c| c.id);
                let selected_thread_topic_id = self.selected_thread_topic().map(|topic| topic.id);
                let matches_selected_thread_topic = msg.thread_topic_id == selected_thread_topic_id;
                let should_append_to_loaded_messages =
                    current_chat_id == Some(msg.chat_id) && matches_selected_thread_topic;
                if should_append_to_loaded_messages
                    && self
                        .messages
                        .iter()
                        .any(|message| message.chat_id == msg.chat_id && message.id >= msg.id)
                {
                    diagnostics::event(
                        "new_message_update_ignored",
                        format!(
                            "reason=stale_loaded_message chat_id={} message_id={} loaded_count={}",
                            msg.chat_id,
                            msg.id,
                            self.messages.len()
                        ),
                    );
                    return None;
                }

                if let Some(sender_identity) = msg.sender_identity {
                    self.clear_typing_activity(msg.chat_id, msg.thread_topic_id, sender_identity);
                }

                let was_following_tail = should_append_to_loaded_messages
                    && !self.newer_history_gap
                    && (self.messages.is_empty() || self.selected_message_is_last());
                let retained = should_append_to_loaded_messages
                    && self.append_remote_message_with_retention(msg.clone());
                let presented =
                    self.terminal_focused && !msg.is_own && retained && was_following_tail;
                if retained && was_following_tail {
                    self.select_message_by_identity(msg.chat_id, msg.id);
                }
                if presented {
                    presented_incoming = Some(PresentedIncomingMessage {
                        chat_id: msg.chat_id,
                        topic_id: msg.thread_topic_id,
                        message_id: msg.id,
                    });
                }

                let selected_topic_unread = selected_thread_topic_id
                    .filter(|topic_id| msg.thread_topic_id == Some(*topic_id))
                    .and_then(|topic_id| {
                        self.thread_topics
                            .iter()
                            .find(|topic| topic.id == topic_id)
                            .map(|topic| topic.unread_count)
                    })
                    .unwrap_or(0);
                if current_chat_id == Some(msg.chat_id) {
                    self.reconcile_thread_topic_unread_for_message(&msg, presented);
                }

                if let Some(chat) = self.chats.iter_mut().find(|c| c.id == msg.chat_id) {
                    chat.last_message =
                        Some(message_display_preview(msg.media.as_ref(), &msg.content));
                    if current_chat_id == Some(msg.chat_id) {
                        match selected_thread_topic_id {
                            Some(topic_id)
                                if msg.thread_topic_id == Some(topic_id) && presented =>
                            {
                                chat.unread_count =
                                    chat.unread_count.saturating_sub(selected_topic_unread);
                            }
                            Some(_) if !msg.is_own => chat.unread_count += 1,
                            Some(_) => {}
                            None if presented => chat.unread_count = 0,
                            None if !msg.is_own => chat.unread_count += 1,
                            None => {}
                        }
                    } else if !msg.is_own {
                        chat.unread_count += 1;
                    }
                }
            }
            Update::EditMessage {
                chat_id,
                message_id,
                new_content,
            } => {
                if let Some(msg) = self
                    .messages
                    .iter_mut()
                    .find(|m| m.id == message_id && m.chat_id == chat_id)
                {
                    msg.content = new_content;
                    msg.is_edited = true;
                }

                if self.selected_chat_id() == Some(chat_id) {
                    self.refresh_selected_chat_last_message_from_loaded_messages();
                }

                if self.editing_message_id == Some(message_id)
                    && self.selected_chat_id() == Some(chat_id)
                {
                    self.set_status(REMOTE_EDIT_WHILE_EDITING_STATUS);
                }
            }
            Update::DeleteMessage {
                chat_id,
                message_id,
            } => {
                self.finish_delete_submissions_for_update(chat_id, message_id);
                let anchors = self.capture_message_window_anchors();
                if chat_id == UNKNOWN_DELETE_UPDATE_CHAT_ID {
                    self.messages.retain(|m| m.id != message_id);
                } else {
                    self.messages
                        .retain(|m| !(m.id == message_id && m.chat_id == chat_id));
                }

                if self.delete_confirmation().is_some_and(|confirmation| {
                    confirmation.message_id == message_id
                        && delete_update_matches_chat(chat_id, confirmation.chat_id)
                }) {
                    self.modal = None;
                }
                self.clear_compose_for_deleted_message(chat_id, message_id);

                self.restore_message_window_anchors(&anchors);
                self.ensure_selected_message_visible();
                if self.selected_chat_id().is_some_and(|selected_chat_id| {
                    delete_update_matches_chat(chat_id, selected_chat_id)
                }) {
                    self.refresh_selected_chat_last_message_from_loaded_messages();
                }
            }
            Update::ReadOutgoingMessages {
                chat_id,
                max_message_id,
            } => {
                for message in &mut self.messages {
                    if message.chat_id == chat_id
                        && message.is_own
                        && message.id <= max_message_id
                        && !matches!(
                            message.status,
                            MessageStatus::Sending | MessageStatus::Failed | MessageStatus::Read
                        )
                    {
                        message.status = MessageStatus::Read;
                    }
                }
            }
            Update::TypingStatus {
                chat_id,
                topic_id,
                sender_identity,
                user_name,
                is_typing,
            } => {
                let key = TypingActivityKey {
                    chat_id,
                    topic_id,
                    sender_identity,
                };
                if is_typing {
                    self.typing_activity.insert(
                        key,
                        TypingActivity {
                            display_name: user_name,
                            expires_at: tokio::time::Instant::now() + TYPING_ACTIVITY_LIFETIME,
                        },
                    );
                } else {
                    self.typing_activity.remove(&key);
                }
            }
            Update::Error(error) => self.set_error(error),
        }
        presented_incoming
    }

    pub fn selected_typing_users(&self) -> Option<Vec<String>> {
        let chat_id = self.selected_chat_id()?;
        let topic_id = self.selected_thread_topic().map(|topic| topic.id);
        let users = self
            .typing_activity
            .iter()
            .filter(|(key, _)| key.chat_id == chat_id && key.topic_id == topic_id)
            .map(|(_, activity)| activity.display_name.clone())
            .collect::<Vec<_>>();
        (!users.is_empty()).then_some(users)
    }

    fn clear_typing_activity(
        &mut self,
        chat_id: i64,
        topic_id: Option<i32>,
        sender_identity: SenderIdentity,
    ) {
        self.typing_activity.remove(&TypingActivityKey {
            chat_id,
            topic_id,
            sender_identity,
        });
    }

    pub fn chat_visible_capacity(&self) -> usize {
        (self
            .chats_area
            .height
            .saturating_sub(PANEL_BORDER_RESERVED_ROWS)
            / CHAT_LIST_ITEM_HEIGHT)
            .max(1) as usize
    }

    pub fn ensure_selected_chat_visible(&mut self) {
        if self.chats.is_empty() {
            self.selected_chat_index = 0;
            self.chat_scroll_offset = 0;
            return;
        }

        self.selected_chat_index = self.selected_chat_index.min(last_index(self.chats.len()));
        let capacity = self.chat_visible_capacity();
        let max_scroll_offset = self.chats.len().saturating_sub(capacity);
        self.chat_scroll_offset = self.chat_scroll_offset.min(max_scroll_offset);
        if self.selected_chat_index < self.chat_scroll_offset {
            self.chat_scroll_offset = self.selected_chat_index;
        } else if self.selected_chat_index >= self.chat_scroll_offset + capacity {
            self.chat_scroll_offset = self.selected_chat_index + 1 - capacity;
        }
    }

    pub fn reset_chat_selection(&mut self) {
        self.selected_chat_index = 0;
        self.chat_scroll_offset = 0;
        self.chat_search_selected_index = 0;
    }

    pub fn message_visible_capacity(&self) -> usize {
        self.messages_area
            .height
            .saturating_sub(PANEL_BORDER_RESERVED_ROWS)
            .max(1) as usize
    }

    fn message_text_width(&self) -> usize {
        self.messages_area.width.saturating_sub(5) as usize
    }

    pub fn ensure_selected_message_visible(&mut self) {
        if self.messages.is_empty() {
            self.selected_message_index = 0;
            self.message_scroll_offset = 0;
            return;
        }

        self.selected_message_index = self
            .selected_message_index
            .min(last_index(self.messages.len()));
        self.message_scroll_offset = self
            .message_scroll_offset
            .min(last_index(self.messages.len()));
        let (selected_is_visible, visible_rows) = self.message_viewport_status();
        if !selected_is_visible
            || (visible_rows < self.message_visible_capacity() && self.message_scroll_offset > 0)
        {
            self.message_scroll_offset = self.message_scroll_offset_for_selected();
        }
    }

    fn message_viewport_status(&self) -> (bool, usize) {
        let capacity = self.message_visible_capacity();
        let width = self.message_text_width();
        let mut visible_rows = 0;
        let mut selected_is_visible = false;

        for (idx, message) in self
            .messages
            .iter()
            .enumerate()
            .skip(self.message_scroll_offset)
        {
            let remaining_rows = capacity.saturating_sub(visible_rows);
            if remaining_rows == 0 {
                break;
            }
            if idx == self.selected_message_index {
                selected_is_visible = true;
            }
            visible_rows +=
                message_visible_row_height_for_width_capped(message, width, remaining_rows);
            if visible_rows >= capacity {
                break;
            }
        }

        (selected_is_visible, visible_rows)
    }

    fn message_scroll_offset_for_selected(&self) -> usize {
        let capacity = self.message_visible_capacity();
        let width = self.message_text_width();
        let mut rows = 0;
        let mut offset = self.selected_message_index;

        for idx in (0..=self.selected_message_index).rev() {
            let remaining_rows = capacity.saturating_sub(rows);
            if remaining_rows == 0 {
                break;
            }
            let message_height = message_visible_row_height_for_width_capped(
                &self.messages[idx],
                width,
                remaining_rows.saturating_add(1),
            );
            if message_height > remaining_rows {
                break;
            }
            rows += message_height;
            offset = idx;
        }

        offset
    }

    pub fn reset_message_selection(&mut self) {
        self.selected_message_index = 0;
        self.message_scroll_offset = 0;
    }

    pub fn selected_message(&self) -> Option<&Message> {
        self.messages.get(self.selected_message_index)
    }

    pub fn selected_media_preview_request(&self) -> Option<(i64, i32)> {
        let message = self.selected_message()?;
        let media = message.media.as_ref()?;
        (media.local_path.is_none()
            && matches!(
                media.kind,
                MessageMediaKind::Photo | MessageMediaKind::Image
            ))
        .then_some((message.chat_id, message.id))
    }

    pub fn apply_selected_media_preview(
        &mut self,
        chat_id: i64,
        message_id: i32,
        path: PathBuf,
    ) -> bool {
        let Some(message) = self.messages.get_mut(self.selected_message_index) else {
            return false;
        };
        if message.chat_id != chat_id || message.id != message_id {
            return false;
        }
        let Some(media) = message.media.as_mut() else {
            return false;
        };
        if !matches!(
            media.kind,
            MessageMediaKind::Photo | MessageMediaKind::Image
        ) {
            return false;
        }
        media.local_path = Some(path);
        true
    }

    pub fn record_downloaded_media(&mut self, chat_id: i64, message_id: i32, path: PathBuf) {
        self.last_downloaded_media = Some(DownloadedMediaReference {
            chat_id,
            message_id,
            path,
        });
    }

    pub fn selected_message_download_path(&self) -> Option<&Path> {
        let message = self.selected_message()?;
        let downloaded = self.last_downloaded_media.as_ref()?;
        (downloaded.chat_id == message.chat_id && downloaded.message_id == message.id)
            .then_some(downloaded.path.as_path())
    }

    pub fn selected_message_is_last(&self) -> bool {
        self.messages.is_empty() || self.selected_message_index >= last_index(self.messages.len())
    }

    pub fn message_index_at_visible_row(&self, row: usize) -> Option<usize> {
        if self.messages.is_empty() {
            return None;
        }

        let mut current_row = 0;
        for (idx, message) in self
            .messages
            .iter()
            .enumerate()
            .skip(self.message_scroll_offset)
        {
            let remaining_rows = self.message_visible_capacity().saturating_sub(current_row);
            let message_height = message_visible_row_height_for_width_capped(
                message,
                self.message_text_width(),
                remaining_rows,
            );
            if row < current_row + message_height {
                return Some(idx);
            }
            current_row += message_height;
            if current_row >= self.message_visible_capacity() {
                return None;
            }
        }
        None
    }

    pub fn select_message_at_visible_row(&mut self, row: usize) {
        if let Some(index) = self.message_index_at_visible_row(row) {
            self.selected_message_index = index;
            self.ensure_selected_message_visible();
        }
    }

    fn is_remote_actionable_message(message: &Message) -> bool {
        !matches!(
            message.status,
            MessageStatus::Sending | MessageStatus::Failed
        )
    }

    pub fn request_edit_selected_message(&mut self) {
        if self.reply_submission_pending() || self.edit_submission_pending() {
            return;
        }
        if let Some((message_id, content, can_edit)) = self.selected_message().map(|msg| {
            (
                msg.id,
                msg.content.clone(),
                msg.is_own && msg.can_edit && Self::is_remote_actionable_message(msg),
            )
        }) {
            if can_edit {
                self.save_current_draft();
                self.enter_edit_mode(message_id, content);
            } else {
                self.set_error(CANNOT_EDIT_MESSAGE_ERROR.to_string());
            }
        }
    }

    pub fn request_reply_to_selected_message(&mut self) {
        if self.reply_submission_pending() || self.edit_submission_pending() {
            return;
        }
        if let Some((message_id, can_reply)) = self
            .selected_message()
            .map(|msg| (msg.id, Self::is_remote_actionable_message(msg)))
        {
            if can_reply {
                self.save_current_draft();
                self.enter_reply_mode(message_id);
            } else {
                self.set_error(CANNOT_REPLY_UNSENT_MESSAGE_ERROR.to_string());
            }
        }
    }

    pub fn request_delete_selected_message(&mut self) {
        let Some((chat_id, message_id, status, can_delete)) = self.selected_message().map(|msg| {
            (
                msg.chat_id,
                msg.id,
                msg.status.clone(),
                msg.is_own && msg.can_delete && Self::is_remote_actionable_message(msg),
            )
        }) else {
            return;
        };

        if status == MessageStatus::Failed {
            self.dismiss_failed_send(chat_id, message_id);
        } else if !self.delete_submission_pending_for(chat_id, message_id) {
            if can_delete {
                self.set_delete_confirmation(DeleteConfirmation {
                    chat_id,
                    message_id,
                });
            } else {
                self.set_error(CANNOT_DELETE_MESSAGE_ERROR.to_string());
            }
        }
    }

    fn dismiss_failed_send(&mut self, chat_id: i64, message_id: i32) {
        self.messages
            .retain(|m| !(m.id == message_id && m.chat_id == chat_id));
        if self.selected_message_index >= self.messages.len() && !self.messages.is_empty() {
            self.selected_message_index = self.messages.len() - 1;
        }
        self.ensure_selected_message_visible();
        self.refresh_selected_chat_last_message_from_loaded_messages();
        self.modal = None;
        self.set_status(FAILED_SEND_DISMISSED_STATUS);
    }

    pub fn select_next_message(&mut self) {
        if !self.messages.is_empty() {
            self.selected_message_index =
                (self.selected_message_index + 1).min(self.messages.len() - 1);
            self.ensure_selected_message_visible();
        }
    }

    pub fn select_prev_message(&mut self) {
        if self.selected_message_index > 0 {
            self.selected_message_index -= 1;
            self.ensure_selected_message_visible();
        }
    }

    pub fn select_first_message(&mut self) {
        if !self.messages.is_empty() {
            self.selected_message_index = 0;
            self.ensure_selected_message_visible();
        }
    }

    pub fn select_last_message(&mut self) {
        if !self.messages.is_empty() {
            self.selected_message_index = self.messages.len() - 1;
            self.ensure_selected_message_visible();
        }
    }

    pub fn page_messages_down(&mut self) {
        if !self.messages.is_empty() {
            self.selected_message_index = (self.selected_message_index
                + self.message_visible_capacity())
            .min(self.messages.len() - 1);
            self.ensure_selected_message_visible();
        }
    }

    pub fn page_messages_up(&mut self) {
        if !self.messages.is_empty() {
            self.selected_message_index = self
                .selected_message_index
                .saturating_sub(self.message_visible_capacity());
            self.message_scroll_offset = self.selected_message_index;
            self.ensure_selected_message_visible();
        }
    }

    pub fn adjust_split_left(&mut self) {
        self.split_ratio = (self.split_ratio - SPLIT_RATIO_STEP).max(MIN_SPLIT_RATIO);
    }

    pub fn adjust_split_right(&mut self) {
        self.split_ratio = (self.split_ratio + SPLIT_RATIO_STEP).min(MAX_SPLIT_RATIO);
    }

    pub fn focus_next_panel(&mut self) {
        self.focused_panel = match self.focused_panel {
            FocusedPanel::Folders => FocusedPanel::Chats,
            FocusedPanel::Chats => FocusedPanel::Messages,
            FocusedPanel::Messages => FocusedPanel::Input,
            FocusedPanel::Input => FocusedPanel::Folders,
        };
    }

    pub fn clear_input_mode(&mut self) {
        self.editing_message_id = None;
        self.replying_to_message_id = None;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.input_scroll_offset = 0;
        self.reset_typing_action_cooldown();
    }

    pub fn cancel_compose_mode(&mut self) {
        self.clear_input_mode();
        self.restore_draft_for_selected_chat();
        self.focused_panel = FocusedPanel::Messages;
    }

    pub fn cancel_input_mode(&mut self) {
        if self.editing_message_id.is_none() && self.replying_to_message_id.is_none() {
            self.discard_draft_for_selected_chat();
            self.clear_input_mode();
            self.focused_panel = FocusedPanel::Messages;
        } else {
            self.cancel_compose_mode();
        }
    }

    pub fn finish_compose_mode(&mut self) {
        self.clear_input_mode();
        self.restore_draft_for_selected_chat();
    }

    pub fn prepare_message_submit(&mut self) -> Option<MessageSubmitAction> {
        if !self.input_has_submit_text() {
            return None;
        }

        let Some(chat_id) = self.selected_chat_id() else {
            self.set_error(NO_CHAT_SELECTED_ERROR.to_string());
            return None;
        };

        let content = self.input_buffer.clone();
        let topic_id = self.selected_thread_topic().map(|topic| topic.id);
        let action = if let Some(message_id) = self.editing_message_id {
            MessageSubmitAction::Edit {
                chat_id,
                message_id,
                content,
            }
        } else if let Some(message_id) = self.replying_to_message_id {
            MessageSubmitAction::Reply {
                chat_id,
                thread_top_message_id: topic_id,
                message_id,
                content,
            }
        } else {
            MessageSubmitAction::Send {
                chat_id,
                thread_top_message_id: topic_id,
                content,
            }
        };
        self.failed_submission_recovery
            .remove(&ConversationScope { chat_id, topic_id });
        Some(action)
    }

    fn refresh_selected_chat_last_message_from_loaded_messages(&mut self) {
        if self.newer_history_gap {
            return;
        }
        let selected_chat_id = self.selected_chat_id();
        let last_message = selected_chat_id.and_then(|chat_id| {
            self.messages
                .iter()
                .rev()
                .find(|message| message.chat_id == chat_id)
                .map(|message| message_display_preview(message.media.as_ref(), &message.content))
        });

        if let Some(chat) = self.chats.get_mut(self.selected_chat_index) {
            chat.last_message = last_message;
        }
    }

    pub fn apply_edit_success(&mut self, message_id: i32, content: String) {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == message_id) {
            msg.content = content;
            msg.is_edited = true;
        }
        self.refresh_selected_chat_last_message_from_loaded_messages();
        self.set_status(MESSAGE_EDITED_STATUS);
        self.finish_edit_submission();
        self.finish_compose_mode();
    }

    pub fn apply_edit_failure(&mut self, error: String) {
        self.finish_edit_submission();
        self.set_error(edit_failed_error(error));
    }

    pub fn apply_reply_success(&mut self, message: Message) {
        let identity = Self::message_identity(&message);
        if !self.newer_history_gap {
            let retained = self.upsert_confirmed_message(message, None);
            if retained {
                self.selected_message_index = self
                    .messages
                    .iter()
                    .position(|message| Self::message_identity(message) == identity)
                    .unwrap_or_else(|| last_index(self.messages.len()));
                self.ensure_selected_message_visible();
            }
        }
        self.refresh_selected_chat_last_message_from_loaded_messages();
        self.set_status(REPLY_SENT_STATUS);
        self.finish_reply_submission();
        self.finish_compose_mode();
    }

    pub fn apply_reply_failure(&mut self, error: String) {
        self.finish_reply_submission();
        self.set_error(reply_failed_error(error));
    }

    pub fn apply_send_pending(
        &mut self,
        temp_id: i32,
        chat_id: i64,
        thread_topic_id: Option<i32>,
        content: String,
    ) {
        self.messages.push(Message {
            id: temp_id,
            chat_id,
            thread_topic_id,
            sender_identity: None,
            sender_name: OWN_SENDER_NAME.to_string(),
            content,
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_content: None,
            media: None,
            status: MessageStatus::Sending,
            can_edit: false,
            can_delete: false,
            error: None,
        });
        self.select_last_message();
        self.refresh_selected_chat_last_message_from_loaded_messages();
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.input_scroll_offset = 0;
        self.reset_typing_action_cooldown();
        self.discard_draft_for_selected_chat();
    }

    pub fn apply_send_success(&mut self, temp_id: i32, sent_message: Message) {
        let chat_id = sent_message.chat_id;
        let sent_topic_id = sent_message.thread_topic_id;
        if self
            .messages
            .iter()
            .any(|message| message.id == temp_id && message.chat_id == chat_id)
        {
            if self.newer_history_gap {
                let anchors = self.capture_message_window_anchors();
                self.messages
                    .retain(|message| !(message.id == temp_id && message.chat_id == chat_id));
                self.restore_message_window_anchors(&anchors);
                self.ensure_selected_message_visible();
            } else {
                self.upsert_confirmed_message(sent_message, Some((chat_id, temp_id)));
            }
        }
        if self.terminal_focused && self.selected_chat_id() == Some(chat_id) {
            let selected_topic_id = self.selected_thread_topic().map(|topic| topic.id);
            if selected_topic_id == sent_topic_id {
                self.clear_selected_conversation_unread(selected_topic_id);
            }
        }
        self.refresh_selected_chat_last_message_from_loaded_messages();
        self.clear_status();
    }

    pub fn mark_send_failed_row(&mut self, chat_id: i64, temp_id: i32, error: &str) {
        if let Some(msg) = self
            .messages
            .iter_mut()
            .find(|message| message.chat_id == chat_id && message.id == temp_id)
        {
            msg.status = MessageStatus::Failed;
            msg.can_edit = false;
            msg.can_delete = false;
            msg.error = Some(error.to_string());
        }
    }

    pub fn apply_send_failure(&mut self, temp_id: i32, error: String) {
        let failed = self
            .messages
            .iter()
            .find(|message| message.id == temp_id)
            .map(|message| {
                (
                    message.chat_id,
                    message.thread_topic_id,
                    message.content.clone(),
                )
            });
        if let Some((chat_id, topic_id, content)) = failed {
            self.mark_send_failed_row(chat_id, temp_id, &error);
            self.recover_failed_submission(0, chat_id, topic_id, content);
            self.save_current_draft();
        }
        self.set_error(send_failed_error(error));
    }

    pub fn apply_delete_success(&mut self, confirmation: DeleteConfirmation) {
        let anchors = self.capture_message_window_anchors();
        self.messages
            .retain(|m| !(m.id == confirmation.message_id && m.chat_id == confirmation.chat_id));
        self.restore_message_window_anchors(&anchors);
        self.ensure_selected_message_visible();
        self.refresh_selected_chat_last_message_from_loaded_messages();
        self.set_status(MESSAGE_DELETED_STATUS);
    }

    pub fn apply_delete_failure(&mut self, _confirmation: DeleteConfirmation, error: String) {
        self.set_error(delete_failed_error(error));
    }

    pub fn cancel_delete_confirmation(&mut self) {
        if matches!(self.modal.as_ref(), Some(ModalState::DeleteConfirmation(_))) {
            self.modal = None;
        }
    }

    pub fn clear_compose_for_deleted_message(&mut self, chat_id: i64, message_id: i32) -> bool {
        let current_chat_matches = self
            .selected_chat_id()
            .is_some_and(|selected_chat_id| delete_update_matches_chat(chat_id, selected_chat_id));
        let compose_target_matches = self.editing_message_id == Some(message_id)
            || self.replying_to_message_id == Some(message_id);

        if current_chat_matches && compose_target_matches {
            self.cancel_compose_mode();
            true
        } else {
            false
        }
    }

    pub fn enter_edit_mode(&mut self, message_id: i32, content: String) {
        self.editing_message_id = Some(message_id);
        self.input_buffer = content;
        self.set_input_cursor_to_end();
        self.focused_panel = FocusedPanel::Input;
    }

    pub fn enter_reply_mode(&mut self, message_id: i32) {
        self.replying_to_message_id = Some(message_id);
        self.set_input_cursor_to_end();
        self.focused_panel = FocusedPanel::Input;
    }

    pub fn set_error(&mut self, error: String) {
        if self.mutation_outcome_unknown {
            return;
        }
        self.error_message = Some(error);
        self.error_timestamp = Some(tokio::time::Instant::now());
        self.clear_status();
    }

    pub fn set_mutation_outcome_unknown(&mut self, error: String) {
        self.error_message = Some(error);
        self.error_timestamp = None;
        self.mutation_outcome_unknown = true;
        self.clear_status();
    }

    pub fn acknowledge_mutation_outcome_unknown(&mut self) {
        self.mutation_outcome_unknown = false;
        self.clear_error();
        self.clear_status();
    }

    pub fn clear_error(&mut self) {
        if self.mutation_outcome_unknown {
            return;
        }
        self.error_message = None;
        self.error_timestamp = None;
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        if self.mutation_outcome_unknown {
            return;
        }
        self.clear_error();
        self.status_message = Some(status.into());
        self.status_timestamp = Some(tokio::time::Instant::now());
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
        self.status_timestamp = None;
    }

    pub fn notification_deadline(&self) -> Option<tokio::time::Instant> {
        let notification = match (self.error_timestamp, self.status_timestamp) {
            (Some(error), Some(status)) => Some(error.min(status)),
            (error, status) => error.or(status),
        }
        .map(|timestamp| timestamp + NOTIFICATION_LIFETIME);
        let typing = self
            .typing_activity
            .values()
            .map(|activity| activity.expires_at)
            .min();
        match (notification, typing) {
            (Some(notification), Some(typing)) => Some(notification.min(typing)),
            (notification, typing) => notification.or(typing),
        }
    }

    pub fn check_notification_timeout(&mut self) -> bool {
        let now = tokio::time::Instant::now();
        let clear_error = self
            .error_timestamp
            .is_some_and(|timestamp| now >= timestamp + NOTIFICATION_LIFETIME);
        let clear_status = self
            .status_timestamp
            .is_some_and(|timestamp| now >= timestamp + NOTIFICATION_LIFETIME);

        if clear_error {
            self.clear_error();
        }
        if clear_status {
            self.clear_status();
        }
        let typing_count = self.typing_activity.len();
        self.typing_activity
            .retain(|_, activity| now < activity.expires_at);

        clear_error || clear_status || self.typing_activity.len() != typing_count
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppState, CANNOT_DELETE_MESSAGE_ERROR, CANNOT_EDIT_MESSAGE_ERROR,
        CANNOT_REPLY_UNSENT_MESSAGE_ERROR, CHAT_LIST_ITEM_HEIGHT, ContextMenuAction,
        ContextMenuTarget, ConversationLoadStatus, DEFAULT_SPLIT_RATIO, DeleteConfirmation,
        FAILED_SEND_DISMISSED_STATUS, FOLDER_VIEWPORT_RESERVED_COLUMNS, FocusedPanel,
        MAX_REMOTE_MESSAGES, MAX_SPLIT_RATIO, MESSAGE_DELETED_STATUS, MESSAGE_EDITED_STATUS,
        MESSAGE_ROW_HEIGHT, MIN_SPLIT_RATIO, MessageSubmitAction, NO_CHAT_SELECTED_ERROR,
        NOTIFICATION_LIFETIME, PANEL_BORDER_RESERVED_COLUMNS, PANEL_BORDER_RESERVED_ROWS,
        PresentedIncomingMessage, REMOTE_EDIT_WHILE_EDITING_STATUS, REPLY_MESSAGE_ROW_HEIGHT,
        REPLY_SENT_STATUS, ReconciliationApply, ReconciliationSnapshot, SPLIT_RATIO_STEP,
        TYPING_ACTION_COOLDOWN, TYPING_ACTIVITY_LIFETIME, delete_failed_error,
        delete_update_matches_chat, edit_failed_error, last_index, message_visible_row_height,
        message_visible_row_height_for_width, reply_failed_error, send_failed_error,
    };
    use crate::telegram::types::{
        Chat, Folder, Message, MessageMedia, MessageStatus, OWN_SENDER_NAME, SenderIdentity,
        ThreadTopic, UNKNOWN_DELETE_UPDATE_CHAT_ID, Update, all_folder,
    };
    use chrono::Utc;
    use ratatui::layout::Rect;
    use std::time::{Duration as StdDuration, Instant};

    const TEST_MESSAGE_AREA_WIDTH: u16 = 80;
    const TEST_VISIBLE_ROW_MESSAGE_AREA_WIDTH: u16 = 40;
    const TEST_SHORT_MESSAGE_AREA_HEIGHT: u16 = 5;
    const TEST_PAGED_MESSAGE_AREA_HEIGHT: u16 = 6;
    const TEST_TALL_MESSAGE_AREA_HEIGHT: u16 = 20;
    const TEST_BLANK_ROW_MESSAGE_AREA_HEIGHT: u16 = 8;
    const TEST_FOLDER_AREA_HEIGHT: u16 = 3;
    const TEST_NARROW_FOLDER_AREA_WIDTH: u16 = 14;
    const TEST_UNREAD_FOLDER_AREA_WIDTH: u16 = 15;
    const TEST_WIDE_FOLDER_AREA_WIDTH: u16 = 40;
    const TEST_EXPANDED_FOLDER_AREA_WIDTH: u16 = 80;
    const TEST_INPUT_AREA_HEIGHT: u16 = 3;
    const TEST_NARROW_INPUT_AREA_WIDTH: u16 = 8;
    const TEST_EXPANDED_INPUT_AREA_WIDTH: u16 = 20;
    const TEST_CHAT_AREA_WIDTH: u16 = 40;
    const TEST_SHORT_CHAT_AREA_HEIGHT: u16 = 6;
    const TEST_TALL_CHAT_AREA_HEIGHT: u16 = 20;

    fn message_area(height: u16) -> Rect {
        message_area_with_width(TEST_MESSAGE_AREA_WIDTH, height)
    }

    fn message_area_with_width(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    fn folder_area(width: u16) -> Rect {
        Rect::new(0, 0, width, TEST_FOLDER_AREA_HEIGHT)
    }

    fn thread_topic(id: i32, title: &str) -> ThreadTopic {
        ThreadTopic {
            id,
            title: title.to_string(),
            top_message_id: id,
            unread_count: 0,
            is_closed: false,
            is_pinned: false,
        }
    }

    fn input_area(width: u16) -> Rect {
        Rect::new(0, 0, width, TEST_INPUT_AREA_HEIGHT)
    }

    fn chat_area(height: u16) -> Rect {
        Rect::new(0, 0, TEST_CHAT_AREA_WIDTH, height)
    }

    fn chat(id: i64, name: &str) -> Chat {
        chat_with_unread(id, name, 0, None)
    }

    fn chat_with_unread(id: i64, name: &str, unread_count: usize, folder_id: Option<i32>) -> Chat {
        Chat {
            id,
            name: name.to_string(),
            last_message: None,
            unread_count,
            is_group: false,
            folder_id,
        }
    }

    fn folder(id: i32, name: &str, unread_count: usize) -> Folder {
        Folder {
            id,
            name: name.to_string(),
            unread_count,
        }
    }

    fn message(id: i32) -> Message {
        Message {
            id,
            chat_id: 10,
            thread_topic_id: None,
            sender_identity: None,
            sender_name: "Alice".to_string(),
            content: format!("message {}", id),
            timestamp: Utc::now(),
            is_own: false,
            is_edited: false,
            reply_to_content: None,
            media: None,
            status: MessageStatus::Delivered,
            can_edit: false,
            can_delete: false,
            error: None,
        }
    }

    fn update_message(id: i32, chat_id: i64, content: &str, is_own: bool) -> Message {
        Message {
            id,
            chat_id,
            thread_topic_id: None,
            sender_identity: Some(SenderIdentity::User(if is_own { 0 } else { 1 })),
            sender_name: if is_own { OWN_SENDER_NAME } else { "Alice" }.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            is_own,
            is_edited: false,
            reply_to_content: None,
            media: None,
            status: MessageStatus::Delivered,
            can_edit: is_own,
            can_delete: is_own,
            error: None,
        }
    }

    fn typing_update(
        chat_id: i64,
        topic_id: Option<i32>,
        user_id: i64,
        user_name: &str,
        is_typing: bool,
    ) -> Update {
        Update::TypingStatus {
            chat_id,
            topic_id,
            sender_identity: SenderIdentity::User(user_id),
            user_name: user_name.to_string(),
            is_typing,
        }
    }

    fn state_with_chats() -> AppState {
        let mut state = AppState::new();
        state.chats = vec![chat(10, "Alice"), chat(20, "Bob")];
        state
    }

    fn state_with_many_chats() -> AppState {
        let mut state = AppState::new();
        state.chats_area = chat_area(TEST_SHORT_CHAT_AREA_HEIGHT);
        state.chats = (0..8)
            .map(|idx| chat(100 + idx, &format!("Chat {}", idx)))
            .collect();
        state
    }

    #[test]
    fn viewport_capacity_constants_are_explicit() {
        assert_eq!(PANEL_BORDER_RESERVED_COLUMNS, 2);
        assert_eq!(PANEL_BORDER_RESERVED_ROWS, 2);
        assert_eq!(CHAT_LIST_ITEM_HEIGHT, 2);
        assert_eq!(FOLDER_VIEWPORT_RESERVED_COLUMNS, 4);
        assert_eq!(MESSAGE_ROW_HEIGHT, 1);
        assert_eq!(REPLY_MESSAGE_ROW_HEIGHT, 2);
    }

    #[test]
    fn message_visible_row_height_accounts_for_reply_preview_rows_and_wrapping() {
        let plain = message(10);
        let mut reply = message(20);
        reply.reply_to_content = Some("quoted".to_string());
        let mut long = message(30);
        long.content = "abcdefghijklmnopqrstuvwxyz".to_string();

        assert_eq!(message_visible_row_height(&plain), MESSAGE_ROW_HEIGHT);
        assert_eq!(message_visible_row_height(&reply), REPLY_MESSAGE_ROW_HEIGHT);
        assert!(message_visible_row_height_for_width(&long, 20) > MESSAGE_ROW_HEIGHT);
    }

    #[test]
    fn last_index_saturates_empty_counts_for_collection_and_viewport_tails() {
        assert_eq!(last_index(0), 0);
        assert_eq!(last_index(1), 0);
        assert_eq!(last_index(3), 2);
    }

    #[test]
    fn visible_folders_use_display_width_for_wide_names() {
        let mut state = AppState::new();
        state.folders_area = folder_area(TEST_NARROW_FOLDER_AREA_WIDTH);
        state.folders = vec![folder(1, "好好好", 0), folder(2, "Later", 0)];

        let (visible, has_left, has_right) = state.get_visible_folders();

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "好好好");
        assert!(!has_left);
        assert!(has_right);
    }

    #[test]
    fn visible_folders_account_for_unread_suffix_width() {
        let mut state = AppState::new();
        state.folders_area = folder_area(TEST_UNREAD_FOLDER_AREA_WIDTH);
        state.folders = vec![folder(1, "好", 12), folder(2, "Later", 0)];

        let (visible, _, has_right) = state.get_visible_folders();

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].unread_count, 12);
        assert!(has_right);
    }

    #[test]
    fn folder_selection_clamps_scroll_offset_when_viewport_grows() {
        let mut state = AppState::new();
        state.folders_area = folder_area(TEST_NARROW_FOLDER_AREA_WIDTH);
        state.folders = vec![all_folder(0), folder(2, "Work", 0), folder(3, "Later", 0)];

        state.select_folder(2);
        assert_eq!(state.selected_folder_index, 2);
        assert_eq!(state.folder_scroll_offset, 2);

        state.folders_area = folder_area(TEST_EXPANDED_FOLDER_AREA_WIDTH);
        state.ensure_selected_folder_visible();

        assert_eq!(state.selected_folder_index, 2);
        assert_eq!(state.folder_scroll_offset, 0);
    }

    #[test]
    fn folder_index_at_visible_column_matches_rendered_label_widths() {
        let mut state = AppState::new();
        state.folders_area = folder_area(TEST_WIDE_FOLDER_AREA_WIDTH);
        state.folders = vec![folder(1, "好", 0), folder(2, "Work", 0)];

        assert_eq!(state.folder_index_at_visible_column(0), Some(0));
        assert_eq!(state.folder_index_at_visible_column(3), Some(0));
        assert_eq!(state.folder_index_at_visible_column(4), None);
        assert_eq!(state.folder_index_at_visible_column(6), None);
        assert_eq!(state.folder_index_at_visible_column(7), Some(1));
    }

    #[test]
    fn folder_index_at_visible_column_ignores_scroll_indicators() {
        let mut state = AppState::new();
        state.folders_area = folder_area(TEST_WIDE_FOLDER_AREA_WIDTH);
        state.folders = vec![all_folder(0), folder(2, "好", 0), folder(3, "Work", 0)];
        state.folder_scroll_offset = 1;

        assert_eq!(state.folder_index_at_visible_column(0), None);
        assert_eq!(state.folder_index_at_visible_column(1), None);
        assert_eq!(state.folder_index_at_visible_column(2), Some(1));
    }

    #[test]
    fn visible_thread_topics_follow_selected_topic_window() {
        let mut state = AppState::new();
        state.thread_topics_area = folder_area(TEST_NARROW_FOLDER_AREA_WIDTH);
        state.thread_topics = vec![
            thread_topic(101, "One"),
            thread_topic(102, "Two"),
            thread_topic(103, "Three"),
        ];

        state.select_thread_topic_at(2);
        let (visible, has_left, has_right) = state.get_visible_thread_topics();

        assert_eq!(state.thread_topic_scroll_offset, 2);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].title, "Three");
        assert!(has_left);
        assert!(!has_right);
    }

    #[test]
    fn thread_topic_index_at_visible_column_uses_scrolled_window() {
        let mut state = AppState::new();
        state.thread_topics_area = folder_area(TEST_WIDE_FOLDER_AREA_WIDTH);
        state.thread_topics = vec![
            thread_topic(101, "One"),
            thread_topic(102, "好"),
            thread_topic(103, "Three"),
        ];
        state.thread_topic_scroll_offset = 1;

        assert_eq!(state.thread_topic_index_at_visible_column(0), None);
        assert_eq!(state.thread_topic_index_at_visible_column(1), None);
        assert_eq!(state.thread_topic_index_at_visible_column(2), Some(1));
    }

    #[test]
    fn incoming_message_in_active_chat_does_not_increment_unread() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(99), folder(2, "Personal", 99)];
        state.chats = vec![
            chat_with_unread(1, "Chat 1", 3, Some(2)),
            chat_with_unread(2, "Chat 2", 0, Some(2)),
        ];
        state.selected_chat_index = 0;

        let presented = state.apply_update(Update::NewMessage(update_message(
            10,
            1,
            "open chat",
            false,
        )));

        assert_eq!(
            presented,
            Some(PresentedIncomingMessage {
                chat_id: 1,
                topic_id: None,
                message_id: 10,
            })
        );
        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(state.folders[0].unread_count, 99);
        assert_eq!(state.folders[1].unread_count, 99);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "open chat");
        assert_eq!(state.chats[0].last_message.as_deref(), Some("open chat"));
    }

    #[test]
    fn unfocused_incoming_message_is_retained_unread_and_not_presented() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(4)];
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, None)];
        state.set_terminal_focused(false);

        let presented = state.apply_update(Update::NewMessage(update_message(
            10,
            1,
            "unseen while away",
            false,
        )));

        assert_eq!(presented, None);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.chats[0].unread_count, 1);
        assert_eq!(state.folders[0].unread_count, 4);
        assert!(!state.terminal_focused());
    }

    #[test]
    fn incoming_message_while_reading_older_is_unread_and_not_presented() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, None)];
        state.messages = vec![
            update_message(10, 1, "older", false),
            update_message(20, 1, "tail", false),
        ];
        state.selected_message_index = 0;

        let presented = state.apply_update(Update::NewMessage(update_message(
            30,
            1,
            "new unread",
            false,
        )));

        assert_eq!(presented, None);
        assert_eq!(state.selected_message().map(|message| message.id), Some(10));
        assert!(state.messages.iter().any(|message| message.id == 30));
        assert_eq!(state.chats[0].unread_count, 1);
    }

    #[test]
    fn incoming_message_across_newer_gap_is_omitted_and_unread() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, None)];
        state.messages = vec![update_message(10, 1, "older", false)];
        state.newer_history_gap = true;

        let presented =
            state.apply_update(Update::NewMessage(update_message(30, 1, "hidden", false)));

        assert_eq!(presented, None);
        assert!(!state.messages.iter().any(|message| message.id == 30));
        assert_eq!(state.chats[0].unread_count, 1);
        assert!(state.newer_history_gap());
    }

    #[test]
    fn own_message_across_newer_gap_updates_preview_without_body_or_unread() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(9)];
        state.chats = vec![chat_with_unread(1, "Chat 1", 4, None)];
        state.messages = vec![update_message(10, 1, "older", false)];
        state.newer_history_gap = true;
        let own = update_message(30, 1, "sent from phone", true);

        let presented = state.apply_update(Update::NewMessage(own));

        assert_eq!(presented, None);
        assert!(!state.messages.iter().any(|message| message.id == 30));
        assert_eq!(
            state.chats[0].last_message.as_deref(),
            Some("sent from phone")
        );
        assert_eq!(state.chats[0].unread_count, 4);
        assert_eq!(state.folders[0].unread_count, 9);
        assert!(state.newer_history_gap());
    }

    #[test]
    fn retained_own_tail_update_keeps_following_incoming_presented() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, None)];
        state.messages = vec![update_message(10, 1, "current tail", false)];
        state.selected_message_index = 0;

        let own = state.apply_update(Update::NewMessage(update_message(
            11,
            1,
            "sent from phone",
            true,
        )));
        assert_eq!(own, None);
        assert_eq!(state.selected_message().map(|message| message.id), Some(11));
        assert_eq!(state.chats[0].unread_count, 0);

        let incoming = state.apply_update(Update::NewMessage(update_message(
            12,
            1,
            "next incoming",
            false,
        )));
        assert_eq!(
            incoming,
            Some(PresentedIncomingMessage {
                chat_id: 1,
                topic_id: None,
                message_id: 12,
            })
        );
        assert_eq!(state.selected_message().map(|message| message.id), Some(12));
        assert_eq!(state.chats[0].unread_count, 0);
    }

    #[test]
    fn stale_incoming_message_for_active_chat_does_not_append_or_replace_preview() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0), folder(2, "Personal", 0)];
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.chats[0].last_message = Some("latest loaded".to_string());
        state.messages = vec![
            update_message(20, 1, "older loaded", false),
            update_message(30, 1, "latest loaded", false),
        ];

        state.apply_update(Update::NewMessage(update_message(
            25,
            1,
            "hours old catch-up",
            false,
        )));

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].content, "latest loaded");
        assert_eq!(
            state.chats[0].last_message.as_deref(),
            Some("latest loaded")
        );
        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(state.folders[0].unread_count, 0);
        assert_eq!(state.folders[1].unread_count, 0);
    }

    #[test]
    fn incoming_message_clears_only_matching_sender_typing_identity() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.apply_update(typing_update(1, None, 1, "Same name", true));
        state.apply_update(typing_update(1, None, 2, "Same name", true));

        state.apply_update(Update::NewMessage(update_message(
            10,
            1,
            "open chat",
            false,
        )));

        assert_eq!(
            state.selected_typing_users(),
            Some(vec!["Same name".to_string()])
        );
    }

    #[test]
    fn topic_typing_status_is_scoped_to_selected_topic() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Forum", 0, None)];
        state.apply_loaded_selected_chat_thread_topics(vec![ThreadTopic {
            id: 101,
            title: "General".to_string(),
            top_message_id: 1001,
            unread_count: 0,
            is_closed: false,
            is_pinned: false,
        }]);

        state.apply_update(typing_update(1, Some(101), 1, "Alice", true));
        state.apply_update(typing_update(1, None, 2, "Bob", true));

        assert_eq!(
            state.selected_typing_users(),
            Some(vec!["Alice".to_string()])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn typing_activity_keeps_stable_identities_refreshes_labels_and_expires() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat", 0, None)];
        state.apply_update(typing_update(1, None, 1, "Same", true));
        state.apply_update(typing_update(1, None, 2, "Same", true));
        assert_eq!(
            state.selected_typing_users(),
            Some(vec!["Same".to_string(), "Same".to_string()])
        );

        tokio::time::advance(StdDuration::from_secs(1)).await;
        state.apply_update(typing_update(1, None, 1, "Renamed", true));
        state.apply_update(typing_update(1, None, 2, "Same", false));
        assert_eq!(
            state.selected_typing_users(),
            Some(vec!["Renamed".to_string()])
        );
        assert_eq!(
            state.notification_deadline(),
            Some(tokio::time::Instant::now() + TYPING_ACTIVITY_LIFETIME)
        );

        tokio::time::advance(TYPING_ACTIVITY_LIFETIME).await;
        assert!(state.check_notification_timeout());
        assert_eq!(state.selected_typing_users(), None);
    }

    #[test]
    fn unresolved_forum_scope_never_presents_topic_message_as_chat_wide() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat(1, "Forum")];
        state.begin_conversation_load();
        let mut topic_message = update_message(12, 1, "unresolved topic", false);
        topic_message.thread_topic_id = Some(101);

        let presented = state.apply_update(Update::NewMessage(topic_message));

        assert_eq!(presented, None);
        assert!(state.messages.is_empty());
        assert_eq!(state.chats[0].unread_count, 1);
    }

    #[test]
    fn incoming_message_for_other_thread_does_not_append_to_selected_thread_view() {
        let mut state = AppState::new();
        state.chats = vec![chat(1, "Forum")];
        state.apply_loaded_selected_chat_thread_topics(vec![
            thread_topic(101, "General"),
            thread_topic(102, "Deployments"),
        ]);
        let mut selected_topic_message = update_message(20, 1, "selected topic", false);
        selected_topic_message.thread_topic_id = Some(101);
        state.apply_loaded_selected_chat_messages(vec![selected_topic_message]);

        let mut other_topic_message = update_message(12, 1, "other topic", false);
        other_topic_message.thread_topic_id = Some(102);
        state.apply_update(Update::NewMessage(other_topic_message));

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "selected topic");
        assert_eq!(state.chats[0].last_message.as_deref(), Some("other topic"));
        assert_eq!(state.chats[0].unread_count, 1);
        assert_eq!(state.thread_topics[1].unread_count, 1);
    }

    #[test]
    fn incoming_message_for_selected_thread_appends_to_selected_thread_view() {
        let mut state = AppState::new();
        state.chats = vec![chat(1, "Forum")];
        state.apply_loaded_selected_chat_thread_topics(vec![thread_topic(101, "General")]);
        state.chats[0].unread_count = 3;
        state.thread_topics[0].unread_count = 3;

        let mut topic_message = update_message(12, 1, "selected topic update", false);
        topic_message.thread_topic_id = Some(101);
        let presented = state.apply_update(Update::NewMessage(topic_message));

        assert_eq!(
            presented,
            Some(PresentedIncomingMessage {
                chat_id: 1,
                topic_id: Some(101),
                message_id: 12,
            })
        );
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "selected topic update");
        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(state.thread_topics[0].unread_count, 0);
    }

    #[test]
    fn loading_selected_thread_messages_clears_only_selected_topic_unread() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(7)];
        state.chats = vec![chat_with_unread(1, "Forum", 7, None)];
        state.chats[0].last_message = Some("server-wide preview".to_string());
        state.apply_loaded_selected_chat_thread_topics(vec![
            thread_topic(101, "General"),
            thread_topic(102, "Deployments"),
        ]);
        state.thread_topics[0].unread_count = 4;
        state.thread_topics[1].unread_count = 3;

        let mut topic_message = update_message(12, 1, "selected topic history", false);
        topic_message.thread_topic_id = Some(101);
        state.apply_loaded_selected_chat_messages(vec![topic_message]);

        assert_eq!(state.chats[0].unread_count, 3);
        assert_eq!(state.folders[0].unread_count, 7);
        assert_eq!(
            state.chats[0].last_message.as_deref(),
            Some("server-wide preview")
        );
        assert_eq!(state.thread_topics[0].unread_count, 0);
        assert_eq!(state.thread_topics[1].unread_count, 3);
    }

    #[test]
    fn incoming_topic_message_clears_only_matching_scope_and_identity() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Forum", 0, None)];
        state.apply_loaded_selected_chat_thread_topics(vec![
            thread_topic(101, "General"),
            thread_topic(102, "Deployments"),
        ]);
        state.apply_update(typing_update(1, Some(101), 1, "Alice", true));
        state.apply_update(typing_update(1, Some(102), 1, "Alice", true));

        let mut topic_message = update_message(10, 1, "topic message", false);
        topic_message.thread_topic_id = Some(102);
        state.apply_update(Update::NewMessage(topic_message));

        state.select_thread_topic_at(0);
        assert_eq!(
            state.selected_typing_users(),
            Some(vec!["Alice".to_string()])
        );
        state.select_thread_topic_at(1);
        assert_eq!(state.selected_typing_users(), None);
    }

    #[test]
    fn incoming_message_in_background_chat_increments_unread() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(99), folder(2, "Personal", 99)];
        state.chats = vec![
            chat_with_unread(1, "Chat 1", 0, Some(2)),
            chat_with_unread(2, "Chat 2", 4, Some(2)),
        ];
        state.selected_chat_index = 0;

        state.apply_update(Update::NewMessage(update_message(
            11,
            2,
            "background",
            false,
        )));

        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(state.chats[1].unread_count, 5);
        assert_eq!(state.folders[0].unread_count, 99);
        assert_eq!(state.folders[1].unread_count, 99);
        assert!(state.messages.is_empty());
        assert_eq!(state.chats[1].last_message.as_deref(), Some("background"));
    }

    #[test]
    fn background_own_message_updates_preview_without_unread_or_selection_yank() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(99)];
        state.chats = vec![
            chat_with_unread(1, "Chat 1", 0, None),
            chat_with_unread(2, "Chat 2", 4, None),
        ];
        state.selected_chat_index = 0;
        let own = update_message(12, 2, "sent from phone", true);

        assert_eq!(state.apply_update(Update::NewMessage(own)), None);

        assert_eq!(state.selected_chat_id(), Some(1));
        assert!(state.messages.is_empty());
        assert_eq!(state.chats[1].unread_count, 4);
        assert_eq!(
            state.chats[1].last_message.as_deref(),
            Some("sent from phone")
        );
        assert_eq!(state.folders[0].unread_count, 99);
    }

    #[test]
    fn read_outgoing_update_marks_loaded_own_messages_read() {
        let mut state = AppState::new();
        state.messages = vec![
            update_message(10, 1, "old own", true),
            update_message(20, 1, "new own", true),
            update_message(15, 1, "incoming", false),
            update_message(10, 2, "other chat", true),
        ];
        state.messages[1].status = MessageStatus::Sending;
        state.messages[3].status = MessageStatus::Sent;

        state.apply_update(Update::ReadOutgoingMessages {
            chat_id: 1,
            max_message_id: 15,
        });

        assert_eq!(state.messages[0].status, MessageStatus::Read);
        assert_eq!(state.messages[1].status, MessageStatus::Sending);
        assert_eq!(state.messages[2].status, MessageStatus::Delivered);
        assert_eq!(state.messages[3].status, MessageStatus::Sent);
    }

    #[test]
    fn incoming_media_message_preview_keeps_photo_visible_without_caption() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, None)];
        state.selected_chat_index = 0;
        let mut update = update_message(11, 1, "", false);
        update.media = Some(MessageMedia::photo());

        state.apply_update(Update::NewMessage(update));

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.chats[0].last_message.as_deref(), Some("[photo]"));
    }

    #[test]
    fn edit_update_warns_when_current_edit_target_changes_remotely() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.messages = vec![update_message(21, 1, "old remote", true)];
        state.enter_edit_mode(21, "local edit in progress".to_string());

        state.apply_update(Update::EditMessage {
            chat_id: 1,
            message_id: 21,
            new_content: "new remote".to_string(),
        });

        assert_eq!(state.messages[0].content, "new remote");
        assert!(state.messages[0].is_edited);
        assert_eq!(state.chats[0].last_message.as_deref(), Some("new remote"));
        assert_eq!(state.input_buffer, "local edit in progress");
        assert_eq!(
            state.status_message.as_deref(),
            Some(REMOTE_EDIT_WHILE_EDITING_STATUS)
        );
    }

    #[test]
    fn edit_update_does_not_warn_for_unrelated_local_edit() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.messages = vec![
            update_message(21, 1, "local target", true),
            update_message(22, 1, "remote target", true),
        ];
        state.enter_edit_mode(21, "local edit in progress".to_string());

        state.apply_update(Update::EditMessage {
            chat_id: 1,
            message_id: 22,
            new_content: "new remote".to_string(),
        });

        assert_eq!(state.messages[1].content, "new remote");
        assert_eq!(state.chats[0].last_message.as_deref(), Some("new remote"));
        assert!(state.status_message.is_none());
        assert_eq!(state.editing_message_id, Some(21));
    }

    #[test]
    fn delete_update_wildcard_matches_any_loaded_chat() {
        assert!(delete_update_matches_chat(UNKNOWN_DELETE_UPDATE_CHAT_ID, 1));
        assert!(delete_update_matches_chat(UNKNOWN_DELETE_UPDATE_CHAT_ID, 2));
        assert!(delete_update_matches_chat(1, 1));
        assert!(!delete_update_matches_chat(1, 2));
    }

    #[test]
    fn error_update_sets_error_banner_state() {
        let mut state = AppState::new();
        state.set_status("Connected");

        state.apply_update(Update::Error("Update error: network down".to_string()));

        assert_eq!(
            state.error_message.as_deref(),
            Some("Update error: network down")
        );
        assert!(state.status_message.is_none());
    }

    #[test]
    fn delete_update_refreshes_selected_chat_preview_from_remaining_loaded_messages() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.messages = vec![
            update_message(20, 1, "previous", false),
            update_message(21, 1, "delete me", true),
        ];
        state.chats[0].last_message = Some("delete me".to_string());
        state.selected_message_index = 1;

        state.apply_update(Update::DeleteMessage {
            chat_id: 1,
            message_id: 21,
        });

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.chats[0].last_message.as_deref(), Some("previous"));
        assert_eq!(state.selected_message_index, 0);
    }

    #[test]
    fn delete_update_clears_selected_chat_preview_when_no_loaded_messages_remain() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.messages = vec![update_message(21, 1, "delete me", true)];
        state.chats[0].last_message = Some("delete me".to_string());

        state.apply_update(Update::DeleteMessage {
            chat_id: 1,
            message_id: 21,
        });

        assert!(state.messages.is_empty());
        assert_eq!(state.chats[0].last_message, None);
    }

    #[test]
    fn delete_update_clears_matching_delete_confirmation() {
        let mut state = AppState::new();
        state.messages = vec![update_message(21, 1, "delete me", true)];
        state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 1,
            message_id: 21,
        });

        state.apply_update(Update::DeleteMessage {
            chat_id: 1,
            message_id: 21,
        });

        assert!(state.messages.is_empty());
        assert!(state.delete_confirmation().is_none());
    }

    #[test]
    fn delete_update_keeps_unrelated_delete_confirmation() {
        let mut state = AppState::new();
        state.messages = vec![
            update_message(21, 1, "delete me", true),
            update_message(22, 2, "other chat", true),
        ];
        state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 2,
            message_id: 22,
        });

        state.apply_update(Update::DeleteMessage {
            chat_id: 1,
            message_id: 21,
        });

        assert_eq!(state.messages.len(), 1);
        assert_eq!(
            state.delete_confirmation(),
            Some(DeleteConfirmation {
                chat_id: 2,
                message_id: 22,
            })
        );
    }

    #[test]
    fn delete_update_clears_matching_edit_compose_mode_and_restores_draft() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.messages = vec![update_message(21, 1, "delete me", true)];
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();
        state.enter_edit_mode(21, "edited text".to_string());

        state.apply_update(Update::DeleteMessage {
            chat_id: 1,
            message_id: 21,
        });

        assert!(state.editing_message_id.is_none());
        assert!(state.replying_to_message_id.is_none());
        assert_eq!(state.input_buffer, "plain draft");
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
    }

    #[test]
    fn delete_update_clears_matching_reply_compose_mode_and_restores_draft() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.messages = vec![update_message(21, 1, "target", false)];
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();
        state.enter_reply_mode(21);
        state.input_buffer = "reply in progress".to_string();

        state.apply_update(Update::DeleteMessage {
            chat_id: 1,
            message_id: 21,
        });

        assert!(state.editing_message_id.is_none());
        assert!(state.replying_to_message_id.is_none());
        assert_eq!(state.input_buffer, "plain draft");
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
    }

    #[test]
    fn delete_update_keeps_unrelated_reply_compose_mode() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat 1", 0, Some(2))];
        state.messages = vec![update_message(21, 1, "target", false)];
        state.enter_reply_mode(21);
        state.input_buffer = "reply in progress".to_string();

        state.apply_update(Update::DeleteMessage {
            chat_id: 2,
            message_id: 21,
        });

        assert_eq!(state.replying_to_message_id, Some(21));
        assert_eq!(state.input_buffer, "reply in progress");
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    #[test]
    fn saves_restores_and_discards_per_chat_drafts() {
        let mut state = state_with_chats();

        state.input_buffer = "alice draft".to_string();
        state.save_current_draft();
        state.select_chat(1);
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, "");

        state.input_buffer = "bob draft".to_string();
        state.save_current_draft();
        state.select_chat(0);
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, "alice draft");

        state.select_chat(1);
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, "bob draft");

        state.discard_draft_for_selected_chat();
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, "");
    }

    #[test]
    fn topic_switches_save_and_restore_exact_scope_drafts() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Forum", 0, None)];
        state.thread_topics = vec![
            thread_topic(101, "General"),
            thread_topic(102, "Deployments"),
        ];
        state.input_buffer = "general draft".to_string();

        state.select_next_thread_topic();
        assert_eq!(state.input_buffer, "");
        state.input_buffer = "deploy draft".to_string();
        state.select_prev_thread_topic();
        assert_eq!(state.input_buffer, "general draft");
        state.select_thread_topic_at(1);
        assert_eq!(state.input_buffer, "deploy draft");
        state.cancel_input_mode();
        state.select_thread_topic_at(0);
        state.select_thread_topic_at(1);
        assert_eq!(state.input_buffer, "");
    }

    #[test]
    fn failed_submission_recovery_is_scope_exact_and_submission_ordered() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Forum", 0, None)];
        state.thread_topics = vec![
            thread_topic(101, "General"),
            thread_topic(102, "Deployments"),
        ];
        state.selected_thread_topic_index = 1;
        state.input_buffer = "sibling draft".to_string();
        state.save_current_draft();
        state.register_mutation_submission(1, 1, Some(101));
        state.register_mutation_submission(2, 1, Some(101));

        state.recover_failed_submission(2, 1, Some(101), "second failed".to_string());
        state.recover_failed_submission(1, 1, Some(101), "first failed".to_string());

        assert_eq!(state.input_buffer, "sibling draft");
        assert_eq!(state.pending_mutation_submission_count(), 0);

        state.select_thread_topic_at(0);
        assert_eq!(state.input_buffer, "first failed\n\nsecond failed");

        state.select_thread_topic_at(1);
        assert_eq!(state.input_buffer, "sibling draft");

        state.select_thread_topic_at(0);
        assert_eq!(state.input_buffer, "first failed\n\nsecond failed");
    }

    #[test]
    fn equal_failed_content_keeps_earliest_submission_order_once() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat", 0, None)];
        state.input_buffer = "newer text".to_string();
        for submission_id in [10, 20, 30] {
            state.register_mutation_submission(submission_id, 1, None);
        }

        state.recover_failed_submission(30, 1, None, "retry once".to_string());
        state.recover_failed_submission(10, 1, None, "retry once".to_string());
        state.recover_failed_submission(20, 1, None, "different failure".to_string());

        let expected = "retry once\n\ndifferent failure\n\nnewer text";
        assert_eq!(state.input_buffer, expected);
        assert_eq!(state.input_buffer.matches("retry once").count(), 1);
        assert_eq!(state.pending_mutation_submission_count(), 0);

        state.save_current_draft();
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, expected);
    }

    #[test]
    fn current_scope_failure_preserves_newer_input_without_duplication() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(1, "Chat", 0, None)];
        state.input_buffer = "newer text".to_string();
        state.register_mutation_submission(7, 1, None);

        state.recover_failed_submission(7, 1, None, "failed text".to_string());
        state.recover_failed_submission(7, 1, None, "failed text".to_string());

        assert_eq!(state.input_buffer, "failed text\n\nnewer text");
        assert_eq!(state.pending_mutation_submission_count(), 0);
    }

    #[test]
    fn draft_helpers_do_not_overwrite_while_editing_or_replying() {
        let mut state = state_with_chats();
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();

        state.enter_edit_mode(42, "edited text".to_string());
        state.save_current_draft();
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, "edited text");

        state.clear_input_mode();
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, "plain draft");

        state.enter_reply_mode(42);
        state.input_buffer = "reply text".to_string();
        state.save_current_draft();
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, "reply text");

        state.clear_input_mode();
        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_buffer, "plain draft");
    }

    #[test]
    fn input_cursor_edits_text_at_cursor_and_handles_unicode() {
        let mut state = AppState::new();

        state.insert_input_char('h');
        state.insert_input_char('é');
        state.insert_input_char('!');
        state.move_input_cursor_left();
        state.insert_input_char('y');
        state.backspace_input_char();
        state.move_input_cursor_left();
        state.delete_input_char();

        assert_eq!(state.input_buffer, "h!");
        assert_eq!(state.input_cursor(), 1);
    }

    #[test]
    fn input_cursor_treats_combining_mark_sequence_as_one_grapheme() {
        let mut state = AppState::new();

        state.insert_input_char('e');
        state.insert_input_char('\u{301}');
        state.insert_input_char('x');
        state.move_input_cursor_left();
        state.backspace_input_char();

        assert_eq!(state.input_buffer, "x");
        assert_eq!(state.input_cursor(), 0);
    }

    #[test]
    fn input_line_deletion_removes_text_before_or_after_cursor() {
        let mut state = AppState::new();
        state.input_buffer = "a好bc".to_string();
        state.move_input_cursor_to_end();
        state.move_input_cursor_left();

        state.delete_input_before_cursor();
        assert_eq!(state.input_buffer, "c");
        assert_eq!(state.input_cursor(), 0);

        state.input_buffer = "a好bc".to_string();
        state.move_input_cursor_to_end();
        state.move_input_cursor_left();
        state.delete_input_after_cursor();

        assert_eq!(state.input_buffer, "a好b");
        assert_eq!(state.input_cursor(), 3);
    }

    #[test]
    fn input_previous_word_deletion_uses_unicode_word_boundaries() {
        let mut state = AppState::new();
        state.input_buffer = "hello 好team  now".to_string();
        state.move_input_cursor_to_end();

        state.delete_input_previous_word();
        assert_eq!(state.input_buffer, "hello 好team  ");

        state.delete_input_previous_word();
        assert_eq!(state.input_buffer, "hello 好");
        assert_eq!(state.input_cursor(), 7);

        state.delete_input_previous_word();
        assert_eq!(state.input_buffer, "hello ");
        assert_eq!(state.input_cursor(), 6);
    }

    #[test]
    fn input_previous_word_deletion_treats_punctuation_as_boundary() {
        let mut state = AppState::new();
        state.input_buffer = "hello, ".to_string();
        state.move_input_cursor_to_end();

        state.delete_input_previous_word();
        assert_eq!(state.input_buffer, "hello");
        assert_eq!(state.input_cursor(), 5);

        state.delete_input_previous_word();
        assert!(state.input_buffer.is_empty());
        assert_eq!(state.input_cursor(), 0);
    }

    #[test]
    fn input_previous_word_deletion_keeps_unicode_word_sequences_together() {
        let mut state = AppState::new();
        state.input_buffer = "say can't 32.3 мир  ".to_string();
        state.move_input_cursor_to_end();

        state.delete_input_previous_word();
        assert_eq!(state.input_buffer, "say can't 32.3 ");

        state.delete_input_previous_word();
        assert_eq!(state.input_buffer, "say can't ");

        state.delete_input_previous_word();
        assert_eq!(state.input_buffer, "say ");
    }

    #[test]
    fn input_cursor_moves_home_end_and_clamps_to_buffer() {
        let mut state = AppState::new();
        state.input_buffer = "abc".to_string();
        state.input_cursor = 99;

        state.move_input_cursor_left();
        assert_eq!(state.input_cursor(), 2);

        state.move_input_cursor_to_start();
        state.move_input_cursor_left();
        assert_eq!(state.input_cursor(), 0);

        state.move_input_cursor_to_end();
        state.move_input_cursor_right();
        assert_eq!(state.input_cursor(), 3);
    }

    #[test]
    fn long_input_scrolls_to_keep_cursor_visible() {
        let mut state = AppState::new();
        state.input_area = input_area(TEST_NARROW_INPUT_AREA_WIDTH);

        for c in "abcdefgh".chars() {
            state.insert_input_char(c);
        }

        assert_eq!(state.input_cursor(), 8);
        assert_eq!(state.effective_input_scroll_offset(), 3);
        assert_eq!(state.visible_input_text(), "defgh");
        assert_eq!(state.visible_input_cursor_column(), 5);

        for _ in 0..6 {
            state.move_input_cursor_left();
        }

        assert_eq!(state.input_cursor(), 2);
        assert_eq!(state.effective_input_scroll_offset(), 2);
        assert_eq!(state.visible_input_text(), "cdefgh");
        assert_eq!(state.visible_input_cursor_column(), 0);
    }

    #[test]
    fn input_scroll_offset_clamps_when_viewport_grows() {
        let mut state = AppState::new();
        state.input_area = input_area(TEST_NARROW_INPUT_AREA_WIDTH);

        for c in "abcdefgh".chars() {
            state.insert_input_char(c);
        }

        assert_eq!(state.effective_input_scroll_offset(), 3);
        assert_eq!(state.visible_input_text(), "defgh");

        state.input_area = input_area(TEST_EXPANDED_INPUT_AREA_WIDTH);
        state.ensure_input_cursor_visible();

        assert_eq!(state.effective_input_scroll_offset(), 0);
        assert_eq!(state.visible_input_text(), "abcdefgh");
        assert_eq!(state.visible_input_cursor_column(), 8);
    }

    #[test]
    fn wide_input_scrolls_by_display_width_not_character_count() {
        let mut state = AppState::new();
        state.input_area = input_area(TEST_NARROW_INPUT_AREA_WIDTH);

        for c in "好好好好".chars() {
            state.insert_input_char(c);
        }

        assert_eq!(state.input_cursor(), 4);
        assert_eq!(state.effective_input_scroll_offset(), 2);
        assert_eq!(state.visible_input_text(), "好好");
        assert_eq!(state.visible_input_cursor_column(), 4);

        for _ in 0..3 {
            state.move_input_cursor_left();
        }

        assert_eq!(state.input_cursor(), 1);
        assert_eq!(state.effective_input_scroll_offset(), 1);
        assert_eq!(state.visible_input_text(), "好好好");
        assert_eq!(state.visible_input_cursor_column(), 0);
    }

    #[test]
    fn input_cursor_moves_to_clicked_visible_display_column() {
        let mut state = AppState::new();
        state.input_area = input_area(TEST_NARROW_INPUT_AREA_WIDTH);
        state.input_buffer = "a好b".to_string();

        state.move_input_cursor_to_visible_column(3);
        assert_eq!(state.input_cursor(), 2);

        state.insert_input_char('X');
        assert_eq!(state.input_buffer, "a好Xb");
    }

    #[test]
    fn input_cursor_click_uses_current_scroll_offset() {
        let mut state = AppState::new();
        state.input_area = input_area(TEST_NARROW_INPUT_AREA_WIDTH);
        state.input_buffer = "abcdefghi".to_string();
        state.move_input_cursor_to_end();

        assert_eq!(state.effective_input_scroll_offset(), 4);

        state.move_input_cursor_to_visible_column(1);
        assert_eq!(state.input_cursor(), 5);
    }

    #[test]
    fn restored_draft_and_edit_mode_place_cursor_at_end() {
        let mut state = state_with_chats();
        state.input_buffer = "plain".to_string();
        state.save_current_draft();
        state.input_buffer.clear();

        state.restore_draft_for_selected_chat();
        assert_eq!(state.input_cursor(), 5);

        state.enter_edit_mode(42, "edited".to_string());
        assert_eq!(state.input_cursor(), 6);
    }

    #[test]
    fn cancel_plain_input_discards_draft_and_returns_to_messages() {
        let mut state = state_with_chats();
        state.focused_panel = FocusedPanel::Input;
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();

        state.cancel_input_mode();

        assert_eq!(state.input_buffer, "");
        assert!(state.conversation_draft(10, None).is_none());
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
    }

    #[test]
    fn cancel_compose_mode_restores_saved_draft_and_returns_to_messages() {
        let mut state = state_with_chats();
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();
        state.enter_edit_mode(42, "edited text".to_string());

        state.cancel_compose_mode();

        assert!(state.editing_message_id.is_none());
        assert!(state.replying_to_message_id.is_none());
        assert_eq!(state.input_buffer, "plain draft");
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
    }

    #[test]
    fn finish_compose_mode_restores_saved_draft_without_changing_focus() {
        let mut state = state_with_chats();
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();
        state.enter_reply_mode(42);
        state.input_buffer = "reply sent".to_string();

        state.finish_compose_mode();

        assert!(state.editing_message_id.is_none());
        assert!(state.replying_to_message_id.is_none());
        assert_eq!(state.input_buffer, "plain draft");
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    #[test]
    fn prepare_message_submit_returns_none_for_empty_input() {
        let mut state = state_with_chats();

        assert_eq!(state.prepare_message_submit(), None);
        assert!(state.error_message.is_none());

        state.input_buffer = "   \t".to_string();
        assert_eq!(state.prepare_message_submit(), None);
        assert!(state.error_message.is_none());
    }

    #[test]
    fn prepare_message_submit_sets_error_when_no_chat_is_selected() {
        let mut state = AppState::new();
        state.input_buffer = "hello".to_string();

        assert_eq!(state.prepare_message_submit(), None);
        assert_eq!(state.error_message.as_deref(), Some(NO_CHAT_SELECTED_ERROR));
    }

    #[test]
    fn prepare_message_submit_returns_send_action_for_plain_input() {
        let mut state = state_with_chats();
        state.input_buffer = "hello".to_string();

        assert_eq!(
            state.prepare_message_submit(),
            Some(MessageSubmitAction::Send {
                chat_id: 10,
                thread_top_message_id: None,
                content: "hello".to_string(),
            })
        );
    }

    #[test]
    fn prepare_message_submit_includes_selected_thread_topic_for_plain_send() {
        let mut state = state_with_chats();
        state.input_buffer = "thread hello".to_string();
        state.apply_loaded_selected_chat_thread_topics(vec![ThreadTopic {
            id: 101,
            title: "General".to_string(),
            top_message_id: 901,
            unread_count: 0,
            is_closed: false,
            is_pinned: false,
        }]);

        assert_eq!(
            state.prepare_message_submit(),
            Some(MessageSubmitAction::Send {
                chat_id: 10,
                thread_top_message_id: Some(101),
                content: "thread hello".to_string(),
            })
        );
    }

    #[test]
    fn prepare_message_submit_returns_edit_or_reply_action_for_compose_modes() {
        let mut state = state_with_chats();
        state.input_buffer = "edited".to_string();
        state.enter_edit_mode(42, "edited".to_string());

        assert_eq!(
            state.prepare_message_submit(),
            Some(MessageSubmitAction::Edit {
                chat_id: 10,
                message_id: 42,
                content: "edited".to_string(),
            })
        );

        state.clear_input_mode();
        state.input_buffer = "reply".to_string();
        state.enter_reply_mode(43);

        assert_eq!(
            state.prepare_message_submit(),
            Some(MessageSubmitAction::Reply {
                chat_id: 10,
                thread_top_message_id: None,
                message_id: 43,
                content: "reply".to_string(),
            })
        );
    }

    #[test]
    fn prepare_message_submit_includes_selected_thread_topic_for_reply() {
        let mut state = state_with_chats();
        state.input_buffer = "thread reply".to_string();
        state.apply_loaded_selected_chat_thread_topics(vec![ThreadTopic {
            id: 101,
            title: "General".to_string(),
            top_message_id: 901,
            unread_count: 0,
            is_closed: false,
            is_pinned: false,
        }]);
        state.enter_reply_mode(43);

        assert_eq!(
            state.prepare_message_submit(),
            Some(MessageSubmitAction::Reply {
                chat_id: 10,
                thread_top_message_id: Some(101),
                message_id: 43,
                content: "thread reply".to_string(),
            })
        );
    }

    #[test]
    fn message_action_failure_errors_use_shared_prefixes() {
        assert_eq!(
            edit_failed_error("network down"),
            "Edit failed: network down"
        );
        assert_eq!(
            reply_failed_error("network down"),
            "Reply failed: network down"
        );
        assert_eq!(
            send_failed_error("network down"),
            "Send failed: network down"
        );
        assert_eq!(
            delete_failed_error("network down"),
            "Delete failed: network down"
        );
    }

    #[test]
    fn apply_edit_success_updates_message_and_finishes_compose() {
        let mut state = state_with_chats();
        state.messages = vec![update_message(42, 10, "old text", true)];
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();
        state.enter_edit_mode(42, "edited draft".to_string());

        state.apply_edit_success(42, "new text".to_string());

        assert_eq!(state.messages[0].content, "new text");
        assert!(state.messages[0].is_edited);
        assert!(state.editing_message_id.is_none());
        assert_eq!(state.input_buffer, "plain draft");
        assert_eq!(state.chats[0].last_message.as_deref(), Some("new text"));
        assert_eq!(state.status_message.as_deref(), Some(MESSAGE_EDITED_STATUS));
    }

    #[test]
    fn apply_edit_failure_sets_error_without_leaving_compose() {
        let mut state = state_with_chats();
        state.enter_edit_mode(42, "edited draft".to_string());

        state.apply_edit_failure("network down".to_string());

        assert_eq!(state.editing_message_id, Some(42));
        assert_eq!(state.input_buffer, "edited draft");
        assert_eq!(
            state.error_message.as_deref(),
            Some(edit_failed_error("network down").as_str())
        );
    }

    #[test]
    fn apply_reply_success_appends_selects_and_finishes_compose() {
        let mut state = state_with_chats();
        state.messages = vec![update_message(41, 10, "existing", false)];
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();
        state.enter_reply_mode(41);
        state.input_buffer = "reply draft".to_string();

        state.apply_reply_success(update_message(42, 10, "new reply", true));

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].content, "new reply");
        assert_eq!(state.selected_message_index, 1);
        assert!(state.replying_to_message_id.is_none());
        assert_eq!(state.input_buffer, "plain draft");
        assert_eq!(state.chats[0].last_message.as_deref(), Some("new reply"));
        assert_eq!(state.status_message.as_deref(), Some(REPLY_SENT_STATUS));
    }

    #[test]
    fn apply_reply_failure_sets_error_without_leaving_compose() {
        let mut state = state_with_chats();
        state.enter_reply_mode(41);
        state.input_buffer = "reply draft".to_string();

        state.apply_reply_failure("network down".to_string());

        assert_eq!(state.replying_to_message_id, Some(41));
        assert_eq!(state.input_buffer, "reply draft");
        assert_eq!(
            state.error_message.as_deref(),
            Some(reply_failed_error("network down").as_str())
        );
    }

    #[test]
    fn apply_send_pending_adds_sending_message_and_clears_draft() {
        let mut state = state_with_chats();
        state.focused_panel = FocusedPanel::Input;
        state.input_buffer = "plain send".to_string();
        state.save_current_draft();

        state.apply_send_pending(-1, 10, None, "plain send".to_string());

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, -1);
        assert_eq!(state.messages[0].content, "plain send");
        assert_eq!(state.messages[0].status, MessageStatus::Sending);
        assert!(!state.messages[0].can_edit);
        assert!(!state.messages[0].can_delete);
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.input_buffer, "");
        assert_eq!(state.chats[0].last_message.as_deref(), Some("plain send"));
        assert!(state.conversation_draft(10, None).is_none());
    }

    #[test]
    fn apply_send_success_replaces_pending_message_clears_progress_and_preserves_read_state() {
        let mut state = state_with_chats();
        state.chats[0].unread_count = 2;
        state.folders = vec![all_folder(2)];
        state.set_status("Sending message…");
        state.apply_send_pending(-1, 10, None, "plain send".to_string());

        let mut sent_message = update_message(42, 10, "plain send", true);
        sent_message.status = MessageStatus::Sent;
        state.apply_send_success(-1, sent_message);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 42);
        assert_eq!(state.messages[0].status, MessageStatus::Sent);
        assert_eq!(state.chats[0].last_message.as_deref(), Some("plain send"));
        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(state.folders[0].unread_count, 2);
        assert!(state.status_message.is_none());
    }

    #[test]
    fn topic_send_success_preserves_other_topic_unread() {
        let mut state = state_with_chats();
        state.chats[0].unread_count = 5;
        state.folders = vec![all_folder(5)];
        state.thread_topics = vec![
            thread_topic(101, "General"),
            thread_topic(102, "Deployments"),
        ];
        state.thread_topics[0].unread_count = 2;
        state.thread_topics[1].unread_count = 3;
        state.apply_send_pending(-1, 10, Some(101), "topic send".to_string());

        let mut sent_message = update_message(42, 10, "topic send", true);
        sent_message.thread_topic_id = Some(101);
        sent_message.status = MessageStatus::Sent;
        state.apply_send_success(-1, sent_message);

        assert_eq!(state.chats[0].unread_count, 3);
        assert_eq!(state.folders[0].unread_count, 5);
        assert_eq!(state.thread_topics[0].unread_count, 0);
        assert_eq!(state.thread_topics[1].unread_count, 3);
    }

    #[test]
    fn unresolved_scope_send_success_does_not_clear_chat_unread() {
        let mut state = state_with_chats();
        state.chats[0].unread_count = 5;
        state.folders = vec![all_folder(5)];
        state.apply_send_pending(-1, 10, Some(101), "topic send".to_string());
        let mut sent_message = update_message(42, 10, "topic send", true);
        sent_message.thread_topic_id = Some(101);
        sent_message.status = MessageStatus::Sent;

        state.apply_send_success(-1, sent_message);

        assert_eq!(state.chats[0].unread_count, 5);
        assert_eq!(state.folders[0].unread_count, 5);
    }

    #[test]
    fn apply_send_failure_marks_pending_message_failed_and_sets_error() {
        let mut state = state_with_chats();
        state.apply_send_pending(-1, 10, None, "plain send".to_string());

        state.apply_send_failure(-1, "network down".to_string());

        assert_eq!(state.messages[0].status, MessageStatus::Failed);
        assert!(!state.messages[0].can_edit);
        assert!(!state.messages[0].can_delete);
        assert_eq!(state.messages[0].error.as_deref(), Some("network down"));
        assert_eq!(state.input_buffer, "plain send");
        assert_eq!(state.input_cursor(), 10);
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        assert_eq!(
            state.error_message.as_deref(),
            Some(send_failed_error("network down").as_str())
        );
    }

    #[test]
    fn apply_delete_success_removes_message_and_preserves_current_modal() {
        let mut state = state_with_chats();
        let confirmation = DeleteConfirmation {
            chat_id: 10,
            message_id: 42,
        };
        state.messages_area = message_area(TEST_SHORT_MESSAGE_AREA_HEIGHT);
        state.messages = vec![
            update_message(41, 10, "keep", true),
            update_message(42, 10, "delete", true),
        ];
        state.selected_message_index = 1;
        state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 10,
            message_id: 41,
        });

        state.apply_delete_success(confirmation);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 41);
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.message_scroll_offset, 0);
        assert_eq!(state.chats[0].last_message.as_deref(), Some("keep"));
        assert_eq!(
            state.delete_confirmation(),
            Some(DeleteConfirmation {
                chat_id: 10,
                message_id: 41,
            })
        );
        assert_eq!(
            state.status_message.as_deref(),
            Some(MESSAGE_DELETED_STATUS)
        );
    }

    #[test]
    fn apply_delete_success_clears_preview_when_no_loaded_messages_remain() {
        let mut state = state_with_chats();
        let confirmation = DeleteConfirmation {
            chat_id: 10,
            message_id: 42,
        };
        state.chats[0].last_message = Some("delete".to_string());
        state.messages = vec![update_message(42, 10, "delete", true)];

        state.apply_delete_success(confirmation);

        assert!(state.messages.is_empty());
        assert_eq!(state.chats[0].last_message, None);
    }

    #[test]
    fn apply_delete_failure_sets_error_and_preserves_newer_confirmation() {
        let mut state = AppState::new();
        let failed = DeleteConfirmation {
            chat_id: 10,
            message_id: 42,
        };
        let newer = DeleteConfirmation {
            chat_id: 10,
            message_id: 43,
        };
        state.set_delete_confirmation(newer);

        state.apply_delete_failure(failed, "network down".to_string());

        assert_eq!(state.delete_confirmation(), Some(newer));
        assert_eq!(
            state.error_message.as_deref(),
            Some(delete_failed_error("network down").as_str())
        );
    }

    #[test]
    fn cancel_delete_confirmation_clears_pending_confirmation() {
        let mut state = AppState::new();
        state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 10,
            message_id: 42,
        });

        state.cancel_delete_confirmation();

        assert!(state.delete_confirmation().is_none());
    }

    #[test]
    fn status_and_error_notifications_are_managed_separately() {
        let mut state = AppState::new();

        state.set_status("Status ok");
        assert_eq!(state.status_message.as_deref(), Some("Status ok"));

        state.set_error("Send failed".to_string());
        assert_eq!(state.error_message.as_deref(), Some("Send failed"));
        assert!(state.status_message.is_none());

        state.set_status("Status ok");
        assert_eq!(state.status_message.as_deref(), Some("Status ok"));
        assert!(state.error_message.is_none());
        assert!(state.error_timestamp.is_none());
    }

    #[test]
    fn notification_deadline_uses_earliest_active_notification() {
        let mut state = AppState::new();
        let now = tokio::time::Instant::now();
        state.error_timestamp = Some(now + StdDuration::from_secs(1));
        state.status_timestamp = Some(now);

        assert_eq!(
            state.notification_deadline(),
            Some(now + NOTIFICATION_LIFETIME)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn notification_clears_at_preserved_effective_boundary() {
        let mut state = AppState::new();
        assert!(state.notification_deadline().is_none());
        state.set_status("Status ok");

        tokio::time::advance(NOTIFICATION_LIFETIME - StdDuration::from_millis(1)).await;
        assert!(!state.check_notification_timeout());
        assert_eq!(state.status_message.as_deref(), Some("Status ok"));

        tokio::time::advance(StdDuration::from_millis(1)).await;
        assert!(state.check_notification_timeout());
        assert!(state.status_message.is_none());
        assert!(state.notification_deadline().is_none());
    }

    #[test]
    fn conversation_load_status_tracks_loading_success_empty_and_failure() {
        let mut state = AppState::new();
        state.messages = vec![message(1)];

        state.begin_conversation_load();
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Loading
        );
        assert!(state.messages.is_empty());

        state.apply_loaded_selected_chat_messages(vec![message(2)]);
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Loaded
        );

        state.begin_conversation_load();
        state.apply_loaded_selected_chat_messages(Vec::new());
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Empty
        );

        state.begin_conversation_load();
        state.mark_conversation_load_failed();
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Failed
        );
    }

    #[test]
    fn typing_action_cooldown_resets_for_context_changes() {
        let mut state = AppState::new();
        let now = Instant::now();

        assert!(state.typing_action_due_at(1, None, now));
        assert!(!state.typing_action_due_at(1, None, now + StdDuration::from_secs(1)));
        assert!(state.typing_action_due_at(2, None, now + StdDuration::from_secs(1)));
        assert!(state.typing_action_due_at(2, Some(10), now + StdDuration::from_secs(2)));
        assert!(state.typing_action_due_at(1, None, now + StdDuration::from_secs(2)));
        assert!(!state.typing_action_due_at(1, None, now + StdDuration::from_secs(3)));
        assert!(state.typing_action_due_at(
            1,
            None,
            now + StdDuration::from_secs(2) + TYPING_ACTION_COOLDOWN
        ));

        state.reset_typing_action_cooldown();
        assert!(state.typing_action_due_at(1, None, now + StdDuration::from_secs(3)));
    }

    #[test]
    fn empty_plain_draft_removes_saved_draft() {
        let mut state = state_with_chats();
        state.input_buffer = "temporary".to_string();
        state.save_current_draft();
        assert_eq!(state.conversation_draft(10, None), Some("temporary"));

        state.input_buffer.clear();
        state.save_current_draft();
        assert!(state.conversation_draft(10, None).is_none());
    }

    #[test]
    fn leave_selected_chat_saves_plain_draft() {
        let mut state = state_with_chats();
        state.input_buffer = "leaving draft".to_string();

        state.leave_selected_chat();

        assert_eq!(state.conversation_draft(10, None), Some("leaving draft"));
        assert_eq!(state.input_buffer, "leaving draft");
    }

    #[test]
    fn leave_selected_chat_clears_compose_without_overwriting_saved_draft() {
        let mut state = state_with_chats();
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();
        state.enter_edit_mode(42, "edited text".to_string());

        state.leave_selected_chat();

        assert!(state.editing_message_id.is_none());
        assert!(state.replying_to_message_id.is_none());
        assert_eq!(state.input_buffer, "");
        assert_eq!(state.conversation_draft(10, None), Some("plain draft"));
    }

    #[test]
    fn selected_folder_filter_id_ignores_all_folder() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0), folder(2, "Personal", 0)];

        assert_eq!(state.selected_folder_filter_id(), None);

        state.selected_folder_index = 1;
        assert_eq!(state.selected_folder_filter_id(), Some(2));
    }

    #[test]
    fn selected_folder_filter_id_allows_telegram_folder_one() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0), folder(1, "Archived", 0)];
        state.selected_folder_index = 1;

        assert_eq!(state.selected_folder_filter_id(), Some(1));
    }

    #[test]
    fn apply_loaded_selected_chat_messages_selects_latest_unread_and_restores_draft() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(99), folder(2, "Personal", 99)];
        state.chats = vec![chat_with_unread(10, "Alice", 3, Some(2))];
        state.input_buffer = "saved draft".to_string();
        state.save_current_draft();
        state.input_buffer = "stale input".to_string();
        state.selected_message_index = 99;
        state.message_scroll_offset = 99;

        state.apply_loaded_selected_chat_messages(vec![
            update_message(41, 10, "older", false),
            update_message(42, 10, "loaded", false),
        ]);

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.selected_message_index, 1);
        assert_eq!(state.message_scroll_offset, 1);
        assert_eq!(state.chats[0].unread_count, 0);
        assert_eq!(state.chats[0].last_message.as_deref(), Some("loaded"));
        assert_eq!(state.folders[0].unread_count, 99);
        assert_eq!(state.folders[1].unread_count, 99);
        assert_eq!(state.input_buffer, "saved draft");
    }

    #[test]
    fn apply_loaded_selected_chat_messages_clears_older_history_exhausted_cache() {
        let mut state = AppState::new();
        state.chats = vec![chat(10, "Alice")];
        state.mark_selected_chat_older_history_exhausted();
        assert!(state.selected_chat_older_history_exhausted());

        state.apply_loaded_selected_chat_messages(vec![message(10)]);

        assert!(!state.selected_chat_older_history_exhausted());
    }

    #[test]
    fn apply_loaded_selected_chat_messages_clears_preview_when_history_is_empty() {
        let mut state = AppState::new();
        state.chats = vec![chat(10, "Alice")];
        state.chats[0].last_message = Some("stale preview".to_string());

        state.apply_loaded_selected_chat_messages(vec![]);

        assert_eq!(state.chats[0].last_message, None);
    }

    #[test]
    fn thread_topic_selection_clamps_to_loaded_topics() {
        let mut state = AppState::new();
        state.apply_loaded_selected_chat_thread_topics(vec![
            ThreadTopic {
                id: 101,
                title: "General".to_string(),
                top_message_id: 101,
                unread_count: 1,
                is_closed: false,
                is_pinned: true,
            },
            ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 102,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            },
        ]);

        assert_eq!(state.selected_thread_topic().unwrap().title, "General");
        state.select_next_thread_topic();
        assert_eq!(state.selected_thread_topic_index, 1);
        assert_eq!(state.selected_thread_topic().unwrap().title, "Deployments");
        state.select_next_thread_topic();
        assert_eq!(state.selected_thread_topic_index, 0);
        state.select_prev_thread_topic();
        assert_eq!(state.selected_thread_topic_index, 1);

        state.selected_thread_topic_index = 99;
        state.apply_loaded_selected_chat_thread_topics(vec![ThreadTopic {
            id: 103,
            title: "Only".to_string(),
            top_message_id: 103,
            unread_count: 0,
            is_closed: false,
            is_pinned: false,
        }]);
        assert_eq!(state.selected_thread_topic_index, 0);
    }

    #[test]
    fn clear_loaded_chat_messages_resets_messages_selection_and_input() {
        let mut state = AppState::new();
        state.messages = vec![message(10), message(20)];
        state.thread_topics = vec![ThreadTopic {
            id: 101,
            title: "General".to_string(),
            top_message_id: 101,
            unread_count: 1,
            is_closed: false,
            is_pinned: true,
        }];
        state.selected_message_index = 1;
        state.message_scroll_offset = 1;
        state.input_buffer = "stale".to_string();

        state.clear_loaded_chat_messages();

        assert!(state.messages.is_empty());
        assert!(state.thread_topics.is_empty());
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.message_scroll_offset, 0);
        assert_eq!(state.input_buffer, "");
    }

    #[test]
    fn chat_search_matches_loaded_chats_by_substring_or_subsequence() {
        let mut state = AppState::new();
        state.chats = vec![
            chat(1, "Alice Personal"),
            chat(2, "Work Team"),
            chat(3, "Project Alpha"),
        ];
        state.messages = vec![message(9)];
        state.input_buffer = "draft".to_string();
        state.conversation_load_status = ConversationLoadStatus::Loaded;
        state.begin_chat_search();

        state.push_chat_search_char('p');
        state.push_chat_search_char('r');
        assert_eq!(state.chat_display_indices(), vec![0, 2]);

        state.push_chat_search_char('o');
        state.push_chat_search_char('j');
        assert_eq!(state.chat_display_indices(), vec![2]);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.selected_chat_search_result_index(), Some(2));
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, 9);
        assert_eq!(state.input_buffer, "draft");
        assert_eq!(
            state.conversation_load_status,
            ConversationLoadStatus::Loaded
        );

        state.pop_chat_search_char();
        state.pop_chat_search_char();
        assert_eq!(state.chat_display_indices(), vec![0, 2]);

        state.chat_search_query = Some("wt".to_string());
        assert_eq!(state.chat_display_indices(), vec![1]);
        assert_eq!(state.selected_chat_search_result_index(), Some(1));
    }

    #[test]
    fn chat_search_keeps_selected_filtered_result_visible_without_moving_normal_scroll() {
        let mut state = state_with_many_chats();
        state.begin_chat_search();
        state.push_chat_search_char('c');

        for _ in 0..5 {
            state.select_next_chat_search_match();
        }

        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.chat_search_selected_index, 5);
        assert_eq!(state.selected_chat_search_result_index(), Some(5));
        assert_eq!(state.selected_chat_display_index(), Some(5));
        assert_eq!(state.chat_search_scroll_offset, 4);
        assert_eq!(state.chat_scroll_offset, 0);

        state.clear_chat_search();
        assert!(!state.chat_search_active());
        assert_eq!(state.chat_search_scroll_offset, 0);
        assert_eq!(state.chat_scroll_offset, 0);
    }

    #[test]
    fn empty_chat_search_starts_on_active_chat_without_changing_it() {
        let mut state = state_with_many_chats();
        state.select_chat(5);

        state.begin_chat_search();

        assert_eq!(state.selected_chat_index, 5);
        assert_eq!(state.chat_search_selected_index, 5);
        assert_eq!(state.selected_chat_search_result_index(), Some(5));
        assert_eq!(state.chat_search_scroll_offset, 4);
    }

    #[test]
    fn chat_selection_keeps_scroll_offset_visible() {
        let mut state = state_with_many_chats();

        state.select_chat(5);
        assert_eq!(state.selected_chat_index, 5);
        assert_eq!(state.chat_scroll_offset, 4);

        state.select_chat(4);
        assert_eq!(state.selected_chat_index, 4);
        assert_eq!(state.chat_scroll_offset, 4);

        state.select_chat(0);
        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.chat_scroll_offset, 0);
    }

    #[test]
    fn chat_selection_clamps_scroll_offset_when_viewport_grows() {
        let mut state = state_with_many_chats();
        state.select_chat(7);
        assert_eq!(state.chat_scroll_offset, 6);

        state.chats_area = chat_area(TEST_TALL_CHAT_AREA_HEIGHT);
        state.ensure_selected_chat_visible();

        assert_eq!(state.selected_chat_index, 7);
        assert_eq!(state.chat_scroll_offset, 0);
    }

    #[test]
    fn reset_chat_selection_clears_scroll() {
        let mut state = state_with_many_chats();
        state.select_chat(7);
        assert!(state.chat_scroll_offset > 0);

        state.reset_chat_selection();

        assert_eq!(state.selected_chat_index, 0);
        assert_eq!(state.chat_scroll_offset, 0);
    }

    #[test]
    fn select_message_at_visible_row_accounts_for_reply_rows_and_scroll_offset() {
        let mut state = AppState::new();
        state.messages_area = message_area_with_width(
            TEST_VISIBLE_ROW_MESSAGE_AREA_WIDTH,
            TEST_SHORT_MESSAGE_AREA_HEIGHT,
        );
        state.messages = vec![message(1), message(2), message(3), message(4)];
        state.messages[1].reply_to_content = Some("earlier".to_string());
        state.message_scroll_offset = 1;

        state.select_message_at_visible_row(0);
        assert_eq!(state.selected_message_index, 1);

        state.select_message_at_visible_row(1);
        assert_eq!(state.selected_message_index, 1);

        state.select_message_at_visible_row(2);
        assert_eq!(state.selected_message_index, 2);
    }

    #[test]
    fn select_message_at_visible_row_ignores_blank_rows() {
        let mut state = AppState::new();
        state.messages_area = message_area_with_width(
            TEST_VISIBLE_ROW_MESSAGE_AREA_WIDTH,
            TEST_BLANK_ROW_MESSAGE_AREA_HEIGHT,
        );
        state.messages = vec![message(1), message(2)];
        state.selected_message_index = 1;

        state.select_message_at_visible_row(5);

        assert_eq!(state.selected_message_index, 1);
    }

    #[test]
    fn selected_message_returns_current_message_when_in_bounds() {
        let mut state = AppState::new();
        state.messages = vec![message(10), message(20)];
        state.selected_message_index = 1;

        assert_eq!(state.selected_message().map(|message| message.id), Some(20));

        state.selected_message_index = 99;
        assert!(state.selected_message().is_none());
    }

    #[test]
    fn selected_media_preview_request_and_apply_require_exact_image_message() {
        let mut state = AppState::new();
        state.messages = vec![message(10), message(20)];
        state.messages[0].chat_id = 1;
        state.messages[0].media = Some(MessageMedia::photo());

        assert_eq!(state.selected_media_preview_request(), Some((1, 10)));
        assert!(!state.apply_selected_media_preview(2, 10, "/tmp/wrong-chat.png".into()));
        assert!(!state.apply_selected_media_preview(1, 20, "/tmp/wrong-message.png".into()));
        assert!(state.apply_selected_media_preview(1, 10, "/tmp/preview.png".into()));
        assert_eq!(state.selected_media_preview_request(), None);
        assert_eq!(
            state.messages[0]
                .media
                .as_ref()
                .and_then(|media| media.local_image_path()),
            Some(std::path::Path::new("/tmp/preview.png"))
        );

        state.selected_message_index = 1;
        assert_eq!(state.selected_media_preview_request(), None);
    }

    #[test]
    fn selected_message_download_path_matches_selected_message_only() {
        let mut state = AppState::new();
        state.messages = vec![message(10), message(20)];
        state.record_downloaded_media(10, 20, "/tmp/downloaded.bin".into());

        assert_eq!(state.selected_message_download_path(), None);

        state.selected_message_index = 1;
        assert_eq!(
            state.selected_message_download_path(),
            Some(std::path::Path::new("/tmp/downloaded.bin"))
        );
    }

    #[test]
    fn selected_message_is_last_treats_empty_or_clamped_tail_as_end() {
        let mut state = AppState::new();
        assert!(state.selected_message_is_last());

        state.messages = vec![message(10), message(20)];
        state.selected_message_index = 0;
        assert!(!state.selected_message_is_last());

        state.selected_message_index = 1;
        assert!(state.selected_message_is_last());

        state.selected_message_index = 99;
        assert!(state.selected_message_is_last());
    }

    #[test]
    fn request_edit_selected_message_enters_edit_mode_and_preserves_draft() {
        let mut state = AppState::new();
        state.chats = vec![chat(10, "Alice")];
        state.messages = vec![update_message(21, 10, "editable", true)];
        state.input_buffer = "plain draft".to_string();

        state.request_edit_selected_message();

        assert_eq!(state.editing_message_id, Some(21));
        assert!(state.replying_to_message_id.is_none());
        assert_eq!(state.input_buffer, "editable");
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        assert_eq!(state.conversation_draft(10, None), Some("plain draft"));
        assert!(state.error_message.is_none());
    }

    #[test]
    fn request_edit_selected_message_reports_error_for_non_editable_message() {
        let mut state = AppState::new();
        state.messages = vec![update_message(21, 10, "not mine", false)];

        state.request_edit_selected_message();

        assert!(state.editing_message_id.is_none());
        assert_eq!(
            state.error_message.as_deref(),
            Some(CANNOT_EDIT_MESSAGE_ERROR)
        );
    }

    #[test]
    fn request_edit_selected_message_rejects_local_only_send_rows() {
        let mut state = AppState::new();
        state.messages = vec![update_message(-1, 10, "pending", true)];
        state.messages[0].status = MessageStatus::Sending;
        state.messages[0].can_edit = true;

        state.request_edit_selected_message();

        assert!(state.editing_message_id.is_none());
        assert_eq!(
            state.error_message.as_deref(),
            Some(CANNOT_EDIT_MESSAGE_ERROR)
        );
    }

    #[test]
    fn request_reply_to_selected_message_enters_reply_mode_and_preserves_draft() {
        let mut state = AppState::new();
        state.chats = vec![chat(10, "Alice")];
        state.messages = vec![update_message(21, 10, "reply target", false)];
        state.input_buffer = "plain draft".to_string();

        state.request_reply_to_selected_message();

        assert_eq!(state.replying_to_message_id, Some(21));
        assert!(state.editing_message_id.is_none());
        assert_eq!(state.input_buffer, "plain draft");
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        assert_eq!(state.conversation_draft(10, None), Some("plain draft"));
    }

    #[test]
    fn request_reply_to_selected_message_rejects_local_only_send_rows() {
        let mut state = AppState::new();
        state.messages = vec![update_message(-1, 10, "failed", true)];
        state.messages[0].status = MessageStatus::Failed;

        state.request_reply_to_selected_message();

        assert!(state.replying_to_message_id.is_none());
        assert_eq!(
            state.error_message.as_deref(),
            Some(CANNOT_REPLY_UNSENT_MESSAGE_ERROR)
        );
    }

    #[test]
    fn request_delete_selected_message_sets_confirmation_for_deletable_message() {
        let mut state = AppState::new();
        state.messages = vec![update_message(21, 10, "delete me", true)];

        state.request_delete_selected_message();

        assert_eq!(
            state.delete_confirmation(),
            Some(DeleteConfirmation {
                chat_id: 10,
                message_id: 21,
            })
        );
        assert!(state.error_message.is_none());
    }

    #[test]
    fn request_delete_selected_message_reports_error_for_non_deletable_message() {
        let mut state = AppState::new();
        state.messages = vec![update_message(21, 10, "not mine", false)];

        state.request_delete_selected_message();

        assert!(state.delete_confirmation().is_none());
        assert_eq!(
            state.error_message.as_deref(),
            Some(CANNOT_DELETE_MESSAGE_ERROR)
        );
    }

    #[test]
    fn request_delete_selected_message_rejects_local_only_send_rows() {
        let mut state = AppState::new();
        state.messages = vec![update_message(-1, 10, "pending", true)];
        state.messages[0].status = MessageStatus::Sending;
        state.messages[0].can_delete = true;

        state.request_delete_selected_message();

        assert!(state.delete_confirmation().is_none());
        assert_eq!(
            state.error_message.as_deref(),
            Some(CANNOT_DELETE_MESSAGE_ERROR)
        );
    }

    #[test]
    fn request_delete_selected_message_dismisses_failed_local_send_without_confirmation() {
        let mut state = state_with_chats();
        state.messages = vec![
            update_message(41, 10, "previous", true),
            update_message(-1, 10, "failed draft", true),
        ];
        state.messages[1].status = MessageStatus::Failed;
        state.messages[1].can_delete = false;
        state.selected_message_index = 1;
        state.chats[0].last_message = Some("failed draft".to_string());
        state.input_buffer = "failed draft".to_string();

        state.request_delete_selected_message();

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "previous");
        assert_eq!(state.selected_message_index, 0);
        assert!(state.delete_confirmation().is_none());
        assert_eq!(state.chats[0].last_message.as_deref(), Some("previous"));
        assert_eq!(state.input_buffer, "failed draft");
        assert_eq!(
            state.status_message.as_deref(),
            Some(FAILED_SEND_DISMISSED_STATUS)
        );
        assert!(state.error_message.is_none());
    }

    #[test]
    fn message_selection_keeps_scroll_offset_visible() {
        let mut state = AppState::new();
        state.messages_area = message_area(TEST_SHORT_MESSAGE_AREA_HEIGHT);
        state.messages = (0..10).map(message).collect();

        state.select_last_message();
        assert_eq!(state.selected_message_index, 9);
        assert_eq!(state.message_scroll_offset, 7);

        state.select_prev_message();
        assert_eq!(state.selected_message_index, 8);
        assert_eq!(state.message_scroll_offset, 7);

        state.select_first_message();
        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.message_scroll_offset, 0);
    }

    #[test]
    fn message_selection_clamps_scroll_offset_when_viewport_grows() {
        let mut state = AppState::new();
        state.messages_area = message_area(TEST_SHORT_MESSAGE_AREA_HEIGHT);
        state.messages = (0..10).map(message).collect();
        state.select_last_message();
        assert_eq!(state.message_scroll_offset, 7);

        state.messages_area = message_area(TEST_TALL_MESSAGE_AREA_HEIGHT);
        state.ensure_selected_message_visible();

        assert_eq!(state.selected_message_index, 9);
        assert_eq!(state.message_scroll_offset, 0);
    }

    #[test]
    fn page_message_navigation_moves_by_visible_capacity() {
        let mut state = AppState::new();
        state.messages_area = message_area(TEST_PAGED_MESSAGE_AREA_HEIGHT);
        state.messages = (0..10).map(message).collect();

        state.page_messages_down();
        assert_eq!(state.selected_message_index, 4);
        assert_eq!(state.message_scroll_offset, 1);

        state.page_messages_down();
        assert_eq!(state.selected_message_index, 8);
        assert_eq!(state.message_scroll_offset, 5);

        state.page_messages_up();
        assert_eq!(state.selected_message_index, 4);
        assert_eq!(state.message_scroll_offset, 4);
    }

    #[test]
    fn reset_message_selection_clears_scroll() {
        let mut state = AppState::new();
        state.messages_area = message_area(TEST_SHORT_MESSAGE_AREA_HEIGHT);
        state.messages = (0..10).map(message).collect();
        state.select_last_message();

        state.reset_message_selection();

        assert_eq!(state.selected_message_index, 0);
        assert_eq!(state.message_scroll_offset, 0);
    }

    #[test]
    fn split_ratio_uses_shared_default_step_and_bounds() {
        let mut state = AppState::new();
        assert_eq!(state.split_ratio, DEFAULT_SPLIT_RATIO);

        state.adjust_split_left();
        assert_eq!(state.split_ratio, DEFAULT_SPLIT_RATIO - SPLIT_RATIO_STEP);

        state.split_ratio = MIN_SPLIT_RATIO;
        state.adjust_split_left();
        assert_eq!(state.split_ratio, MIN_SPLIT_RATIO);

        state.split_ratio = MAX_SPLIT_RATIO;
        state.adjust_split_right();
        assert_eq!(state.split_ratio, MAX_SPLIT_RATIO);
    }

    #[test]
    fn context_menu_capabilities_clamp_and_stay_mutually_exclusive_with_delete() {
        let mut state = AppState::new();
        state.screen_area = Rect::new(0, 0, 80, 24);
        state.chats = vec![chat_with_unread(7, "Alice", 3, Some(1))];
        let target = ContextMenuTarget::Chat { chat_id: 7 };

        assert!(state.open_context_menu(target, 79, 23));
        assert_eq!(
            state.context_menu().map(|menu| menu.actions.as_slice()),
            Some(
                [
                    ContextMenuAction::OpenChat,
                    ContextMenuAction::MarkChatRead,
                    ContextMenuAction::CopyChatName,
                ]
                .as_slice()
            )
        );
        let area = state
            .context_menu_rect()
            .expect("menu should have geometry");
        assert!(area.x + area.width <= state.screen_area.width);
        assert!(area.y + area.height <= state.screen_area.height);

        state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 7,
            message_id: 1,
        });
        assert!(state.context_menu().is_none());
        assert!(!state.open_context_menu(target, 1, 1));
        assert!(state.delete_confirmation().is_some());
    }

    #[test]
    fn delete_submission_owner_suppresses_duplicates_and_remote_delete_settles_it() {
        let mut state = AppState::new();
        let confirmation = DeleteConfirmation {
            chat_id: 7,
            message_id: 9,
        };
        state.chats = vec![chat_with_unread(7, "Alice", 0, None)];
        let mut selected = update_message(9, 7, "delete", true);
        selected.can_delete = true;
        state.messages = vec![selected];

        assert!(state.begin_delete_submission(1, confirmation));
        assert!(!state.begin_delete_submission(2, confirmation));
        state.request_delete_selected_message();
        assert!(state.delete_confirmation().is_none());
        assert!(state.error_message.is_none());
        assert!(
            !state
                .context_actions_for_target(ContextMenuTarget::Message {
                    chat_id: 7,
                    message_id: 9,
                })
                .contains(&ContextMenuAction::DeleteMessage)
        );
        assert!(!state.finish_delete_submission(2, confirmation));

        state.apply_update(Update::DeleteMessage {
            chat_id: 7,
            message_id: 9,
        });
        assert!(!state.delete_submission_pending_for(7, 9));
        assert!(!state.finish_delete_submission(1, confirmation));
    }

    #[test]
    fn manual_mark_read_owners_are_per_chat_phase_and_snapshot_exact() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(9)];
        state.chats = vec![
            chat_with_unread(7, "Alice", 3, Some(1)),
            chat_with_unread(8, "Bob", 2, Some(1)),
        ];
        let target = ContextMenuTarget::Chat { chat_id: 7 };

        assert!(state.begin_manual_mark_read(7, 1));
        assert!(!state.begin_manual_mark_read(7, 2));
        assert!(state.begin_manual_mark_read(8, 3));
        assert!(
            !state
                .context_actions_for_target(target)
                .contains(&ContextMenuAction::MarkChatRead)
        );

        assert!(state.accept_manual_mark_read_success(7, 1, 10));
        assert!(!state.accept_manual_mark_read_success(7, 1, 11));
        assert!(!state.finish_manual_mark_read_failure(7, 1));
        assert!(!state.finish_manual_mark_read_snapshot(7, 9));
        assert!(state.manual_mark_read_pending(7));
        assert!(state.finish_manual_mark_read_snapshot(7, 10));
        assert!(!state.manual_mark_read_pending(7));

        assert!(!state.finish_manual_mark_read_failure(8, 2));
        assert!(state.finish_manual_mark_read_failure(8, 3));
        assert!(!state.manual_mark_read_pending(8));
        assert_eq!(state.chats[0].unread_count, 3);
        assert_eq!(state.folders[0].unread_count, 9);
    }

    #[test]
    fn context_menu_re_resolves_message_capabilities_after_target_disappears() {
        let mut state = AppState::new();
        state.chats = vec![chat_with_unread(7, "Alice", 0, Some(1))];
        state.messages = vec![update_message(1, 7, "https://example.com", true)];
        let target = ContextMenuTarget::Message {
            chat_id: 7,
            message_id: 1,
        };

        let actions = state.context_actions_for_target(target);
        assert!(actions.contains(&ContextMenuAction::ReplyMessage));
        assert!(actions.contains(&ContextMenuAction::OpenMessageLink));
        assert!(actions.contains(&ContextMenuAction::DeleteMessage));

        state.messages.clear();
        assert!(state.context_actions_for_target(target).is_empty());
    }

    #[test]
    fn split_drag_clamps_and_releases() {
        let mut state = AppState::new();
        state.screen_area = Rect::new(10, 0, 100, 24);

        state.begin_split_drag(50);
        state.drag_split_to(50);
        assert_eq!(state.split_ratio, DEFAULT_SPLIT_RATIO);
        state.drag_split_to(0);
        assert_eq!(state.split_ratio, MIN_SPLIT_RATIO);
        state.drag_split_to(200);
        assert_eq!(state.split_ratio, MAX_SPLIT_RATIO);
        state.end_split_drag();
        assert!(!state.split_drag_active);
    }

    #[test]
    fn focused_panel_labels_match_rendered_names() {
        assert_eq!(FocusedPanel::Folders.label(), "Folders");
        assert_eq!(FocusedPanel::Chats.label(), "Chats");
        assert_eq!(FocusedPanel::Messages.label(), "Messages");
        assert_eq!(FocusedPanel::Input.label(), "Input");
    }

    #[test]
    fn focus_next_panel_cycles_through_all_panels() {
        let mut state = AppState::new();
        assert_eq!(state.focused_panel, FocusedPanel::Folders);

        state.focus_next_panel();
        assert_eq!(state.focused_panel, FocusedPanel::Chats);
        state.focus_next_panel();
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        state.focus_next_panel();
        assert_eq!(state.focused_panel, FocusedPanel::Input);
        state.focus_next_panel();
        assert_eq!(state.focused_panel, FocusedPanel::Folders);
    }

    #[test]
    fn latest_load_keeps_newest_remote_window_and_protected_local_rows() {
        let mut state = state_with_chats();
        let mut failed = message(-1);
        failed.status = MessageStatus::Failed;
        state.messages = vec![failed];
        let remote = (1..=(MAX_REMOTE_MESSAGES as i32 + 20))
            .map(message)
            .collect::<Vec<_>>();

        state.apply_loaded_selected_chat_messages(remote);

        assert_eq!(state.remote_message_count(), MAX_REMOTE_MESSAGES);
        assert_eq!(state.messages.len(), MAX_REMOTE_MESSAGES + 1);
        assert_eq!(state.messages[0].id, 21);
        assert_eq!(state.messages.last().map(|message| message.id), Some(-1));
        assert!(!state.newer_history_gap());
    }

    #[test]
    fn open_snapshot_preserves_newer_live_tail_once_under_cap() {
        let mut state = state_with_chats();
        state.messages = vec![message(501)];

        state.apply_loaded_selected_chat_messages(
            (1..=MAX_REMOTE_MESSAGES as i32).map(message).collect(),
        );

        assert_eq!(state.remote_message_count(), MAX_REMOTE_MESSAGES);
        assert_eq!(state.messages.first().map(|message| message.id), Some(2));
        assert_eq!(state.messages.last().map(|message| message.id), Some(501));
        assert_eq!(
            state
                .messages
                .iter()
                .filter(|message| message.id == 501)
                .count(),
            1
        );
    }

    #[test]
    fn streaming_retention_preserves_reader_anchor_then_records_newer_gap() {
        let mut state = state_with_chats();
        state.messages = (1..=MAX_REMOTE_MESSAGES as i32).map(message).collect();
        state.selected_message_index = 249;
        state.message_scroll_offset = 240;

        for id in (MAX_REMOTE_MESSAGES as i32 + 1)..=10_000 {
            state.apply_update(Update::NewMessage(message(id)));
        }

        assert_eq!(state.remote_message_count(), MAX_REMOTE_MESSAGES);
        assert_eq!(
            state.selected_message().map(|message| message.id),
            Some(250)
        );
        assert_eq!(
            state
                .messages
                .get(state.message_scroll_offset)
                .map(|message| message.id),
            Some(241)
        );
        assert!(state.newer_history_gap());
        assert_eq!(
            state.chats[0].last_message.as_deref(),
            Some("message 10000")
        );
    }

    #[test]
    fn older_prepend_trims_newer_edge_and_preserves_stable_anchors() {
        let mut state = state_with_chats();
        state.messages = (1_001..=1_500).map(message).collect();
        state.selected_message_index = 0;
        state.message_scroll_offset = 0;
        let older = (981..=1_000).map(message).collect();

        let prepend = state.prepend_loaded_selected_chat_messages(older);

        assert_eq!(prepend.added, 20);
        assert_eq!(state.remote_message_count(), MAX_REMOTE_MESSAGES);
        assert_eq!(state.messages.first().map(|message| message.id), Some(981));
        assert_eq!(state.messages.last().map(|message| message.id), Some(1_480));
        assert_eq!(
            state.selected_message().map(|message| message.id),
            Some(1_001)
        );
        assert_eq!(
            state
                .messages
                .get(state.message_scroll_offset)
                .map(|message| message.id),
            Some(1_001)
        );
        assert!(state.newer_history_gap());
    }

    #[test]
    fn head_eviction_reopens_exact_scope_older_history() {
        let mut state = state_with_chats();
        state.messages = (1..=MAX_REMOTE_MESSAGES as i32).map(message).collect();
        state.selected_message_index = MAX_REMOTE_MESSAGES - 1;
        state.message_scroll_offset = MAX_REMOTE_MESSAGES - 10;
        state.mark_selected_chat_older_history_exhausted();
        assert!(state.selected_chat_older_history_exhausted());

        state.apply_update(Update::NewMessage(message(501)));

        assert!(!state.selected_chat_older_history_exhausted());
        assert_eq!(state.messages.first().map(|message| message.id), Some(2));
        assert!(!state.newer_history_gap());
    }

    #[test]
    fn deletion_before_selection_preserves_selected_message_identity() {
        let mut state = state_with_chats();
        state.messages = vec![message(1), message(2), message(3)];
        state.selected_message_index = 1;

        state.apply_update(Update::DeleteMessage {
            chat_id: 10,
            message_id: 1,
        });

        assert_eq!(state.selected_message().map(|message| message.id), Some(2));
    }

    #[test]
    fn following_the_tail_keeps_sliding_window_without_gap() {
        let mut state = state_with_chats();
        state.messages = (1..=MAX_REMOTE_MESSAGES as i32).map(message).collect();
        state.select_last_message();
        state.message_scroll_offset = MAX_REMOTE_MESSAGES - 10;

        for id in (MAX_REMOTE_MESSAGES as i32 + 1)..=10_000 {
            state.apply_update(Update::NewMessage(message(id)));
            state.select_last_message();
        }

        assert!(!state.newer_history_gap());
        assert_eq!(state.remote_message_count(), MAX_REMOTE_MESSAGES);
        assert_eq!(
            state.messages.first().map(|message| message.id),
            Some(9_501)
        );
        assert_eq!(
            state.messages.last().map(|message| message.id),
            Some(10_000)
        );
    }

    #[test]
    fn refreshed_latest_preserves_unsaved_input() {
        let mut state = state_with_chats();
        state.messages = vec![message(1)];
        state.input_buffer = "unsaved draft".to_string();
        state.input_cursor = state.input_buffer.len();

        state.apply_refreshed_selected_chat_messages(vec![message(2)]);

        assert_eq!(state.input_buffer, "unsaved draft");
        assert_eq!(state.input_cursor, "unsaved draft".len());
        assert_eq!(state.messages[0].id, 2);
    }

    #[test]
    fn reply_submission_has_single_in_flight_owner() {
        let mut state = state_with_chats();

        assert!(state.begin_reply_submission(1));
        assert!(!state.begin_reply_submission(2));
        state.apply_reply_failure("offline".to_string());
        assert!(!state.reply_submission_pending());
        assert!(state.begin_reply_submission(3));
    }

    #[test]
    fn in_flight_compose_owner_rejects_new_keyboard_or_mouse_compose_mode() {
        let mut state = state_with_chats();
        let mut editable = message(1);
        editable.is_own = true;
        editable.can_edit = true;
        state.messages = vec![editable];
        state.request_reply_to_selected_message();
        assert!(state.begin_reply_submission(1));

        let actions = state.context_actions_for_target(ContextMenuTarget::Message {
            chat_id: 10,
            message_id: 1,
        });
        assert!(!actions.contains(&ContextMenuAction::ReplyMessage));
        assert!(!actions.contains(&ContextMenuAction::EditMessage));
        state.request_edit_selected_message();
        assert_eq!(state.replying_to_message_id, Some(1));
        assert_eq!(state.editing_message_id, None);
        assert!(state.reply_submission_pending());

        state.cancel_compose_mode();
        assert!(state.reply_submission_pending());
        state.request_edit_selected_message();
        assert_eq!(state.editing_message_id, None);

        state.finish_reply_submission();
        state.request_edit_selected_message();
        state.begin_edit_submission(2);
        state.request_reply_to_selected_message();
        assert_eq!(state.editing_message_id, Some(1));
        assert_eq!(state.replying_to_message_id, None);
        assert!(state.edit_submission_pending());
    }

    #[test]
    fn edit_submission_releases_only_matching_owner() {
        let mut state = state_with_chats();
        state.begin_edit_submission(10);
        assert!(state.edit_submission_pending());
        assert!(!state.edit_submission_matches(9));
        if state.edit_submission_matches(9) {
            state.finish_edit_submission();
        }
        assert!(state.edit_submission_matches(10));
        state.finish_edit_submission();
        assert!(!state.edit_submission_pending());
    }

    #[test]
    fn confirmed_bodies_are_omitted_when_gap_opens_in_flight() {
        let mut state = state_with_chats();
        state.messages = (1..=MAX_REMOTE_MESSAGES as i32).map(message).collect();
        state.selected_message_index = 0;
        state.apply_send_pending(-1, 10, None, "pending".to_string());
        state.selected_message_index = 0;
        state.apply_update(Update::NewMessage(message(501)));
        assert!(state.newer_history_gap());

        let mut sent = message(502);
        sent.is_own = true;
        state.apply_send_success(-1, sent.clone());
        state.apply_reply_success(sent);

        assert!(state.messages.iter().all(|message| message.id != -1));
        assert!(state.messages.iter().all(|message| message.id != 502));
        assert_eq!(state.remote_message_count(), MAX_REMOTE_MESSAGES);
    }

    #[test]
    fn retention_rejection_does_not_claim_server_history_exhaustion() {
        let mut state = state_with_chats();
        state.messages = (1..=MAX_REMOTE_MESSAGES as i32).map(message).collect();
        state.selected_message_index = 0;
        state.message_scroll_offset = 0;
        state.replying_to_message_id = Some(MAX_REMOTE_MESSAGES as i32);

        let prepend = state.prepend_loaded_selected_chat_messages((-19..=0).map(message).collect());

        assert_eq!(prepend.added, 0);
        assert!(prepend.had_unique_messages);
        assert!(!state.selected_chat_older_history_exhausted());
        assert_eq!(state.remote_message_count(), MAX_REMOTE_MESSAGES);
    }

    #[test]
    fn send_and_reply_completions_deduplicate_live_echoes() {
        let mut send_state = state_with_chats();
        send_state.messages = vec![message(1)];
        send_state.apply_send_pending(-1, 10, None, "sent".to_string());
        let mut echoed_send = message(2);
        echoed_send.is_own = true;
        send_state.apply_update(Update::NewMessage(echoed_send.clone()));
        send_state.apply_send_success(-1, echoed_send);
        assert_eq!(
            send_state
                .messages
                .iter()
                .filter(|message| message.id == 2)
                .count(),
            1
        );
        assert!(send_state.messages.iter().all(|message| message.id != -1));

        let mut reply_state = state_with_chats();
        reply_state.messages = vec![message(1)];
        reply_state.request_reply_to_selected_message();
        let mut echoed_reply = message(2);
        echoed_reply.is_own = true;
        reply_state.apply_update(Update::NewMessage(echoed_reply.clone()));
        reply_state.apply_reply_success(echoed_reply);
        assert_eq!(
            reply_state
                .messages
                .iter()
                .filter(|message| message.id == 2)
                .count(),
            1
        );

        let mut completion_first_send = state_with_chats();
        completion_first_send.messages = vec![message(1)];
        completion_first_send.apply_send_pending(-1, 10, None, "sent".to_string());
        let mut sent = message(2);
        sent.is_own = true;
        completion_first_send.apply_send_success(-1, sent.clone());
        completion_first_send.apply_update(Update::NewMessage(sent));
        assert_eq!(
            completion_first_send
                .messages
                .iter()
                .filter(|message| message.id == 2)
                .count(),
            1
        );

        let mut completion_first_reply = state_with_chats();
        completion_first_reply.messages = vec![message(1)];
        let mut reply = message(2);
        reply.is_own = true;
        completion_first_reply.apply_reply_success(reply.clone());
        completion_first_reply.apply_update(Update::NewMessage(reply));
        assert_eq!(
            completion_first_reply
                .messages
                .iter()
                .filter(|message| message.id == 2)
                .count(),
            1
        );
    }

    #[test]
    fn loading_one_topic_keeps_sibling_topic_exhaustion() {
        let mut state = state_with_chats();
        state.thread_topics = vec![thread_topic(101, "One"), thread_topic(102, "Two")];
        state.selected_thread_topic_index = 0;
        state.mark_selected_chat_older_history_exhausted();
        assert!(state.selected_chat_older_history_exhausted());

        state.selected_thread_topic_index = 1;
        state.apply_loaded_selected_chat_messages(vec![message(102)]);
        state.selected_thread_topic_index = 0;

        assert!(state.selected_chat_older_history_exhausted());
    }

    #[test]
    fn reconciliation_preserve_keeps_gap_while_replacement_clears_it() {
        let mut state = state_with_chats();
        state.folders = vec![all_folder(0)];
        state.messages = (1..=MAX_REMOTE_MESSAGES as i32).map(message).collect();
        state.selected_message_index = 0;
        state.message_scroll_offset = 0;
        state.apply_update(Update::NewMessage(message(501)));
        assert!(state.newer_history_gap());
        let context = state.reconciliation_context();
        let mut latest = message(700);
        latest.chat_id = 10;

        assert_eq!(
            state.apply_reconciliation_snapshot(
                context,
                ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(10, "Alice"), chat(20, "Bob")],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(10),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: vec![latest.clone()],
                },
            ),
            ReconciliationApply::Applied {
                conversation_replaced: false
            }
        );
        assert!(state.newer_history_gap());

        state.select_last_message();
        let replacement_context = state.reconciliation_context();
        assert_eq!(
            state.apply_reconciliation_snapshot(
                replacement_context,
                ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(10, "Alice"), chat(20, "Bob")],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(10),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: vec![latest],
                },
            ),
            ReconciliationApply::Applied {
                conversation_replaced: true
            }
        );
        assert!(!state.newer_history_gap());
    }

    #[test]
    fn reconciliation_refreshes_metadata_without_yanking_older_history() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0), folder(2, "Work", 4)];
        state.selected_folder_index = 1;
        state.chats = vec![chat(10, "Alice"), chat(20, "Work")];
        state.selected_chat_index = 1;
        state.thread_topics = vec![thread_topic(100, "General")];
        state.messages = vec![message(1), message(2), message(3)];
        for message in &mut state.messages {
            message.chat_id = 20;
            message.thread_topic_id = Some(100);
        }
        state.selected_message_index = 1;
        state.focused_panel = FocusedPanel::Messages;
        state.input_buffer = "preserved draft".to_string();
        state.chat_search_query = Some("work".to_string());
        let context = state.reconciliation_context();
        state.selected_message_index = 0;

        let mut refreshed_chat = chat_with_unread(20, "Work renamed", 3, Some(2));
        refreshed_chat.last_message = Some("new server preview".to_string());
        let mut refreshed_topic = thread_topic(100, "General renamed");
        refreshed_topic.unread_count = 2;
        let mut latest = message(9);
        latest.chat_id = 20;
        latest.thread_topic_id = Some(100);
        let applied = state.apply_reconciliation_snapshot(
            context,
            ReconciliationSnapshot {
                folders: vec![all_folder(3), folder(2, "Work", 3)],
                selected_folder_id: Some(2),
                chats: vec![refreshed_chat],
                chat_last_message_ids: Default::default(),
                selected_chat_id: Some(20),
                thread_topics: vec![refreshed_topic],
                selected_topic_id: Some(100),
                messages: vec![latest],
            },
        );

        assert_eq!(
            applied,
            ReconciliationApply::Applied {
                conversation_replaced: false
            }
        );
        assert_eq!(state.chats[0].name, "Work renamed");
        assert_eq!(
            state
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(state.selected_message().map(|message| message.id), Some(1));
        assert_eq!(state.focused_panel, FocusedPanel::Messages);
        assert_eq!(state.input_buffer, "preserved draft");
        assert_eq!(state.chat_search_query.as_deref(), Some("work"));
        assert_eq!(state.chats[0].unread_count, 3);
        assert_eq!(state.thread_topics[0].unread_count, 2);
        assert_eq!(state.folders[0].unread_count, 3);
    }

    #[test]
    fn reconciliation_cancels_stale_compose_and_modal_targets_without_losing_draft() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat(10, "Alice")];
        state.messages = vec![message(1)];
        state.messages[0].is_own = true;
        state.messages[0].can_edit = true;
        state.input_buffer = "plain draft".to_string();
        state.save_current_draft();
        state.request_edit_selected_message();
        state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 10,
            message_id: 1,
        });
        let context = state.reconciliation_context();
        let mut replacement = message(9);
        replacement.chat_id = 10;

        assert_eq!(
            state.apply_reconciliation_snapshot(
                context,
                ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(10, "Alice")],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(10),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: vec![replacement],
                },
            ),
            ReconciliationApply::Applied {
                conversation_replaced: true
            }
        );
        assert_eq!(state.input_buffer, "plain draft");
        assert!(state.editing_message_id.is_none());
        assert!(state.delete_confirmation().is_none());
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    #[test]
    fn reconciliation_topic_fallback_saves_old_scope_and_restores_destination_draft() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat(10, "Forum")];
        state.thread_topics = vec![thread_topic(101, "General"), thread_topic(102, "Old")];
        state.selected_thread_topic_index = 1;
        state.input_buffer = "old topic draft".to_string();
        state.focused_panel = FocusedPanel::Input;
        state.insert_conversation_draft(10, Some(101), "general draft");
        let context = state.reconciliation_context();
        let mut replacement = message(9);
        replacement.chat_id = 10;
        replacement.thread_topic_id = Some(101);

        assert_eq!(
            state.apply_reconciliation_snapshot(
                context,
                ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(10, "Forum")],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(10),
                    thread_topics: vec![thread_topic(101, "General")],
                    selected_topic_id: Some(101),
                    messages: vec![replacement],
                },
            ),
            ReconciliationApply::Applied {
                conversation_replaced: true
            }
        );
        assert_eq!(
            state.selected_thread_topic().map(|topic| topic.id),
            Some(101)
        );
        assert_eq!(state.input_buffer, "general draft");
        assert_eq!(
            state.conversation_draft(10, Some(102)),
            Some("old topic draft")
        );
    }

    #[test]
    fn reconciliation_topic_to_chat_wide_fallback_restores_chat_draft() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat(10, "Forum")];
        state.thread_topics = vec![thread_topic(101, "Removed")];
        state.input_buffer = "removed topic draft".to_string();
        state.insert_conversation_draft(10, None, "chat draft");
        let context = state.reconciliation_context();
        let mut replacement = message(9);
        replacement.chat_id = 10;

        assert_eq!(
            state.apply_reconciliation_snapshot(
                context,
                ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(10, "Forum")],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(10),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: vec![replacement],
                },
            ),
            ReconciliationApply::Applied {
                conversation_replaced: true
            }
        );
        assert!(state.selected_thread_topic().is_none());
        assert_eq!(state.input_buffer, "chat draft");
        assert_eq!(
            state.conversation_draft(10, Some(101)),
            Some("removed topic draft")
        );
    }

    #[test]
    fn reconciliation_falls_back_by_stable_id_and_restores_the_new_chat_draft() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat(10, "Alice"), chat(20, "Removed")];
        state.selected_chat_index = 1;
        state.input_buffer = "removed chat draft".to_string();
        state.focused_panel = FocusedPanel::Input;
        state.insert_conversation_draft(10, None, "alice draft");
        let context = state.reconciliation_context();
        let mut replacement = message(9);
        replacement.chat_id = 10;

        assert_eq!(
            state.apply_reconciliation_snapshot(
                context,
                ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(10, "Alice")],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(10),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: vec![replacement],
                },
            ),
            ReconciliationApply::Applied {
                conversation_replaced: true
            }
        );
        assert_eq!(state.selected_chat_id(), Some(10));
        assert_eq!(state.input_buffer, "alice draft");
        assert_eq!(
            state.conversation_draft(20, None),
            Some("removed chat draft")
        );
        assert_eq!(state.focused_panel, FocusedPanel::Input);
    }

    #[test]
    fn reconciliation_preserves_local_send_until_completion_and_rejects_stale_context() {
        let mut state = AppState::new();
        state.folders = vec![all_folder(0)];
        state.chats = vec![chat(10, "Alice"), chat(20, "Bob")];
        state.messages = vec![message(1)];
        state.apply_send_pending(-1, 10, None, "pending".to_string());
        state.focused_panel = FocusedPanel::Input;
        let context = state.reconciliation_context();
        let mut refreshed = message(2);
        refreshed.chat_id = 10;

        assert_eq!(
            state.apply_reconciliation_snapshot(
                context,
                ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(10, "Alice"), chat(20, "Bob")],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(10),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: vec![refreshed],
                },
            ),
            ReconciliationApply::Applied {
                conversation_replaced: true
            }
        );
        assert_eq!(state.selected_message().map(|message| message.id), Some(-1));
        assert_eq!(state.focused_panel, FocusedPanel::Input);

        let mut sent = message(3);
        sent.chat_id = 10;
        sent.is_own = true;
        state.apply_send_success(-1, sent);
        assert_eq!(
            state
                .messages
                .iter()
                .filter(|message| message.id == 3)
                .count(),
            1
        );
        assert!(!state.messages.iter().any(|message| message.id == -1));

        let stale = state.reconciliation_context();
        state.selected_chat_index = 1;
        assert_eq!(
            state.apply_reconciliation_snapshot(
                stale,
                ReconciliationSnapshot {
                    folders: vec![all_folder(0)],
                    selected_folder_id: Some(0),
                    chats: vec![chat(10, "Alice")],
                    chat_last_message_ids: Default::default(),
                    selected_chat_id: Some(10),
                    thread_topics: Vec::new(),
                    selected_topic_id: None,
                    messages: Vec::new(),
                },
            ),
            ReconciliationApply::Stale
        );
        assert_eq!(state.selected_chat_id(), Some(20));
    }
}
