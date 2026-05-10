use crate::{app::App, config::Theme, state::FocusedPanel, text::truncate_with_ellipsis};
use ratatui::{
    Frame,
    layout::Position,
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

pub fn render_input(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let is_focused = app.state.focused_panel == FocusedPanel::Input;

    let title = if let Some(msg_id) = app.state.editing_message_id {
        format!(" Editing message #{} ", msg_id)
    } else if let Some(reply_id) = app.state.replying_to_message_id {
        if let Some(msg) = app.state.messages.iter().find(|m| m.id == reply_id) {
            let preview = truncate_with_ellipsis(&msg.content, 20);
            format!(" Replying to {}: {} ", msg.sender_name, preview)
        } else {
            " Replying… ".to_string()
        }
    } else if is_focused {
        " Type message… ".to_string()
    } else {
        " Input ".to_string()
    };

    let visible_text = app.state.visible_input_text();
    let paragraph = Paragraph::new(visible_text)
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

    if is_focused {
        let cursor_x = app.state.visible_input_cursor_column() as u16;
        frame.set_cursor_position(Position::new(
            area.x.saturating_add(1).saturating_add(cursor_x),
            area.y.saturating_add(1),
        ));
    }
}
