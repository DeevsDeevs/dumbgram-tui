use crate::{app::App, config::Theme, state::FocusedPanel};
use ratatui::{
    Frame,
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

pub fn render_input(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let is_focused = app.state.focused_panel == FocusedPanel::Input;

    let title = if let Some(msg_id) = app.state.editing_message_id {
        format!(" Editing message #{} ", msg_id)
    } else if let Some(reply_id) = app.state.replying_to_message_id {
        if let Some(msg) = app.state.messages.iter().find(|m| m.id == reply_id) {
            let preview: String = msg.content.chars().take(20).collect();
            format!(" Replying to {}: {} ", msg.sender_name, preview)
        } else {
            " Replying... ".to_string()
        }
    } else if is_focused {
        " Type message... ".to_string()
    } else {
        " Input ".to_string()
    };

    let paragraph = Paragraph::new(app.state.input_buffer.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if is_focused {
                    Style::default().fg(theme.border_focused)
                } else {
                    Style::default().fg(theme.border)
                })
                .title(title),
        )
        .style(Style::default().fg(theme.foreground));

    frame.render_widget(paragraph, area);
}
