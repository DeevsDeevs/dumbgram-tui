use super::{SELECTED_ROW_SYMBOL, list_text_width, selected_list_index};
use crate::{
    app::App,
    config::Theme,
    state::FocusedPanel,
    telegram::types::{MessageStatus, message_display_content},
    text::{display_width, truncate_with_ellipsis},
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

pub fn render_messages(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let text_width = list_text_width(area.width);

    let items: Vec<ListItem> = if app.state.messages.is_empty() {
        vec![ListItem::new(Line::from(Span::raw(
            message_empty_placeholder(app.state.selected_chat_id().is_some()),
        )))]
    } else {
        app.state
            .messages
            .iter()
            .map(|msg| {
                let time_str = msg.timestamp.format("%H:%M").to_string();

                let status_label = message_status_label(&msg.status, msg.is_own);

                let metadata =
                    message_metadata(&time_str, msg.is_edited, status_label, msg.error.as_deref());

                let msg_color = if msg.status == MessageStatus::Failed {
                    ratatui::style::Color::Red
                } else if msg.is_own {
                    theme.own_message
                } else {
                    theme.other_message
                };

                let sender = format!("{}: ", msg.sender_name);
                let content_width = text_width
                    .saturating_sub(display_width(&sender) + display_width(&metadata))
                    .max(1);
                let display_content = message_display_content(msg.media.as_ref(), &msg.content);
                let content = truncate_with_ellipsis(&display_content, content_width);

                let mut main_spans = vec![Span::styled(
                    sender,
                    Style::default().add_modifier(Modifier::BOLD),
                )];
                main_spans.extend(message_content_spans(&content, msg_color));
                main_spans.push(Span::raw(metadata));
                let main_line = Line::from(main_spans);

                if let Some(reply_content) = &msg.reply_to_content {
                    let reply_line = Line::from(vec![Span::styled(
                        format!(
                            "{REPLY_LINE_PREFIX}{REPLY_MARKER}{REPLY_MARKER_SEPARATOR}{}",
                            truncate_with_ellipsis(reply_content, reply_content_width(text_width))
                        ),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )]);
                    ListItem::new(vec![main_line, reply_line])
                } else {
                    ListItem::new(main_line)
                }
            })
            .collect()
    };

    let chat_name = app
        .state
        .chats
        .get(app.state.selected_chat_index)
        .map(|c| c.name.as_str())
        .unwrap_or(MESSAGE_PANEL_LABEL);
    let position_label = selected_message_position(app);
    let typing_label = selected_chat_typing_label(app);
    let title = message_panel_title(chat_name, &position_label, &typing_label, area.width);

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.state.focused_panel == FocusedPanel::Messages {
                    Style::default().fg(theme.border_focused)
                } else {
                    Style::default().fg(theme.border)
                })
                .title(title),
        )
        .highlight_style(
            Style::default()
                .bg(theme.selection)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(SELECTED_ROW_SYMBOL);

    let selected_index =
        selected_list_index(app.state.selected_message_index, app.state.messages.len());

    let mut list_state = ListState::default()
        .with_offset(app.state.message_scroll_offset)
        .with_selected(selected_index);
    frame.render_stateful_widget(list, area, &mut list_state);

    if app.state.delete_confirmation.is_some() {
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
            .style(Style::default().bg(Color::Black).fg(Color::Red));
        frame.render_widget(Clear, popup_area);
        frame.render_widget(confirmation, popup_area);
    }
}

pub(crate) fn message_empty_placeholder(has_selected_chat: bool) -> &'static str {
    if has_selected_chat {
        MESSAGE_EMPTY_NO_MESSAGES_LABEL
    } else {
        MESSAGE_EMPTY_NO_CHAT_LABEL
    }
}

pub(crate) fn message_status_label(status: &MessageStatus, is_own: bool) -> &'static str {
    if !is_own {
        return "";
    }

    match status {
        MessageStatus::Sending => "sending",
        MessageStatus::Sent => "sent",
        MessageStatus::Delivered => "delivered",
        MessageStatus::Read => "read",
        MessageStatus::Failed => "failed",
    }
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
        .selected_chat_id()
        .and_then(|chat_id| app.state.typing_users.get(&chat_id))
        .map(|users| typing_label(users))
        .unwrap_or_default()
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
    typing_label: &str,
    area_width: u16,
) -> String {
    let max_title_width = message_title_width(area_width);
    if max_title_width == 0 {
        return String::new();
    }

    let suffix = format!(" {}{}", position_label, typing_label);
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
        MESSAGE_EMPTY_NO_MESSAGES_LABEL, MESSAGE_TITLE_BORDER_RESERVED_COLUMNS, REPLY_LINE_PREFIX,
        REPLY_MARKER, REPLY_MARKER_SEPARATOR, display_width, message_content_spans,
        message_empty_placeholder, message_metadata, message_panel_title, message_position_label,
        message_status_label, message_title_width, reply_content_width, typing_label,
    };
    use crate::telegram::types::MessageStatus;

    #[test]
    fn message_position_label_shows_clamped_position() {
        assert_eq!(message_position_label(0, 0), "0/0");
        assert_eq!(message_position_label(0, 3), "1/3");
        assert_eq!(message_position_label(99, 3), "3/3");
    }

    #[test]
    fn message_empty_placeholder_distinguishes_missing_chat_from_empty_history() {
        assert_eq!(
            message_empty_placeholder(false),
            MESSAGE_EMPTY_NO_CHAT_LABEL
        );
        assert_eq!(
            message_empty_placeholder(true),
            MESSAGE_EMPTY_NO_MESSAGES_LABEL
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
    fn message_status_labels_are_explicit_and_non_emoji() {
        assert_eq!(
            message_status_label(&MessageStatus::Sending, true),
            "sending"
        );
        assert_eq!(message_status_label(&MessageStatus::Sent, true), "sent");
        assert_eq!(
            message_status_label(&MessageStatus::Delivered, true),
            "delivered"
        );
        assert_eq!(message_status_label(&MessageStatus::Read, true), "read");
        assert_eq!(message_status_label(&MessageStatus::Failed, true), "failed");
    }

    #[test]
    fn message_status_labels_are_hidden_for_incoming_messages() {
        assert_eq!(message_status_label(&MessageStatus::Delivered, false), "");
        assert_eq!(message_status_label(&MessageStatus::Read, false), "");
    }

    #[test]
    fn message_status_labels_distinguish_delivery_from_read() {
        assert_ne!(
            message_status_label(&MessageStatus::Delivered, true),
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
        let metadata = message_metadata("12:34", true, "read", Some("network down"));

        assert_eq!(metadata, " 12:34 · edited · read · error: network down");
        assert!(!metadata.contains("[edited]"));
        assert!(!metadata.contains(" | "));
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
            24,
        );

        assert!(title.contains('…'));
        assert!(title.contains("3/12"));
        assert!(display_width(&title) <= 22);
    }

    #[test]
    fn message_title_uses_display_width_for_wide_chat_name() {
        let title = message_panel_title("好好好 chat", "3/12", " · 好 typing", 18);

        assert!(title.contains("3/12"));
        assert!(display_width(&title) <= 16);
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
