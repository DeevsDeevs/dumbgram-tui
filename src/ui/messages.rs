use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};
use crate::{app::App, config::Theme, state::FocusedPanel};

pub fn render_messages(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let items: Vec<ListItem> = app
        .state
        .messages
        .iter()
        .map(|msg| {
            let time_str = msg.timestamp.format("%H:%M").to_string();
            
            let edited_indicator = if msg.is_edited { " [edited]" } else { "" };
            
            let reply_indicator = if msg.reply_to_id.is_some() {
                "  └─ Reply\n"
            } else {
                ""
            };
            
            let msg_color = if msg.is_own {
                theme.own_message
            } else {
                theme.other_message
            };
            
            let content = Line::from(vec![
                Span::raw(reply_indicator),
                Span::styled(
                    format!("{}: ", msg.sender_name),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(&msg.content, Style::default().fg(msg_color)),
                Span::raw(format!(" {}{}", time_str, edited_indicator)),
            ]);
            
            ListItem::new(content)
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

    let mut list_state = ListState::default().with_selected(Some(app.state.selected_message_index));
    frame.render_stateful_widget(list, area, &mut list_state);
}
