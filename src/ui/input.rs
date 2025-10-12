use ratatui::{
    style::Style,
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::{app::App, config::Theme, state::FocusedPanel};

pub fn render_input(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let is_focused = app.state.focused_panel == FocusedPanel::Input;
    
    let title = if is_focused {
        " Type message... "
    } else {
        " Input "
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
