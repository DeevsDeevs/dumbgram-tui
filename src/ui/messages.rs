use crate::{
    app::App,
    config::Theme,
    state::FocusedPanel,
    telegram::types::MessageStatus,
    text::{display_width, truncate_with_ellipsis},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

pub fn render_messages(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let text_width = area.width.saturating_sub(5) as usize;

    let items: Vec<ListItem> = app
        .state
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
            let content = truncate_with_ellipsis(&msg.content, content_width);

            let main_line = Line::from(vec![
                Span::styled(sender, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(content, Style::default().fg(msg_color)),
                Span::raw(metadata),
            ]);

            if let Some(reply_content) = &msg.reply_to_content {
                let reply_line = Line::from(vec![Span::styled(
                    format!(
                        "   └─ Reply: {}",
                        truncate_with_ellipsis(reply_content, text_width.saturating_sub(13))
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
        .collect();

    let chat_name = app
        .state
        .chats
        .get(app.state.selected_chat_index)
        .map(|c| c.name.as_str())
        .unwrap_or("Messages");
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
        .highlight_symbol("▶ ");

    let selected_index = if app.state.messages.is_empty() {
        None
    } else {
        Some(
            app.state
                .selected_message_index
                .min(app.state.messages.len().saturating_sub(1)),
        )
    };

    let mut list_state = ListState::default()
        .with_offset(app.state.message_scroll_offset)
        .with_selected(selected_index);
    frame.render_stateful_widget(list, area, &mut list_state);

    if app.state.delete_confirmation.is_some() {
        let popup_area = centered_rect(60, 20, area);
        let confirmation = Paragraph::new(" Delete this message? (y/n) ")
            .block(Block::default().borders(Borders::ALL).title(" Confirm "))
            .style(Style::default().bg(Color::Black).fg(Color::Red));
        frame.render_widget(Clear, popup_area);
        frame.render_widget(confirmation, popup_area);
    }
}

fn message_status_label(status: &MessageStatus, is_own: bool) -> &'static str {
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

fn message_metadata(
    time_str: &str,
    is_edited: bool,
    status_label: &str,
    error: Option<&str>,
) -> String {
    let mut parts = vec![time_str.to_string()];
    if is_edited {
        parts.push("edited".to_string());
    }
    if !status_label.is_empty() {
        parts.push(status_label.to_string());
    }
    if let Some(error) = error {
        parts.push(format!("error: {error}"));
    }

    format!(" {}", parts.join(" · "))
}

fn selected_message_position(app: &App) -> String {
    if app.state.messages.is_empty() {
        "0/0".to_string()
    } else {
        format!(
            "{}/{}",
            app.state
                .selected_message_index
                .min(app.state.messages.len() - 1)
                + 1,
            app.state.messages.len()
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

fn typing_label(users: &[String]) -> String {
    match users {
        [] => String::new(),
        [user] => format!(" · {} typing", truncate_with_ellipsis(user, 18)),
        _ => format!(" · {} typing", users.len()),
    }
}

fn message_panel_title(
    chat_name: &str,
    position_label: &str,
    typing_label: &str,
    area_width: u16,
) -> String {
    let max_title_width = area_width.saturating_sub(2) as usize;
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
        display_width, message_metadata, message_panel_title, message_status_label, typing_label,
    };
    use crate::telegram::types::MessageStatus;

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
    fn message_metadata_uses_plain_unicode_separators() {
        let metadata = message_metadata("12:34", true, "read", Some("network down"));

        assert_eq!(metadata, " 12:34 · edited · read · error: network down");
        assert!(!metadata.contains("[edited]"));
        assert!(!metadata.contains(" | "));
    }

    #[test]
    fn message_metadata_keeps_incoming_messages_unlabeled() {
        assert_eq!(message_metadata("12:34", false, "", None), " 12:34");
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
