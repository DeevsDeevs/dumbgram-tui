use crate::{app::App, config::Theme, state::FocusedPanel};
use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

pub fn render_chats(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let items: Vec<ListItem> = app
        .state
        .chats
        .iter()
        .map(|chat| {
            let unread_indicator = if chat.unread_count > 0 {
                format!("[{}] ", chat.unread_count)
            } else {
                String::new()
            };

            let group_indicator = if chat.is_group { "[G] " } else { "" };

            let last_msg = chat.last_message.as_deref().unwrap_or("");

            let lines = vec![
                Line::from(vec![
                    Span::raw(unread_indicator),
                    Span::raw(group_indicator),
                    Span::styled(&chat.name, Style::default().add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![Span::raw("  "), Span::raw(last_msg)]),
            ];

            ListItem::new(lines)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.state.focused_panel == FocusedPanel::Chats {
                    Style::default().fg(theme.border_focused)
                } else {
                    Style::default().fg(theme.border)
                })
                .title(" Chats "),
        )
        .highlight_style(
            Style::default()
                .bg(theme.selection)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    let selected_index = if app.state.chats.is_empty() {
        None
    } else {
        Some(
            app.state
                .selected_chat_index
                .min(app.state.chats.len().saturating_sub(1)),
        )
    };

    let mut list_state = ListState::default().with_selected(selected_index);
    frame.render_stateful_widget(list, area, &mut list_state);
}
