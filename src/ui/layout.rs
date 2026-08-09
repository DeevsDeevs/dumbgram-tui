use super::{render_chats, render_folders, render_input, render_messages, render_thread_topics};
use crate::{
    app::App,
    config::Theme,
    diagnostics,
    state::{AppState, FocusedPanel},
    telegram::types::{Message, MessageStatus},
};
use std::time::Instant;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

pub(crate) const HELP_SEPARATOR: &str = " · ";
pub(crate) const LEGACY_HELP_SEPARATOR: &str = " | ";
pub(crate) const FOCUS_LABEL_PREFIX: &str = "Focus:";
pub(crate) const STATUS_BANNER_PREFIX: &str = "OK";
pub(crate) const ERROR_BANNER_PREFIX: &str = "!";
pub(crate) const FOLDERS_HELP_LABEL: &str = "Folders: Left/Right switch";
pub(crate) const SINGLE_FOLDER_HELP_LABEL: &str = "Folders: no other folders";
pub(crate) const CHATS_HELP_LABEL: &str = "Chats: Up/Down choose";
pub(crate) const SINGLE_CHAT_HELP_LABEL: &str = "Chats: no other chats";
pub(crate) const NO_CHAT_INPUT_HELP_LABEL: &str = "Input: no chat selected";
pub(crate) const EMPTY_INPUT_HELP_LABEL: &str = "Input: type a message";
pub(crate) const EMPTY_EDIT_HELP_LABEL: &str = "Editing: type replacement text";
pub(crate) const EMPTY_REPLY_HELP_LABEL: &str = "Replying: type a reply";
pub(crate) const NO_MESSAGE_HELP_LABEL: &str = "Messages: no message selected";
pub(crate) const MAIN_CONTENT_MIN_HEIGHT: u16 = 1;
pub(crate) const INPUT_PANEL_HEIGHT: u16 = 3;
pub(crate) const HELP_BAR_HEIGHT: u16 = 1;
pub(crate) const HIDE_HELP_CONTROL_LABEL: &str = "? hide help";
pub(crate) const BANNER_HEIGHT: u16 = 3;
pub(crate) const FOLDERS_PANEL_HEIGHT: u16 = 3;
pub(crate) const THREAD_TOPICS_PANEL_HEIGHT: u16 = 3;
pub(crate) const IMAGE_VIEWPORT_TITLE: &str = "Image preview";
pub(crate) const IMAGE_VIEWPORT_MIN_WIDTH: u16 = 24;
pub(crate) const IMAGE_VIEWPORT_MAX_WIDTH: u16 = 48;
const SLOW_RENDER_LOG_THRESHOLD_MS: u128 = 100;

pub fn render_layout(frame: &mut Frame, app: &mut App, theme: &Theme) {
    let render_started = diagnostics::enabled().then(Instant::now);
    app.state.screen_area = frame.area();

    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        frame.area(),
    );

    let has_banner = app.state.error_message.is_some() || app.state.status_message.is_some();

    let show_help_bar = app.state.show_help_bar;
    let mut vertical_constraints = vec![
        Constraint::Min(MAIN_CONTENT_MIN_HEIGHT),
        Constraint::Length(INPUT_PANEL_HEIGHT),
    ];
    if show_help_bar {
        vertical_constraints.push(Constraint::Length(HELP_BAR_HEIGHT));
    }
    if has_banner {
        vertical_constraints.push(Constraint::Length(BANNER_HEIGHT));
    }

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vertical_constraints)
        .split(frame.area());

    let horizontal_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((app.state.split_ratio * 100.0) as u16),
            Constraint::Percentage((100.0 - app.state.split_ratio * 100.0) as u16),
        ])
        .split(main_chunks[0]);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(FOLDERS_PANEL_HEIGHT),
            Constraint::Min(MAIN_CONTENT_MIN_HEIGHT),
        ])
        .split(horizontal_chunks[0]);

    let right_chunks = if app.state.thread_topics.is_empty() {
        None
    } else {
        Some(
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(THREAD_TOPICS_PANEL_HEIGHT),
                    Constraint::Min(MAIN_CONTENT_MIN_HEIGHT),
                ])
                .split(horizontal_chunks[1]),
        )
    };
    let message_viewport_area = right_chunks
        .as_ref()
        .map_or(horizontal_chunks[1], |chunks| chunks[1]);
    let (messages_area, image_area) = message_and_image_areas(message_viewport_area, app);

    app.state.folders_area = left_chunks[0];
    app.state.chats_area = left_chunks[1];
    app.state.messages_area = messages_area;
    app.state.thread_topics_area = right_chunks
        .as_ref()
        .map_or(Rect::default(), |chunks| chunks[0]);
    app.state.terminal_image_area = image_area;
    app.state.input_area = main_chunks[1];

    render_folders(frame, left_chunks[0], app, theme);
    render_chats(frame, left_chunks[1], app, theme);
    if let Some(chunks) = right_chunks.as_ref() {
        render_thread_topics(frame, chunks[0], app, theme);
    }
    render_messages(frame, messages_area, app, theme);
    render_image_viewport(frame, image_area, theme);
    render_input(frame, main_chunks[1], app, theme);
    if show_help_bar {
        render_help_bar(frame, main_chunks[2], app, theme);
    }

    let banner_index = 2 + usize::from(show_help_bar);
    if let Some(error) = app.state.error_message.as_ref() {
        render_error_banner(frame, main_chunks[banner_index], error, theme);
    } else if let Some(status) = app.state.status_message.as_ref() {
        render_status_banner(frame, main_chunks[banner_index], status, theme);
    }

    render_context_menu(frame, app, theme);

    if let Some(started) = render_started {
        let elapsed_ms = started.elapsed().as_millis();
        if elapsed_ms >= SLOW_RENDER_LOG_THRESHOLD_MS {
            diagnostics::event(
                "slow_render",
                format!(
                    "elapsed_ms={elapsed_ms} folders={} chats={} messages={} focused={} help_bar={} terminal={}x{}",
                    app.state.folders.len(),
                    app.state.chats.len(),
                    app.state.messages.len(),
                    app.state.focused_panel.label(),
                    app.state.show_help_bar,
                    frame.area().width,
                    frame.area().height
                ),
            );
        }
    }
}

fn render_context_menu(frame: &mut Frame, app: &App, theme: &Theme) {
    let (Some(menu), Some(area)) = (app.state.context_menu(), app.state.context_menu_rect()) else {
        return;
    };
    let items = menu
        .actions
        .iter()
        .map(|action| ListItem::new(action.label()))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Actions "))
        .style(Style::default().fg(theme.foreground).bg(theme.background))
        .highlight_style(
            Style::default()
                .fg(theme.selection_foreground)
                .bg(theme.selection)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default().with_selected(Some(menu.highlighted));
    frame.render_widget(Clear, area);
    frame.render_stateful_widget(list, area, &mut state);
}

fn message_and_image_areas(area: Rect, app: &App) -> (Rect, Rect) {
    let has_selected_local_image = app
        .state
        .selected_message()
        .and_then(|message| message.media.as_ref())
        .and_then(|media| media.local_image_path())
        .is_some();
    if !has_selected_local_image || area.width < IMAGE_VIEWPORT_MIN_WIDTH * 2 {
        return (area, Rect::default());
    }

    let image_width = (area.width / 3).clamp(IMAGE_VIEWPORT_MIN_WIDTH, IMAGE_VIEWPORT_MAX_WIDTH);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(image_width)])
        .split(area);
    (chunks[0], chunks[1])
}

fn render_image_viewport(frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(IMAGE_VIEWPORT_TITLE)
        .style(Style::default().bg(theme.background));
    frame.render_widget(block, area);
}

fn render_help_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let focus = app.state.focused_panel.label();

    let controls = help_bar_controls(app);

    let help = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} {} ", FOCUS_LABEL_PREFIX, focus),
            Style::default()
                .fg(theme.border_focused)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(controls),
    ]))
    .style(Style::default().fg(theme.foreground).bg(theme.background));

    frame.render_widget(help, area);
}

fn help_bar_controls(app: &App) -> String {
    if app.state.delete_confirmation().is_some() {
        join_help_controls(&["Confirm delete: y yes", "n/Esc/Ctrl-C cancel"])
    } else if app.state.context_menu().is_some() {
        join_help_controls(&["Menu: Up/Down choose", "Enter select", "Esc close"])
    } else if app.state.editing_message_id.is_some() {
        if app.state.focused_panel != FocusedPanel::Input {
            let edit_label = if app.state.input_has_submit_text() {
                "Editing: focus Input to save"
            } else {
                "Editing: focus Input to type"
            };
            join_help_controls(&[
                edit_label,
                "Tab focus",
                "Esc/Ctrl-C cancel",
                HIDE_HELP_CONTROL_LABEL,
            ])
        } else if !app.state.input_has_submit_text() {
            join_help_controls(&[
                EMPTY_EDIT_HELP_LABEL,
                "Ctrl-A/E/B/F/D/U/K/W edit",
                "Esc/Ctrl-C cancel",
            ])
        } else {
            join_help_controls(&[
                "Editing: Tab focus",
                "Enter save",
                "Ctrl-A/E/B/F/D/U/K/W edit",
                "Esc/Ctrl-C cancel",
            ])
        }
    } else if app.state.replying_to_message_id.is_some() {
        if app.state.focused_panel != FocusedPanel::Input {
            let reply_label = if app.state.input_has_submit_text() {
                "Replying: focus Input to send"
            } else {
                "Replying: focus Input to type"
            };
            join_help_controls(&[
                reply_label,
                "Tab focus",
                "Esc/Ctrl-C cancel",
                HIDE_HELP_CONTROL_LABEL,
            ])
        } else if !app.state.input_has_submit_text() {
            join_help_controls(&[
                EMPTY_REPLY_HELP_LABEL,
                "Ctrl-A/E/B/F/D/U/K/W edit",
                "Esc/Ctrl-C cancel",
            ])
        } else {
            join_help_controls(&[
                "Replying: Tab focus",
                "Enter send",
                "Ctrl-A/E/B/F/D/U/K/W edit",
                "Esc/Ctrl-C cancel",
            ])
        }
    } else if app.state.focused_panel == FocusedPanel::Input {
        if app.state.selected_chat_id().is_none() {
            join_help_controls(&[
                NO_CHAT_INPUT_HELP_LABEL,
                "Tab focus",
                "Ctrl-A/E/B/F/D/U/K/W edit",
                "Esc/Ctrl-C cancel",
                "choose a chat before sending",
            ])
        } else if !app.state.input_has_submit_text() {
            join_help_controls(&[
                EMPTY_INPUT_HELP_LABEL,
                "Ctrl-A/E/B/F/D/U/K/W edit",
                "Esc/Ctrl-C cancel",
            ])
        } else {
            join_help_controls(&[
                "Input: Tab focus",
                "Enter send",
                "Ctrl-A/E/B/F/D/U/K/W edit",
                "Esc/Ctrl-C cancel",
            ])
        }
    } else if app.state.focused_panel == FocusedPanel::Folders {
        let folder_label = if app.state.folders.len() > 1 {
            FOLDERS_HELP_LABEL
        } else {
            SINGLE_FOLDER_HELP_LABEL
        };
        join_help_controls(&[
            folder_label,
            "Down chats",
            "Tab focus",
            "q quit",
            "< > resize",
            HIDE_HELP_CONTROL_LABEL,
        ])
    } else if app.state.focused_panel == FocusedPanel::Chats {
        if app.state.chat_search_active() {
            return join_help_controls(&[
                "Search chats: type",
                "Enter open",
                "Esc clear",
                "Backspace edit",
                "Up/Down browse",
                HIDE_HELP_CONTROL_LABEL,
            ]);
        }
        let chats_label = if app.state.chats.len() > 1 {
            CHATS_HELP_LABEL
        } else {
            SINGLE_CHAT_HELP_LABEL
        };
        let mut controls = vec![chats_label];
        if app.state.chats.len() > 1 {
            controls.push("letters jump");
        }
        controls.extend([
            "/ search",
            "Right messages",
            "Left folders",
            "Tab focus",
            "q quit",
            "< > resize",
            HIDE_HELP_CONTROL_LABEL,
        ]);
        join_help_controls(&controls)
    } else if app.state.focused_panel == FocusedPanel::Messages
        && app.state.selected_message().is_none()
    {
        join_help_controls(&[
            NO_MESSAGE_HELP_LABEL,
            "Enter input",
            "Left chats",
            "Tab focus",
            "q quit",
            "< > resize",
            HIDE_HELP_CONTROL_LABEL,
        ])
    } else if app.state.focused_panel == FocusedPanel::Messages
        && app
            .state
            .selected_message()
            .is_some_and(|message| message.status == MessageStatus::Sending)
    {
        let mut controls = vec!["Sending: waiting for Telegram"];
        if app.state.newer_history_gap() {
            controls.push("End refresh latest");
        }
        controls.extend([
            "edit/delete/reply disabled",
            "Enter input",
            "Left chats",
            "Tab focus",
            "q quit",
            "< > resize",
            HIDE_HELP_CONTROL_LABEL,
        ]);
        join_help_controls(&controls)
    } else if app.state.focused_panel == FocusedPanel::Messages
        && app
            .state
            .selected_message()
            .is_some_and(|message| message.status == MessageStatus::Failed)
    {
        let mut controls = vec!["Failed send: d dismiss"];
        if app.state.newer_history_gap() {
            controls.push("End refresh latest");
        }
        controls.extend([
            "Enter input to retry",
            "Left chats",
            "Tab focus",
            "q quit",
            "< > resize",
            HIDE_HELP_CONTROL_LABEL,
        ]);
        join_help_controls(&controls)
    } else if app.state.focused_panel == FocusedPanel::Messages {
        app.state
            .selected_message()
            .map(|message| selected_message_help_controls(&app.state, message))
            .unwrap_or_else(|| {
                join_help_controls(&[
                    NO_MESSAGE_HELP_LABEL,
                    "Left chats",
                    "Tab focus",
                    HIDE_HELP_CONTROL_LABEL,
                ])
            })
    } else {
        join_help_controls(&["q quit", "Tab focus", "< > resize", HIDE_HELP_CONTROL_LABEL])
    }
}

fn selected_message_help_controls(state: &AppState, message: &Message) -> String {
    let has_messages = !state.messages.is_empty();
    let at_loaded_top = state.selected_message_index == 0 && has_messages;
    let at_loaded_bottom = state.selected_message_is_last() && has_messages;
    let movement_label = if state.newer_history_gap() && at_loaded_bottom {
        "Messages: End/Down/PgDn refresh latest · Up/Pg/Home move"
    } else if state.newer_history_gap() {
        "Messages: End refresh latest · Up/Down/Pg/Home move"
    } else if at_loaded_top && at_loaded_bottom && state.selected_chat_older_history_exhausted() {
        "Messages: no older history · Pg/Home/End move"
    } else if at_loaded_top && state.selected_chat_older_history_exhausted() {
        "Messages: no older history · Down/PgDn/Home/End move"
    } else if at_loaded_top && at_loaded_bottom {
        "Messages: Up/PgUp older · Pg/Home/End move"
    } else if at_loaded_top {
        "Messages: Up/PgUp older · Down/PgDn/Home/End move"
    } else if at_loaded_bottom {
        "Messages: Up/Pg/Home/End move"
    } else {
        "Messages: Up/Down/Pg/Home/End move"
    };
    let mut controls = vec![movement_label, "Enter input"];

    if !state.thread_topics.is_empty() {
        controls.push("Left/Right topics");
    }

    if message.is_own && message.can_edit {
        controls.push("e edit");
    }

    controls.push("r reply");

    if !message.content.trim().is_empty() {
        controls.push("c copy text");
    }

    if crate::links::first_url(&message.content).is_some() {
        controls.push("o open link");
    }

    if message
        .media
        .as_ref()
        .is_some_and(|media| media.kind.is_downloadable())
    {
        controls.push("s save media");
    }

    if state.selected_message_download_path().is_some() {
        controls.push("v open saved");
    }

    if message.is_own && message.can_delete {
        controls.push("d delete");
    }

    controls.extend([
        "Left chats",
        "Tab focus",
        "q quit",
        "< > resize",
        HIDE_HELP_CONTROL_LABEL,
    ]);
    join_help_controls(&controls)
}

fn join_help_controls(parts: &[&str]) -> String {
    parts.join(HELP_SEPARATOR)
}

fn render_error_banner(frame: &mut Frame, area: Rect, error: &str, theme: &Theme) {
    let error_widget = Paragraph::new(Span::raw(format!(" {} {}", ERROR_BANNER_PREFIX, error)))
        .style(Style::default().fg(theme.error).bg(theme.background))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.error)),
        );

    frame.render_widget(error_widget, area);
}

fn render_status_banner(frame: &mut Frame, area: Rect, status: &str, theme: &Theme) {
    let status_widget = Paragraph::new(Span::raw(format!(" {} {}", STATUS_BANNER_PREFIX, status)))
        .style(
            Style::default()
                .fg(theme.border_focused)
                .bg(theme.background),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border_focused)),
        );

    frame.render_widget(status_widget, area);
}

#[cfg(test)]
mod tests {
    use super::{
        BANNER_HEIGHT, ERROR_BANNER_PREFIX, FOCUS_LABEL_PREFIX, FOLDERS_PANEL_HEIGHT,
        HELP_BAR_HEIGHT, HELP_SEPARATOR, IMAGE_VIEWPORT_MIN_WIDTH, IMAGE_VIEWPORT_TITLE,
        INPUT_PANEL_HEIGHT, LEGACY_HELP_SEPARATOR, MAIN_CONTENT_MIN_HEIGHT, STATUS_BANNER_PREFIX,
        THREAD_TOPICS_PANEL_HEIGHT, help_bar_controls,
    };
    use crate::app::App;
    use crate::state::{DeleteConfirmation, FocusedPanel};
    use crate::telegram::types::{
        Chat, Folder, Message, MessageMedia, MessageStatus, OWN_SENDER_NAME, Update, all_folder,
    };
    use crate::ui::render_app_to_string_for_test;
    use chrono::Utc;
    use ratatui::layout::Rect;

    fn folder(id: i32, name: &str) -> Folder {
        Folder {
            id,
            name: name.to_string(),
            unread_count: 0,
        }
    }

    fn chat(id: i64, name: &str) -> Chat {
        Chat {
            id,
            name: name.to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }
    }

    fn message_with_status(status: MessageStatus) -> Message {
        Message {
            id: -1,
            chat_id: 10,
            thread_topic_id: None,
            sender_name: OWN_SENDER_NAME.to_string(),
            content: "failed draft".to_string(),
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_content: None,
            media: None,
            status,
            can_edit: false,
            can_delete: false,
            error: None,
        }
    }

    fn assert_unicode_separators(label: &str) {
        assert!(
            label.contains(HELP_SEPARATOR),
            "missing Unicode separator in {label:?}"
        );
        assert!(
            !label.contains(LEGACY_HELP_SEPARATOR),
            "ASCII pipe separator should not render in {label:?}"
        );
    }

    #[test]
    fn fixed_layout_heights_are_explicit() {
        assert_eq!(MAIN_CONTENT_MIN_HEIGHT, 1);
        assert_eq!(INPUT_PANEL_HEIGHT, 3);
        assert_eq!(HELP_BAR_HEIGHT, 1);
        assert_eq!(BANNER_HEIGHT, 3);
        assert_eq!(FOLDERS_PANEL_HEIGHT, 3);
        assert_eq!(THREAD_TOPICS_PANEL_HEIGHT, 3);
        assert_eq!(IMAGE_VIEWPORT_MIN_WIDTH, 24);
    }

    #[test]
    fn image_viewport_splits_messages_to_right_only_for_selected_local_image() {
        let full_area = Rect::new(0, 0, 120, 30);
        let mut app = App::new();

        let (messages_area, image_area) = super::message_and_image_areas(full_area, &app);
        assert_eq!(messages_area, full_area);
        assert!(image_area.is_empty());

        app.state.messages = vec![message_with_status(MessageStatus::Delivered)];
        app.state.messages[0].media = Some(MessageMedia::photo().with_local_path("/tmp/photo.jpg"));
        let (messages_area, image_area) = super::message_and_image_areas(full_area, &app);

        assert!(messages_area.width < full_area.width);
        assert!(!image_area.is_empty());
        assert_eq!(image_area.x, messages_area.x + messages_area.width);
    }

    #[test]
    fn layout_renders_image_viewport_without_covering_message_list() {
        let mut app = App::new();
        app.state.chats = vec![chat(10, "Media")];
        app.state.messages = vec![message_with_status(MessageStatus::Delivered)];
        app.state.messages[0].media = Some(MessageMedia::photo().with_local_path("/tmp/photo.jpg"));

        let rendered = render_app_to_string_for_test(&mut app);

        assert!(rendered.contains(IMAGE_VIEWPORT_TITLE));
        assert!(!app.state.terminal_image_area.is_empty());
        assert!(app.state.messages_area.width < app.state.terminal_image_area.x);
    }

    #[test]
    fn layout_renders_error_banner_before_status_banner() {
        let mut app = App::new();
        app.state.error_message = Some("network down".to_string());
        app.state.status_message = Some("saved".to_string());

        let rendered = render_app_to_string_for_test(&mut app);

        assert!(
            rendered.contains(&format!("{ERROR_BANNER_PREFIX} network down")),
            "missing error banner"
        );
        assert!(
            !rendered.contains(&format!("{STATUS_BANNER_PREFIX} saved")),
            "status banner should not render while an error is active"
        );
    }

    #[test]
    fn layout_can_hide_help_bar_without_hiding_status_banner() {
        let mut app = App::new();
        app.state.show_help_bar = false;
        app.state.status_message = Some("ready".to_string());

        let rendered = render_app_to_string_for_test(&mut app);

        assert!(
            !rendered.contains(FOCUS_LABEL_PREFIX),
            "hidden help bar should omit focus controls"
        );
        assert!(
            rendered.contains(&format!("{STATUS_BANNER_PREFIX} ready")),
            "hidden help bar should not hide the status banner"
        );
    }

    #[test]
    fn help_bar_modes_use_unicode_separators() {
        let mut confirm = App::new();
        confirm.state.set_delete_confirmation(DeleteConfirmation {
            chat_id: 10,
            message_id: 20,
        });

        let mut editing = App::new();
        editing.state.editing_message_id = Some(20);
        editing.state.input_buffer = "replacement".to_string();

        let mut replying = App::new();
        replying.state.replying_to_message_id = Some(20);
        replying.state.input_buffer = "reply".to_string();

        let mut input = App::new();
        input.state.focused_panel = FocusedPanel::Input;
        input.state.chats = vec![crate::telegram::types::Chat {
            id: 10,
            name: "General".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];

        let mut folders = App::new();
        folders.state.focused_panel = FocusedPanel::Folders;

        let mut chats = App::new();
        chats.state.focused_panel = FocusedPanel::Chats;

        let mut empty_messages = App::new();
        empty_messages.state.focused_panel = FocusedPanel::Messages;

        let mut sending = App::new();
        sending.state.focused_panel = FocusedPanel::Messages;
        sending.state.messages = vec![message_with_status(MessageStatus::Sending)];

        let mut failed = App::new();
        failed.state.focused_panel = FocusedPanel::Messages;
        failed.state.messages = vec![message_with_status(MessageStatus::Failed)];

        let mut normal = App::new();
        normal.state.focused_panel = FocusedPanel::Messages;
        normal.state.messages = vec![message_with_status(MessageStatus::Sent)];
        normal.state.messages[0].can_edit = true;
        normal.state.messages[0].can_delete = true;

        for app in [
            &confirm,
            &editing,
            &replying,
            &input,
            &folders,
            &chats,
            &empty_messages,
            &sending,
            &failed,
            &normal,
        ] {
            assert_unicode_separators(&help_bar_controls(app));
        }
    }

    #[test]
    fn help_bar_omits_submit_when_compose_input_is_empty() {
        let mut editing = App::new();
        editing.state.focused_panel = FocusedPanel::Input;
        editing.state.editing_message_id = Some(20);
        assert_eq!(
            help_bar_controls(&editing),
            "Editing: type replacement text · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );

        editing.state.input_buffer = "   ".to_string();
        assert_eq!(
            help_bar_controls(&editing),
            "Editing: type replacement text · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );

        editing.state.input_buffer = "replacement".to_string();
        assert_eq!(
            help_bar_controls(&editing),
            "Editing: Tab focus · Enter save · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );

        let mut replying = App::new();
        replying.state.focused_panel = FocusedPanel::Input;
        replying.state.replying_to_message_id = Some(20);
        assert_eq!(
            help_bar_controls(&replying),
            "Replying: type a reply · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );

        replying.state.input_buffer = "   ".to_string();
        assert_eq!(
            help_bar_controls(&replying),
            "Replying: type a reply · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );

        replying.state.input_buffer = "reply".to_string();
        assert_eq!(
            help_bar_controls(&replying),
            "Replying: Tab focus · Enter send · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );
    }

    #[test]
    fn help_bar_uses_focus_aware_compose_controls_outside_input() {
        let mut editing = App::new();
        editing.state.focused_panel = FocusedPanel::Messages;
        editing.state.editing_message_id = Some(20);
        assert_eq!(
            help_bar_controls(&editing),
            "Editing: focus Input to type · Tab focus · Esc/Ctrl-C cancel · ? hide help"
        );

        editing.state.input_buffer = "replacement".to_string();
        assert_eq!(
            help_bar_controls(&editing),
            "Editing: focus Input to save · Tab focus · Esc/Ctrl-C cancel · ? hide help"
        );

        let mut replying = App::new();
        replying.state.focused_panel = FocusedPanel::Messages;
        replying.state.replying_to_message_id = Some(20);
        assert_eq!(
            help_bar_controls(&replying),
            "Replying: focus Input to type · Tab focus · Esc/Ctrl-C cancel · ? hide help"
        );

        replying.state.input_buffer = "reply".to_string();
        assert_eq!(
            help_bar_controls(&replying),
            "Replying: focus Input to send · Tab focus · Esc/Ctrl-C cancel · ? hide help"
        );
    }

    #[test]
    fn help_bar_omits_send_when_input_is_empty() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Input;
        app.state.chats = vec![crate::telegram::types::Chat {
            id: 10,
            name: "General".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];

        assert_eq!(
            help_bar_controls(&app),
            "Input: type a message · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );

        app.state.input_buffer = "   ".to_string();
        assert_eq!(
            help_bar_controls(&app),
            "Input: type a message · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );

        app.state.input_buffer = "draft".to_string();
        assert_eq!(
            help_bar_controls(&app),
            "Input: Tab focus · Enter send · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
        );
    }

    #[test]
    fn help_bar_omits_send_when_input_has_no_selected_chat() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Input;

        assert_eq!(
            help_bar_controls(&app),
            "Input: no chat selected · Tab focus · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel · choose a chat before sending"
        );

        app.state.input_buffer = "draft".to_string();
        assert_eq!(
            help_bar_controls(&app),
            "Input: no chat selected · Tab focus · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel · choose a chat before sending"
        );
    }

    #[test]
    fn help_bar_uses_panel_specific_controls_for_folders_and_chats() {
        let mut folders = App::new();
        folders.state.focused_panel = FocusedPanel::Folders;
        assert_eq!(
            help_bar_controls(&folders),
            "Folders: no other folders · Down chats · Tab focus · q quit · < > resize · ? hide help"
        );

        folders.state.folders = vec![all_folder(0), folder(2, "Work")];
        assert_eq!(
            help_bar_controls(&folders),
            "Folders: Left/Right switch · Down chats · Tab focus · q quit · < > resize · ? hide help"
        );

        let mut chats = App::new();
        chats.state.focused_panel = FocusedPanel::Chats;
        assert_eq!(
            help_bar_controls(&chats),
            "Chats: no other chats · / search · Right messages · Left folders · Tab focus · q quit · < > resize · ? hide help"
        );

        chats.state.chats = vec![crate::telegram::types::Chat {
            id: 10,
            name: "General".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        }];
        assert_eq!(
            help_bar_controls(&chats),
            "Chats: no other chats · / search · Right messages · Left folders · Tab focus · q quit · < > resize · ? hide help"
        );

        chats.state.chats.push(crate::telegram::types::Chat {
            id: 11,
            name: "Random".to_string(),
            last_message: None,
            unread_count: 0,
            is_group: false,
            folder_id: None,
        });
        assert_eq!(
            help_bar_controls(&chats),
            "Chats: Up/Down choose · letters jump · / search · Right messages · Left folders · Tab focus · q quit · < > resize · ? hide help"
        );

        chats.state.begin_chat_search();
        assert_eq!(
            help_bar_controls(&chats),
            "Search chats: type · Enter open · Esc clear · Backspace edit · Up/Down browse · ? hide help"
        );
    }

    #[test]
    fn help_bar_omits_message_actions_when_no_message_is_selected() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Messages;

        assert_eq!(
            help_bar_controls(&app),
            "Messages: no message selected · Enter input · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );
    }

    #[test]
    fn help_bar_explains_in_flight_send_when_sending_row_is_selected() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages = vec![message_with_status(MessageStatus::Sending)];

        assert_eq!(
            help_bar_controls(&app),
            "Sending: waiting for Telegram · edit/delete/reply disabled · Enter input · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );
    }

    #[test]
    fn help_bar_explains_failed_send_dismissal_when_failed_row_is_selected() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages = vec![message_with_status(MessageStatus::Failed)];

        assert_eq!(
            help_bar_controls(&app),
            "Failed send: d dismiss · Enter input to retry · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );
    }

    #[test]
    fn local_send_help_keeps_newer_gap_refresh_discoverable() {
        let mut app = App::new();
        app.state.chats = vec![chat(10, "General")];
        app.state.messages = (1..=500)
            .map(|id| {
                let mut message = message_with_status(MessageStatus::Sent);
                message.id = id;
                message
            })
            .collect();
        app.state.selected_message_index = 0;
        let mut omitted = message_with_status(MessageStatus::Sent);
        omitted.id = 501;
        app.state.apply_update(Update::NewMessage(omitted));
        app.state
            .apply_send_pending(-2, 10, None, "pending".to_string());
        app.state.focused_panel = FocusedPanel::Messages;

        assert!(help_bar_controls(&app).contains("End refresh latest"));

        app.state.apply_send_failure(-2, "offline".to_string());
        app.state.focused_panel = FocusedPanel::Messages;
        assert!(help_bar_controls(&app).contains("End refresh latest"));
    }

    #[test]
    fn help_bar_uses_capability_aware_controls_for_selected_messages() {
        let mut reply_only = App::new();
        reply_only.state.focused_panel = FocusedPanel::Messages;
        reply_only.state.messages = vec![message_with_status(MessageStatus::Sent)];

        assert_eq!(
            help_bar_controls(&reply_only),
            "Messages: Up/PgUp older · Pg/Home/End move · Enter input · r reply · c copy text · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );

        reply_only
            .state
            .messages
            .push(message_with_status(MessageStatus::Sent));
        reply_only.state.selected_message_index = 1;
        assert_eq!(
            help_bar_controls(&reply_only),
            "Messages: Up/Pg/Home/End move · Enter input · r reply · c copy text · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );

        reply_only
            .state
            .messages
            .push(message_with_status(MessageStatus::Sent));
        reply_only.state.selected_message_index = 1;
        assert_eq!(
            help_bar_controls(&reply_only),
            "Messages: Up/Down/Pg/Home/End move · Enter input · r reply · c copy text · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );

        let mut full_actions = App::new();
        full_actions.state.focused_panel = FocusedPanel::Messages;
        full_actions.state.messages = vec![message_with_status(MessageStatus::Sent)];
        full_actions.state.messages[0].can_edit = true;
        full_actions.state.messages[0].can_delete = true;

        assert_eq!(
            help_bar_controls(&full_actions),
            "Messages: Up/PgUp older · Pg/Home/End move · Enter input · e edit · r reply · c copy text · d delete · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );

        let mut link_actions = App::new();
        link_actions.state.focused_panel = FocusedPanel::Messages;
        link_actions.state.messages = vec![message_with_status(MessageStatus::Sent)];
        link_actions.state.messages[0].content = "open https://example.org".to_string();

        assert_eq!(
            help_bar_controls(&link_actions),
            "Messages: Up/PgUp older · Pg/Home/End move · Enter input · r reply · c copy text · o open link · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );

        let mut media_actions = App::new();
        media_actions.state.focused_panel = FocusedPanel::Messages;
        media_actions.state.messages = vec![message_with_status(MessageStatus::Sent)];
        media_actions.state.messages[0].media = Some(MessageMedia::photo());

        assert_eq!(
            help_bar_controls(&media_actions),
            "Messages: Up/PgUp older · Pg/Home/End move · Enter input · r reply · c copy text · s save media · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );

        media_actions
            .state
            .record_downloaded_media(10, -1, "/tmp/downloaded-photo.jpg".into());
        assert_eq!(
            help_bar_controls(&media_actions),
            "Messages: Up/PgUp older · Pg/Home/End move · Enter input · r reply · c copy text · s save media · v open saved · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );
    }

    #[test]
    fn help_bar_stops_advertising_older_history_when_exhausted() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.chats = vec![chat(10, "General")];
        app.state.messages = vec![message_with_status(MessageStatus::Sent)];
        app.state.mark_selected_chat_older_history_exhausted();

        assert_eq!(
            help_bar_controls(&app),
            "Messages: no older history · Pg/Home/End move · Enter input · r reply · c copy text · Left chats · Tab focus · q quit · < > resize · ? hide help"
        );
    }
}
