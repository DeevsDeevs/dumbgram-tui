use crate::{
    app::App,
    config::Theme,
    state::{
        FOLDER_LEFT_SCROLL_INDICATOR, FOLDER_RIGHT_SCROLL_INDICATOR, FOLDER_SEPARATOR, FocusedPanel,
    },
};
use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub(crate) const FOLDER_PANEL_LABEL: &str = "Folders";
pub(crate) const FOLDER_PANEL_TITLE: &str = " Folders ";
pub(crate) const FOLDER_EMPTY_LABEL: &str = " No folders loaded ";

pub fn render_folders(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let (visible_folders, has_left, has_right) = app.state.get_visible_folders();

    let mut spans = Vec::new();

    if has_left {
        spans.push(Span::styled(
            FOLDER_LEFT_SCROLL_INDICATOR,
            Style::default().fg(theme.selected_item),
        ));
    }

    for (idx, folder) in visible_folders.iter().enumerate() {
        let global_idx = app.state.folder_scroll_offset + idx;
        let is_selected = global_idx == app.state.selected_folder_index;

        if idx > 0 {
            spans.push(Span::raw(FOLDER_SEPARATOR));
        }

        let style = if is_selected {
            Style::default()
                .fg(theme.selected_item)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.foreground)
        };

        let label = if folder.unread_count > 0 {
            format!(" {} ({}) ", folder.name, folder.unread_count)
        } else {
            format!(" {} ", folder.name)
        };
        spans.push(Span::styled(label, style));
    }

    if has_right {
        spans.push(Span::styled(
            FOLDER_RIGHT_SCROLL_INDICATOR,
            Style::default().fg(theme.selected_item),
        ));
    }

    if spans.is_empty() {
        spans.push(Span::raw(FOLDER_EMPTY_LABEL));
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
            .title(FOLDER_PANEL_TITLE),
    );

    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::{FOLDER_EMPTY_LABEL, FOLDER_PANEL_LABEL, FOLDER_PANEL_TITLE};

    #[test]
    fn folder_panel_title_wraps_label_for_ratatui_title() {
        assert_eq!(FOLDER_PANEL_LABEL, "Folders");
        assert_eq!(FOLDER_PANEL_TITLE, " Folders ");
    }

    #[test]
    fn folder_empty_label_is_plain_text() {
        assert_eq!(FOLDER_EMPTY_LABEL, " No folders loaded ");
    }
}
