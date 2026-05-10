use super::{render_chats, render_folders, render_input, render_messages};
use crate::{app::App, config::Theme, state::FocusedPanel, telegram::types::MessageStatus};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub fn render_layout(frame: &mut Frame, app: &mut App, theme: &Theme) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        frame.area(),
    );

    let has_banner = app.state.error_message.is_some() || app.state.status_message.is_some();

    let main_chunks = if has_banner {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Length(3),
            ])
            .split(frame.area())
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(frame.area())
    };

    let horizontal_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((app.state.split_ratio * 100.0) as u16),
            Constraint::Percentage((100.0 - app.state.split_ratio * 100.0) as u16),
        ])
        .split(main_chunks[0]);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(horizontal_chunks[0]);

    app.state.folders_area = left_chunks[0];
    app.state.chats_area = left_chunks[1];
    app.state.messages_area = horizontal_chunks[1];
    app.state.input_area = main_chunks[1];

    render_folders(frame, left_chunks[0], app, theme);
    render_chats(frame, left_chunks[1], app, theme);
    render_messages(frame, horizontal_chunks[1], app, theme);
    render_input(frame, main_chunks[1], app, theme);
    render_help_bar(frame, main_chunks[2], app, theme);

    if let Some(error) = app.state.error_message.as_ref() {
        render_error_banner(frame, main_chunks[3], error, theme);
    } else if let Some(status) = app.state.status_message.as_ref() {
        render_status_banner(frame, main_chunks[3], status, theme);
    }
}

fn render_help_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let focus = match app.state.focused_panel {
        FocusedPanel::Folders => "Folders",
        FocusedPanel::Chats => "Chats",
        FocusedPanel::Messages => "Messages",
        FocusedPanel::Input => "Input",
    };

    let controls = help_bar_controls(app);

    let help = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" Focus: {} ", focus),
            Style::default()
                .fg(theme.border_focused)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(controls),
    ]))
    .style(Style::default().fg(theme.foreground).bg(theme.background));

    frame.render_widget(help, area);
}

fn help_bar_controls(app: &App) -> &'static str {
    if app.state.delete_confirmation.is_some() {
        "Confirm delete: y yes · n/Esc cancel"
    } else if app.state.editing_message_id.is_some() {
        "Editing: Tab focus · Enter save · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
    } else if app.state.replying_to_message_id.is_some() {
        "Replying: Tab focus · Enter send · Ctrl-A/E/B/F/D/U/K/W edit · Esc/Ctrl-C cancel"
    } else if app.state.focused_panel == FocusedPanel::Input {
        "Input: Tab focus · Enter send · Ctrl-A/E/B/F/D/U/K/W edit · Esc cancel"
    } else if app.state.focused_panel == FocusedPanel::Messages
        && app
            .state
            .selected_message()
            .is_some_and(|message| message.status == MessageStatus::Sending)
    {
        "Sending: waiting for Telegram · edit/delete/reply disabled"
    } else if app.state.focused_panel == FocusedPanel::Messages
        && app
            .state
            .selected_message()
            .is_some_and(|message| message.status == MessageStatus::Failed)
    {
        "Failed send: d dismiss · edit restored input then Enter retry"
    } else {
        "q quit · Tab focus · arrows/Pg/Home/End move · e edit · r reply · d delete · </> split"
    }
}

fn render_error_banner(frame: &mut Frame, area: Rect, error: &str, theme: &Theme) {
    let error_widget = Paragraph::new(Span::raw(format!(" ! {}", error)))
        .style(Style::default().fg(theme.error).bg(theme.background))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.error)),
        );

    frame.render_widget(error_widget, area);
}

fn render_status_banner(frame: &mut Frame, area: Rect, status: &str, theme: &Theme) {
    let status_widget = Paragraph::new(Span::raw(format!(" OK {}", status)))
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
    use super::help_bar_controls;
    use crate::app::App;
    use crate::state::{DeleteConfirmation, FocusedPanel};
    use crate::telegram::types::{Message, MessageStatus};
    use chrono::Utc;

    fn message_with_status(status: MessageStatus) -> Message {
        Message {
            id: -1,
            chat_id: 10,
            sender_name: "You".to_string(),
            content: "failed draft".to_string(),
            timestamp: Utc::now(),
            is_own: true,
            is_edited: false,
            reply_to_content: None,
            status,
            can_edit: false,
            can_delete: false,
            error: None,
        }
    }

    fn assert_unicode_separators(label: &str) {
        assert!(
            label.contains('·'),
            "missing Unicode separator in {label:?}"
        );
        assert!(
            !label.contains(" | "),
            "ASCII pipe separator should not render in {label:?}"
        );
    }

    #[test]
    fn help_bar_modes_use_unicode_separators() {
        let mut confirm = App::new();
        confirm.state.delete_confirmation = Some(DeleteConfirmation {
            chat_id: 10,
            message_id: 20,
        });

        let mut editing = App::new();
        editing.state.editing_message_id = Some(20);

        let mut replying = App::new();
        replying.state.replying_to_message_id = Some(20);

        let mut input = App::new();
        input.state.focused_panel = FocusedPanel::Input;

        let mut sending = App::new();
        sending.state.focused_panel = FocusedPanel::Messages;
        sending.state.messages = vec![message_with_status(MessageStatus::Sending)];

        let mut failed = App::new();
        failed.state.focused_panel = FocusedPanel::Messages;
        failed.state.messages = vec![message_with_status(MessageStatus::Failed)];

        let mut normal = App::new();
        normal.state.focused_panel = FocusedPanel::Messages;
        normal.state.messages = vec![message_with_status(MessageStatus::Sent)];

        for app in [
            &confirm, &editing, &replying, &input, &sending, &failed, &normal,
        ] {
            assert_unicode_separators(help_bar_controls(app));
        }
    }

    #[test]
    fn help_bar_explains_in_flight_send_when_sending_row_is_selected() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages = vec![message_with_status(MessageStatus::Sending)];

        assert_eq!(
            help_bar_controls(&app),
            "Sending: waiting for Telegram · edit/delete/reply disabled"
        );
    }

    #[test]
    fn help_bar_explains_failed_send_dismissal_when_failed_row_is_selected() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages = vec![message_with_status(MessageStatus::Failed)];

        assert_eq!(
            help_bar_controls(&app),
            "Failed send: d dismiss · edit restored input then Enter retry"
        );
    }

    #[test]
    fn help_bar_uses_normal_message_controls_for_non_failed_rows() {
        let mut app = App::new();
        app.state.focused_panel = FocusedPanel::Messages;
        app.state.messages = vec![message_with_status(MessageStatus::Sent)];

        assert_eq!(
            help_bar_controls(&app),
            "q quit · Tab focus · arrows/Pg/Home/End move · e edit · r reply · d delete · </> split"
        );
    }
}
