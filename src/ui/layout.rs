use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};
use crate::{app::App, config::Theme};
use super::{render_folders, render_chats, render_messages, render_input};

pub fn render_layout(frame: &mut Frame, app: &mut App, theme: &Theme) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(3),
        ])
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
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(horizontal_chunks[0]);

    app.state.folders_area = left_chunks[0];
    app.state.chats_area = left_chunks[1];
    app.state.messages_area = horizontal_chunks[1];
    app.state.input_area = main_chunks[1];

    render_folders(frame, left_chunks[0], app, theme);
    render_chats(frame, left_chunks[1], app, theme);
    render_messages(frame, horizontal_chunks[1], app, theme);
    render_input(frame, main_chunks[1], app, theme);
}
