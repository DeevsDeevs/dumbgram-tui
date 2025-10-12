use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::{app::App, config::Theme};
use super::{render_folders, render_chats, render_messages, render_input};

pub fn render_layout(frame: &mut Frame, app: &mut App, theme: &Theme) {
    let error_height = if app.state.error_message.is_some() { 2 } else { 0 };
    
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(error_height),
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
    
    if app.state.error_message.is_some() {
        render_error_banner(frame, main_chunks[2], &app.state.error_message.as_ref().unwrap());
    }
}

fn render_error_banner(frame: &mut Frame, area: Rect, error: &str) {
    let error_widget = Paragraph::new(Span::raw(format!(" ❌ {}", error)))
        .style(Style::default().fg(Color::Red).bg(Color::Black))
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Red)));
    
    frame.render_widget(error_widget, area);
}
