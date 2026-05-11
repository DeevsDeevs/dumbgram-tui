use super::{SELECTED_ROW_SYMBOL, list_text_width, selected_list_index};
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

pub(crate) const CHAT_PANEL_LABEL: &str = "Chats";
pub(crate) const CHAT_EMPTY_NO_FOLDER_LABEL: &str = "No folder selected";
pub(crate) const CHAT_EMPTY_NO_CHATS_LABEL: &str = "No chats loaded";
pub(crate) const CHAT_SEARCH_NO_MATCHES_LABEL: &str = "No matching loaded chats";
pub(crate) const CHAT_EMPTY_PREVIEW_LABEL: &str = "No message preview";
pub(crate) const CHAT_PREVIEW_PREFIX: &str = "  ";

pub fn render_chats(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let text_width = list_text_width(area.width);

    let display_indices = app.state.chat_display_indices();
    let items: Vec<ListItem> = if app.state.chats.is_empty() {
        vec![ListItem::new(Line::from(Span::raw(
            chat_empty_placeholder(!app.state.folders.is_empty()),
        )))]
    } else if app.state.chat_search_active() && display_indices.is_empty() {
        vec![ListItem::new(Line::from(Span::raw(
            chat_search_empty_placeholder(),
        )))]
    } else {
        display_indices
            .iter()
            .map(|&chat_index| {
                let chat = &app.state.chats[chat_index];
                let unread_indicator = chat_unread_indicator(chat.unread_count);
                let group_indicator = chat_group_indicator(chat.is_group);

                let name_prefix_width =
                    display_width(&unread_indicator) + display_width(group_indicator);
                let name_width = text_width.saturating_sub(name_prefix_width).max(1);
                let name = truncate_with_ellipsis(&chat.name, name_width);
                let last_msg = truncate_with_ellipsis(
                    chat_preview_text(chat.last_message.as_deref()),
                    chat_preview_width(text_width),
                );

                let lines = vec![
                    Line::from(vec![
                        Span::styled(unread_indicator, Style::default().fg(theme.unread_chat)),
                        Span::raw(group_indicator),
                        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![Span::raw(CHAT_PREVIEW_PREFIX), Span::raw(last_msg)]),
                ];

                ListItem::new(lines)
            })
            .collect()
    };

    let title = if app.state.chat_search_active() {
        chat_search_panel_title(
            app.state.chat_search_query(),
            app.state.selected_chat_display_index().unwrap_or(0),
            display_indices.len(),
        )
    } else {
        chat_panel_title(app.state.selected_chat_index, app.state.chats.len())
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
        .highlight_symbol(SELECTED_ROW_SYMBOL);

    let selected_index = if app.state.chat_search_active() {
        selected_list_index(
            app.state.selected_chat_display_index().unwrap_or(0),
            display_indices.len(),
        )
    } else {
        selected_list_index(app.state.selected_chat_index, app.state.chats.len())
    };
    let offset = if app.state.chat_search_active() {
        app.state.chat_search_scroll_offset
    } else {
        app.state.chat_scroll_offset
    };

    let mut list_state = ListState::default()
        .with_offset(offset)
        .with_selected(selected_index);
    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(crate) fn chat_search_panel_title(
    query: &str,
    selected_index: usize,
    chat_count: usize,
) -> String {
    let query = if query.is_empty() { "type…" } else { query };
    if chat_count == 0 {
        format!(" {CHAT_PANEL_LABEL} /{query} 0/0 ")
    } else {
        format!(
            " {CHAT_PANEL_LABEL} /{query} {}/{} ",
            selected_index.min(chat_count - 1) + 1,
            chat_count
        )
    }
}

pub(crate) fn chat_panel_title(selected_index: usize, chat_count: usize) -> String {
    if chat_count == 0 {
        format!(" {CHAT_PANEL_LABEL} 0/0 ")
    } else {
        format!(
            " {CHAT_PANEL_LABEL} {}/{} ",
            selected_index.min(chat_count - 1) + 1,
            chat_count
        )
    }
}

pub(crate) fn chat_unread_indicator(unread_count: usize) -> String {
    if unread_count > 0 {
        format!("{} unread · ", unread_count)
    } else {
        String::new()
    }
}

pub(crate) fn chat_group_indicator(is_group: bool) -> &'static str {
    if is_group { "group · " } else { "" }
}

pub(crate) fn chat_empty_placeholder(has_selected_folder: bool) -> &'static str {
    if has_selected_folder {
        CHAT_EMPTY_NO_CHATS_LABEL
    } else {
        CHAT_EMPTY_NO_FOLDER_LABEL
    }
}

pub(crate) fn chat_search_empty_placeholder() -> &'static str {
    CHAT_SEARCH_NO_MATCHES_LABEL
}

pub(crate) fn chat_preview_text(last_message: Option<&str>) -> &str {
    last_message
        .filter(|message| !message.trim().is_empty())
        .unwrap_or(CHAT_EMPTY_PREVIEW_LABEL)
}

pub(crate) fn chat_preview_width(text_width: usize) -> usize {
    text_width.saturating_sub(display_width(CHAT_PREVIEW_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::{
        CHAT_EMPTY_NO_CHATS_LABEL, CHAT_EMPTY_NO_FOLDER_LABEL, CHAT_EMPTY_PREVIEW_LABEL,
        CHAT_PREVIEW_PREFIX, CHAT_SEARCH_NO_MATCHES_LABEL, chat_empty_placeholder,
        chat_group_indicator, chat_panel_title, chat_preview_text, chat_preview_width,
        chat_search_empty_placeholder, chat_search_panel_title, chat_unread_indicator,
        display_width,
    };

    #[test]
    fn chat_panel_title_shows_clamped_position() {
        assert_eq!(chat_panel_title(0, 0), " Chats 0/0 ");
        assert_eq!(chat_panel_title(0, 4), " Chats 1/4 ");
        assert_eq!(chat_panel_title(99, 4), " Chats 4/4 ");
    }

    #[test]
    fn chat_search_panel_title_shows_query_and_filtered_position() {
        assert_eq!(chat_search_panel_title("", 0, 0), " Chats /type… 0/0 ");
        assert_eq!(chat_search_panel_title("ali", 0, 2), " Chats /ali 1/2 ");
    }

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

    #[test]
    fn chat_empty_placeholder_distinguishes_missing_folder_from_empty_chat_list() {
        assert_eq!(chat_empty_placeholder(false), CHAT_EMPTY_NO_FOLDER_LABEL);
        assert_eq!(chat_empty_placeholder(true), CHAT_EMPTY_NO_CHATS_LABEL);
    }

    #[test]
    fn chat_search_empty_placeholder_explains_loaded_scope() {
        assert_eq!(
            chat_search_empty_placeholder(),
            CHAT_SEARCH_NO_MATCHES_LABEL
        );
        assert!(chat_search_empty_placeholder().contains("loaded chats"));
    }

    #[test]
    fn chat_preview_text_uses_placeholder_for_missing_or_empty_preview() {
        assert_eq!(chat_preview_text(Some("latest")), "latest");
        assert_eq!(chat_preview_text(Some("   ")), CHAT_EMPTY_PREVIEW_LABEL);
        assert_eq!(chat_preview_text(None), CHAT_EMPTY_PREVIEW_LABEL);
    }

    #[test]
    fn chat_preview_width_reserves_visible_prefix() {
        assert_eq!(CHAT_PREVIEW_PREFIX, "  ");
        assert_eq!(display_width(CHAT_PREVIEW_PREFIX), 2);
        assert_eq!(chat_preview_width(30), 28);
        assert_eq!(chat_preview_width(1), 0);
    }
}
