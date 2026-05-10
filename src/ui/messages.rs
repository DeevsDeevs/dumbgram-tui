use crate::{app::App, config::Theme, state::FocusedPanel, telegram::types::MessageStatus};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

pub fn render_messages(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let items: Vec<ListItem> = app
        .state
        .messages
        .iter()
        .map(|msg| {
            let time_str = msg.timestamp.format("%H:%M").to_string();

            let status_icon = match msg.status {
                MessageStatus::Sending => "⏱",
                MessageStatus::Sent => "✓",
                MessageStatus::Delivered => "✓✓",
                MessageStatus::Read => "✓✓",
                MessageStatus::Failed => "❌",
            };

            let edited_indicator = if msg.is_edited { " [edited]" } else { "" };

            let error_text = if let Some(err) = &msg.error {
                format!(" ({})", err)
            } else {
                String::new()
            };

            let msg_color = if msg.status == MessageStatus::Failed {
                ratatui::style::Color::Red
            } else if msg.is_own {
                theme.own_message
            } else {
                theme.other_message
            };

            let main_line = Line::from(vec![
                Span::styled(
                    format!("{}: ", msg.sender_name),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(&msg.content, Style::default().fg(msg_color)),
                Span::raw(format!(
                    " {}{} {}{}",
                    time_str, edited_indicator, status_icon, error_text
                )),
            ]);

            if let Some(reply_content) = &msg.reply_to_content {
                let reply_line = Line::from(vec![Span::styled(
                    format!(
                        "   └─ Reply: {}",
                        reply_content.chars().take(40).collect::<String>()
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

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.state.focused_panel == FocusedPanel::Messages {
                    Style::default().fg(theme.border_focused)
                } else {
                    Style::default().fg(theme.border)
                })
                .title(format!(" {} ", chat_name)),
        )
        .highlight_style(
            Style::default()
                .bg(theme.selection)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    let selected_index = if app.state.messages.is_empty() {
        None
    } else {
        Some(
            app.state
                .selected_message_index
                .min(app.state.messages.len().saturating_sub(1)),
        )
    };

    let mut list_state = ListState::default().with_selected(selected_index);
    frame.render_stateful_widget(list, area, &mut list_state);

    if app.state.confirm_delete_message_id.is_some() {
        let popup_area = centered_rect(60, 20, area);
        let confirmation = Paragraph::new(" Delete this message? (y/n) ")
            .block(Block::default().borders(Borders::ALL).title(" Confirm "))
            .style(Style::default().bg(Color::Black).fg(Color::Red));
        frame.render_widget(Clear, popup_area);
        frame.render_widget(confirmation, popup_area);
    }
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
