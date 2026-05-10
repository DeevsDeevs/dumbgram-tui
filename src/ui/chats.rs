use crate::{
    app::App,
    config::Theme,
    state::FocusedPanel,
    text::{display_width, truncate_with_ellipsis},
};
use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

pub fn render_chats(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let text_width = area.width.saturating_sub(5) as usize;

    let items: Vec<ListItem> = app
        .state
        .chats
        .iter()
        .map(|chat| {
            let unread_indicator = chat_unread_indicator(chat.unread_count);
            let group_indicator = chat_group_indicator(chat.is_group);

            let name_prefix_width =
                display_width(&unread_indicator) + display_width(group_indicator);
            let name_width = text_width.saturating_sub(name_prefix_width).max(1);
            let name = truncate_with_ellipsis(&chat.name, name_width);
            let last_msg = truncate_with_ellipsis(
                chat.last_message.as_deref().unwrap_or(""),
                text_width.saturating_sub(2),
            );

            let lines = vec![
                Line::from(vec![
                    Span::styled(unread_indicator, Style::default().fg(theme.unread_chat)),
                    Span::raw(group_indicator),
                    Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![Span::raw("  "), Span::raw(last_msg)]),
            ];

            ListItem::new(lines)
        })
        .collect();

    let title = if app.state.chats.is_empty() {
        " Chats 0/0 ".to_string()
    } else {
        format!(
            " Chats {}/{} ",
            app.state.selected_chat_index.min(app.state.chats.len() - 1) + 1,
            app.state.chats.len()
        )
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.state.focused_panel == FocusedPanel::Chats {
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

    let selected_index = if app.state.chats.is_empty() {
        None
    } else {
        Some(
            app.state
                .selected_chat_index
                .min(app.state.chats.len().saturating_sub(1)),
        )
    };

    let mut list_state = ListState::default()
        .with_offset(app.state.chat_scroll_offset)
        .with_selected(selected_index);
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn chat_unread_indicator(unread_count: usize) -> String {
    if unread_count > 0 {
        format!("{} unread · ", unread_count)
    } else {
        String::new()
    }
}

fn chat_group_indicator(is_group: bool) -> &'static str {
    if is_group { "group · " } else { "" }
}

#[cfg(test)]
mod tests {
    use super::{chat_group_indicator, chat_unread_indicator};

    #[test]
    fn chat_metadata_indicators_use_plain_middle_dot_text() {
        assert_eq!(chat_unread_indicator(3), "3 unread · ");
        assert_eq!(chat_group_indicator(true), "group · ");
    }

    #[test]
    fn chat_metadata_indicators_omit_empty_state() {
        assert_eq!(chat_unread_indicator(0), "");
        assert_eq!(chat_group_indicator(false), "");
    }
}
