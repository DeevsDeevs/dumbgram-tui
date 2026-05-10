use crate::{app::App, config::Theme, state::FocusedPanel};
use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub fn render_folders(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let (visible_folders, has_left, has_right) = app.state.get_visible_folders();

    let mut spans = Vec::new();

    if has_left {
        spans.push(Span::styled("◀ ", Style::default().fg(theme.selected_item)));
    }

    for (idx, folder) in visible_folders.iter().enumerate() {
        let global_idx = app.state.folder_scroll_offset + idx;
        let is_selected = global_idx == app.state.selected_folder_index;

        if idx > 0 {
            spans.push(Span::raw(" │ "));
        }

        let style = if is_selected {
            Style::default()
                .fg(theme.selected_item)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.foreground)
        };

        spans.push(Span::styled(format!(" {} ", folder.name), style));
    }

    if has_right {
        spans.push(Span::styled(" ▶", Style::default().fg(theme.selected_item)));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if app.state.focused_panel == FocusedPanel::Folders {
                Style::default().fg(theme.border_focused)
            } else {
                Style::default().fg(theme.border)
            })
            .title(" Folders "),
    );

    frame.render_widget(paragraph, area);
}
