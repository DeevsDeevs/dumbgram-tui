use super::{SELECTED_ROW_SYMBOL, list_text_width, selected_list_index};
use crate::{
    app::App,
    config::Theme,
    state::{
        ConversationLoadStatus, FOLDER_LEFT_SCROLL_INDICATOR, FOLDER_RIGHT_SCROLL_INDICATOR,
        FOLDER_SEPARATOR, FocusedPanel,
    },
    telegram::types::{MessageStatus, ThreadTopic, message_display_content},
    text::{display_width, truncate_with_ellipsis, wrap_display_lines_limited},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

pub(crate) const MESSAGE_PANEL_LABEL: &str = "Messages";
pub(crate) const MESSAGE_EMPTY_NO_CHAT_LABEL: &str = "No chat selected";
pub(crate) const MESSAGE_EMPTY_NO_MESSAGES_LABEL: &str = "No messages loaded";
pub(crate) const MESSAGE_LOADING_LABEL: &str = "Loading messages…";
pub(crate) const MESSAGE_LOAD_FAILED_LABEL: &str = "Messages failed to load";
pub(crate) const NEWER_HISTORY_GAP_LABEL: &str = " · newer gap";
pub(crate) const MESSAGE_METADATA_SEPARATOR: &str = " · ";
pub(crate) const EDITED_METADATA_LABEL: &str = "edited";
pub(crate) const REPLY_LINE_PREFIX: &str = "   ";
pub(crate) const REPLY_MARKER: &str = "└─ Reply:";
pub(crate) const REPLY_MARKER_SEPARATOR: &str = " ";
pub(crate) const DELETE_CONFIRMATION_TEXT: &str = " Delete? y yes · n/Esc/Ctrl-C cancel ";
pub(crate) const DELETE_CONFIRMATION_TITLE: &str = " Confirm ";
pub(crate) const DELETE_CONFIRMATION_POPUP_WIDTH_PERCENT: u16 = 60;
pub(crate) const DELETE_CONFIRMATION_POPUP_HEIGHT_PERCENT: u16 = 20;
pub(crate) const MESSAGE_TITLE_BORDER_RESERVED_COLUMNS: u16 = 2;
pub(crate) const THREAD_TOPICS_PANEL_TITLE: &str = " Topics ";

pub fn render_thread_topics(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    app: &App,
    theme: &Theme,
) {
    let (visible_topics, has_left, has_right) = app.state.get_visible_thread_topics();
    let mut spans = Vec::new();

    if has_left {
        spans.push(Span::styled(
            FOLDER_LEFT_SCROLL_INDICATOR,
            Style::default().fg(theme.selected_item),
        ));
    }

    for (idx, topic) in visible_topics.iter().enumerate() {
        let global_idx = app.state.thread_topic_scroll_offset + idx;

        if idx > 0 {
            spans.push(Span::raw(FOLDER_SEPARATOR));
        }

        let style = if global_idx == app.state.selected_thread_topic_index {
            Style::default()
                .fg(theme.selected_item)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.foreground)
        };
        spans.push(Span::styled(thread_topic_tab_label(topic), style));
    }

    if has_right {
        spans.push(Span::styled(
            FOLDER_RIGHT_SCROLL_INDICATOR,
            Style::default().fg(theme.selected_item),
        ));
    }

    let paragraph = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if app.state.focused_panel == FocusedPanel::Messages {
                Style::default().fg(theme.border_focused)
            } else {
                Style::default().fg(theme.border)
            })
            .title(THREAD_TOPICS_PANEL_TITLE),
    );

    frame.render_widget(paragraph, area);
}

fn thread_topic_tab_label(topic: &ThreadTopic) -> String {
    if topic.unread_count > 0 {
        format!(" {} ({}) ", topic.title, topic.unread_count)
    } else {
        format!(" {} ", topic.title)
    }
}

pub fn render_messages(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let text_width = list_text_width(area.width);

    let (items, selected_index) = if app.state.messages.is_empty() {
        (
            vec![ListItem::new(Line::from(Span::raw(
                message_empty_placeholder(
                    app.state.selected_chat_id().is_some(),
                    app.state.conversation_load_status,
                ),
            )))],
            selected_list_index(app.state.selected_message_index, 0),
        )
    } else {
        visible_message_items(app, theme, text_width, area.height)
    };

    let chat_name = app
        .state
        .chats
        .get(app.state.selected_chat_index)
        .map(|c| c.name.as_str())
        .unwrap_or(MESSAGE_PANEL_LABEL);
    let position_label = selected_message_position(app);
    let topic_label = selected_thread_topic_label(app);
    let typing_label = selected_chat_typing_label(app);
    let gap_label = if app.state.newer_history_gap() {
        NEWER_HISTORY_GAP_LABEL
    } else {
        ""
    };
    let title = message_panel_title(
        chat_name,
        &position_label,
        &topic_label,
        gap_label,
        &typing_label,
        area.width,
    );

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(
                    if app.state.focused_panel == FocusedPanel::Messages
                        || app.state.split_drag_active
                    {
                        Style::default().fg(theme.border_focused)
                    } else {
                        Style::default().fg(theme.border)
                    },
                )
                .title(title),
        )
        .highlight_style(
            Style::default()
                .fg(theme.selection_foreground)
                .bg(theme.selection)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(SELECTED_ROW_SYMBOL);

    let mut list_state = ListState::default().with_selected(selected_index);
    frame.render_stateful_widget(list, area, &mut list_state);

    if app.state.delete_confirmation().is_some() {
        let popup_area = centered_rect(
            DELETE_CONFIRMATION_POPUP_WIDTH_PERCENT,
            DELETE_CONFIRMATION_POPUP_HEIGHT_PERCENT,
            area,
        );
        let confirmation = Paragraph::new(DELETE_CONFIRMATION_TEXT)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(DELETE_CONFIRMATION_TITLE),
            )
            .style(Style::default().bg(theme.background).fg(theme.error));
        frame.render_widget(Clear, popup_area);
        frame.render_widget(confirmation, popup_area);
    }
}

pub(crate) fn message_empty_placeholder(
    has_selected_chat: bool,
    load_status: ConversationLoadStatus,
) -> &'static str {
    match load_status {
        ConversationLoadStatus::Loading => MESSAGE_LOADING_LABEL,
        ConversationLoadStatus::Failed => MESSAGE_LOAD_FAILED_LABEL,
        _ if has_selected_chat => MESSAGE_EMPTY_NO_MESSAGES_LABEL,
        _ => MESSAGE_EMPTY_NO_CHAT_LABEL,
    }
}

pub(crate) fn message_status_label(status: &MessageStatus, is_own: bool) -> &'static str {
    if !is_own {
        return "";
    }

    match status {
        MessageStatus::Sending => "sending",
        MessageStatus::Sent | MessageStatus::Delivered => "✓",
        MessageStatus::Read => "✓✓",
        MessageStatus::Failed => "failed",
    }
}

fn visible_message_items(
    app: &App,
    theme: &Theme,
    text_width: usize,
    area_height: u16,
) -> (Vec<ListItem<'static>>, Option<usize>) {
    let capacity = area_height.saturating_sub(crate::state::PANEL_BORDER_RESERVED_ROWS) as usize;
    let mut remaining_rows = capacity.max(1);
    let mut items = Vec::new();
    let mut selected_index = None;

    for (idx, msg) in app
        .state
        .messages
        .iter()
        .enumerate()
        .skip(app.state.message_scroll_offset)
    {
        if remaining_rows == 0 {
            break;
        }
        if idx == app.state.selected_message_index {
            selected_index = Some(items.len());
        }

        let (item, row_count) = message_item(msg, theme, text_width, remaining_rows);
        items.push(item);
        remaining_rows = remaining_rows.saturating_sub(row_count);
    }

    if selected_index.is_none()
        && app.state.selected_message_index >= app.state.message_scroll_offset
    {
        selected_index = selected_list_index(
            app.state
                .selected_message_index
                .saturating_sub(app.state.message_scroll_offset),
            items.len(),
        );
    }

    (items, selected_index)
}

fn message_item(
    msg: &crate::telegram::types::Message,
    theme: &Theme,
    text_width: usize,
    max_rows: usize,
) -> (ListItem<'static>, usize) {
    let time_str = msg.timestamp.format("%H:%M").to_string();
    let status_label = message_status_label(&msg.status, msg.is_own);
    let metadata = message_metadata(&time_str, msg.is_edited, status_label, msg.error.as_deref());
    let msg_color = if msg.status == MessageStatus::Failed {
        ratatui::style::Color::Red
    } else if msg.is_own {
        theme.own_message
    } else {
        theme.other_message
    };
    let reply_rows = usize::from(msg.reply_to_content.is_some() && max_rows > 1);
    let content_rows = max_rows.saturating_sub(reply_rows).max(1);
    let mut lines = message_lines(
        &msg.sender_name,
        msg.media.as_ref(),
        &msg.content,
        &metadata,
        msg_color,
        text_width,
        content_rows,
    );

    if let Some(reply_content) = &msg.reply_to_content
        && lines.len() < max_rows
    {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "{REPLY_LINE_PREFIX}{REPLY_MARKER}{REPLY_MARKER_SEPARATOR}{}",
                truncate_with_ellipsis(reply_content, reply_content_width(text_width))
            ),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )]));
    }

    let row_count = lines.len();
    (ListItem::new(lines), row_count)
}

fn message_lines(
    sender_name: &str,
    media: Option<&crate::telegram::types::MessageMedia>,
    content: &str,
    metadata: &str,
    msg_color: Color,
    text_width: usize,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let sender = format!("{sender_name}: ");
    let first_content_width = text_width
        .saturating_sub(display_width(&sender) + display_width(metadata))
        .max(1);
    let continuation_width = text_width.saturating_sub(display_width(&sender)).max(1);
    let display_content = message_display_content(media, content);
    let wrapped_content = wrap_display_lines_limited(
        &display_content,
        first_content_width,
        continuation_width,
        max_lines,
    );

    wrapped_content
        .into_iter()
        .enumerate()
        .map(|(index, content_line)| {
            if index == 0 {
                let mut spans = vec![Span::styled(
                    sender.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )];
                spans.extend(message_content_spans(&content_line, msg_color));
                spans.push(Span::raw(metadata.to_string()));
                Line::from(spans)
            } else {
                let mut spans = vec![Span::raw(" ".repeat(display_width(&sender)))];
                spans.extend(message_content_spans(&content_line, msg_color));
                Line::from(spans)
            }
        })
        .collect()
}

fn reply_content_width(text_width: usize) -> usize {
    text_width.saturating_sub(
        display_width(REPLY_LINE_PREFIX)
            + display_width(REPLY_MARKER)
            + display_width(REPLY_MARKER_SEPARATOR),
    )
}

pub(crate) fn message_content_spans(content: &str, default_color: Color) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    for link in crate::links::links_in_text(content) {
        if cursor < link.start {
            spans.push(Span::styled(
                content[cursor..link.start].to_string(),
                Style::default().fg(default_color),
            ));
        }
        spans.push(Span::styled(
            content[link.start..link.end].to_string(),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::UNDERLINED),
        ));
        cursor = link.end;
    }
    if cursor < content.len() {
        spans.push(Span::styled(
            content[cursor..].to_string(),
            Style::default().fg(default_color),
        ));
    }
    if spans.is_empty() {
        spans.push(Span::styled("", Style::default().fg(default_color)));
    }
    spans
}

pub(crate) fn message_metadata(
    time_str: &str,
    is_edited: bool,
    status_label: &str,
    error: Option<&str>,
) -> String {
    let mut parts = vec![time_str.to_string()];
    if is_edited {
        parts.push(EDITED_METADATA_LABEL.to_string());
    }
    if !status_label.is_empty() {
        parts.push(status_label.to_string());
    }
    if let Some(error) = error {
        parts.push(format!("error: {error}"));
    }

    format!(" {}", parts.join(MESSAGE_METADATA_SEPARATOR))
}

fn selected_message_position(app: &App) -> String {
    message_position_label(app.state.selected_message_index, app.state.messages.len())
}

pub(crate) fn message_position_label(selected_index: usize, message_count: usize) -> String {
    if message_count == 0 {
        "0/0".to_string()
    } else {
        format!(
            "{}/{}",
            selected_index.min(message_count - 1) + 1,
            message_count
        )
    }
}

fn selected_chat_typing_label(app: &App) -> String {
    app.state
        .selected_typing_users()
        .map(|users| typing_label(users))
        .unwrap_or_default()
}

fn selected_thread_topic_label(app: &App) -> String {
    app.state
        .selected_thread_topic()
        .map(|topic| {
            thread_topic_label(
                &topic.title,
                app.state.selected_thread_topic_index,
                app.state.thread_topics.len(),
            )
        })
        .unwrap_or_default()
}

pub(crate) fn thread_topic_label(title: &str, selected_index: usize, topic_count: usize) -> String {
    if topic_count == 0 {
        String::new()
    } else {
        format!(
            "{}#{} {}/{}",
            MESSAGE_METADATA_SEPARATOR,
            truncate_with_ellipsis(title, 18),
            selected_index.min(topic_count - 1) + 1,
            topic_count
        )
    }
}

pub(crate) fn typing_label(users: &[String]) -> String {
    match users {
        [] => String::new(),
        [user] => format!(
            "{}{} typing",
            MESSAGE_METADATA_SEPARATOR,
            truncate_with_ellipsis(user, 18)
        ),
        _ => format!("{}{} typing", MESSAGE_METADATA_SEPARATOR, users.len()),
    }
}

fn message_panel_title(
    chat_name: &str,
    position_label: &str,
    topic_label: &str,
    gap_label: &str,
    typing_label: &str,
    area_width: u16,
) -> String {
    let max_title_width = message_title_width(area_width);
    if max_title_width == 0 {
        return String::new();
    }

    let suffix = format!(
        " {}{}{}{}",
        gap_label, position_label, topic_label, typing_label
    );
    let chat_name_width = max_title_width
        .saturating_sub(display_width(&suffix) + 2)
        .max(1);
    let title = format!(
        " {}{} ",
        truncate_with_ellipsis(chat_name, chat_name_width),
        suffix
    );

    truncate_with_ellipsis(&title, max_title_width)
}

fn message_title_width(area_width: u16) -> usize {
    area_width.saturating_sub(MESSAGE_TITLE_BORDER_RESERVED_COLUMNS) as usize
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::{
        DELETE_CONFIRMATION_POPUP_HEIGHT_PERCENT, DELETE_CONFIRMATION_POPUP_WIDTH_PERCENT,
        DELETE_CONFIRMATION_TEXT, DELETE_CONFIRMATION_TITLE, MESSAGE_EMPTY_NO_CHAT_LABEL,
        MESSAGE_EMPTY_NO_MESSAGES_LABEL, MESSAGE_LOAD_FAILED_LABEL, MESSAGE_LOADING_LABEL,
        MESSAGE_TITLE_BORDER_RESERVED_COLUMNS, NEWER_HISTORY_GAP_LABEL, REPLY_LINE_PREFIX,
        REPLY_MARKER, REPLY_MARKER_SEPARATOR, THREAD_TOPICS_PANEL_TITLE, display_width,
        message_content_spans, message_empty_placeholder, message_lines, message_metadata,
        message_panel_title, message_position_label, message_status_label, message_title_width,
        reply_content_width, thread_topic_label, thread_topic_tab_label, typing_label,
    };
    use crate::{
        state::ConversationLoadStatus,
        telegram::types::{MessageStatus, ThreadTopic},
    };

    #[test]
    fn message_position_label_shows_clamped_position() {
        assert_eq!(message_position_label(0, 0), "0/0");
        assert_eq!(message_position_label(0, 3), "1/3");
        assert_eq!(message_position_label(99, 3), "3/3");
    }

    #[test]
    fn thread_topic_tab_label_matches_folder_tab_style() {
        assert_eq!(THREAD_TOPICS_PANEL_TITLE, " Topics ");
        assert_eq!(
            thread_topic_tab_label(&ThreadTopic {
                id: 101,
                title: "General".to_string(),
                top_message_id: 101,
                unread_count: 2,
                is_closed: false,
                is_pinned: true,
            }),
            " General (2) "
        );
        assert_eq!(
            thread_topic_tab_label(&ThreadTopic {
                id: 102,
                title: "Deployments".to_string(),
                top_message_id: 102,
                unread_count: 0,
                is_closed: false,
                is_pinned: false,
            }),
            " Deployments "
        );
    }

    #[test]
    fn message_empty_placeholder_distinguishes_load_states() {
        assert_eq!(
            message_empty_placeholder(false, ConversationLoadStatus::Idle),
            MESSAGE_EMPTY_NO_CHAT_LABEL
        );
        assert_eq!(
            message_empty_placeholder(true, ConversationLoadStatus::Empty),
            MESSAGE_EMPTY_NO_MESSAGES_LABEL
        );
        assert_eq!(
            message_empty_placeholder(true, ConversationLoadStatus::Loading),
            MESSAGE_LOADING_LABEL
        );
        assert_eq!(
            message_empty_placeholder(true, ConversationLoadStatus::Failed),
            MESSAGE_LOAD_FAILED_LABEL
        );
    }

    #[test]
    fn delete_confirmation_text_stays_plain_and_keyboard_discoverable() {
        assert_eq!(DELETE_CONFIRMATION_TITLE, " Confirm ");
        assert_eq!(
            DELETE_CONFIRMATION_TEXT,
            " Delete? y yes · n/Esc/Ctrl-C cancel "
        );
        assert_eq!(DELETE_CONFIRMATION_POPUP_WIDTH_PERCENT, 60);
        assert_eq!(DELETE_CONFIRMATION_POPUP_HEIGHT_PERCENT, 20);
        assert!(DELETE_CONFIRMATION_TEXT.contains("y yes"));
        assert!(DELETE_CONFIRMATION_TEXT.contains("n/Esc/Ctrl-C cancel"));
    }

    #[test]
    fn message_status_labels_use_compact_checkmarks() {
        assert_eq!(
            message_status_label(&MessageStatus::Sending, true),
            "sending"
        );
        assert_eq!(message_status_label(&MessageStatus::Sent, true), "✓");
        assert_eq!(message_status_label(&MessageStatus::Delivered, true), "✓");
        assert_eq!(message_status_label(&MessageStatus::Read, true), "✓✓");
        assert_eq!(message_status_label(&MessageStatus::Failed, true), "failed");
    }

    #[test]
    fn message_status_labels_are_hidden_for_incoming_messages() {
        assert_eq!(message_status_label(&MessageStatus::Delivered, false), "");
        assert_eq!(message_status_label(&MessageStatus::Read, false), "");
    }

    #[test]
    fn message_status_labels_distinguish_sent_from_read() {
        assert_ne!(
            message_status_label(&MessageStatus::Sent, true),
            message_status_label(&MessageStatus::Read, true)
        );
    }

    #[test]
    fn reply_content_width_reserves_visible_reply_prefix() {
        let prefix_width = display_width(REPLY_LINE_PREFIX)
            + display_width(REPLY_MARKER)
            + display_width(REPLY_MARKER_SEPARATOR);

        assert_eq!(prefix_width, 13);
        assert_eq!(reply_content_width(30), 17);
        assert_eq!(reply_content_width(8), 0);
    }

    #[test]
    fn message_metadata_uses_plain_unicode_separators() {
        let metadata = message_metadata("12:34", true, "✓✓", Some("network down"));

        assert_eq!(metadata, " 12:34 · edited · ✓✓ · error: network down");
        assert!(!metadata.contains("[edited]"));
        assert!(!metadata.contains(" | "));
    }

    #[test]
    fn message_lines_wrap_long_content_without_ellipsis() {
        let lines = message_lines(
            "Alice",
            None,
            "abcdefghijklmnopqrstuvwxyz",
            " 12:34",
            ratatui::style::Color::Gray,
            20,
            usize::MAX,
        );
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let letters_only = rendered
            .chars()
            .filter(|ch| ch.is_ascii_lowercase())
            .collect::<String>();

        assert!(lines.len() > 1);
        assert!(letters_only.ends_with("abcdefghijklmnopqrstuvwxyz"));
        assert!(!rendered.contains('…'));
    }

    #[test]
    fn message_content_spans_style_urls_without_changing_text() {
        let spans =
            message_content_spans("see https://example.org now", ratatui::style::Color::Gray);
        let rendered = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, "see https://example.org now");
        assert!(spans.iter().any(|span| {
            span.content.as_ref() == "https://example.org"
                && span.style.fg == Some(ratatui::style::Color::Blue)
                && span
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::UNDERLINED)
        }));
    }

    #[test]
    fn message_metadata_keeps_incoming_messages_unlabeled() {
        assert_eq!(message_metadata("12:34", false, "", None), " 12:34");
    }

    #[test]
    fn message_title_width_reserves_border_columns() {
        assert_eq!(MESSAGE_TITLE_BORDER_RESERVED_COLUMNS, 2);
        assert_eq!(message_title_width(24), 22);
        assert_eq!(message_title_width(1), 0);
    }

    #[test]
    fn message_title_truncates_long_chat_name_and_keeps_position() {
        let title = message_panel_title(
            "A very long chat name that would otherwise crowd the title",
            "3/12",
            "",
            "",
            "",
            24,
        );

        assert!(title.contains('…'));
        assert!(title.contains("3/12"));
        assert!(display_width(&title) <= 22);
    }

    #[test]
    fn message_title_keeps_newer_gap_marker_when_narrow() {
        let title = message_panel_title(
            "A long chat name",
            "500/500",
            " · #Long topic 3/9",
            NEWER_HISTORY_GAP_LABEL,
            " · typing",
            18,
        );

        assert!(title.contains("newer"));
        assert!(display_width(&title) <= 16);
    }

    #[test]
    fn message_title_uses_display_width_for_wide_chat_name() {
        let title = message_panel_title("好好好 chat", "3/12", "", "", " · 好 typing", 18);

        assert!(title.contains("3/12"));
        assert!(display_width(&title) <= 16);
    }

    #[test]
    fn thread_topic_label_shows_selected_topic_position() {
        let label = thread_topic_label("Deployments", 1, 3);

        assert_eq!(label, " · #Deployments 2/3");
    }

    #[test]
    fn thread_topic_label_truncates_long_topic_names() {
        let label = thread_topic_label("A very long topic name that will not fit", 0, 2);

        assert!(label.contains('…'));
        assert!(label.ends_with(" 1/2"));
    }

    #[test]
    fn typing_label_truncates_long_single_user_name() {
        let label = typing_label(&["Alexandria Catherine Montgomery".to_string()]);

        assert!(label.contains('…'));
        assert!(label.ends_with(" typing"));
    }

    #[test]
    fn typing_label_summarizes_multiple_users() {
        let label = typing_label(&["Alice".to_string(), "Bob".to_string()]);

        assert_eq!(label, " · 2 typing");
    }
}
