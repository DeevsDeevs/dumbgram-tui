# Dumbgram TUI

A minimalist Telegram TUI client built with Rust, focusing on essential features without the bloat.

## Current Status: Phase 1 Complete ✅

### What's Working

- ✅ **Modular Architecture**: Clean separation of concerns across modules
- ✅ **Mock Telegram Client**: Fully functional mock client with sample data
- ✅ **Basic TUI Layout**: Split-panel design with folders, chats, messages, and input
- ✅ **Mouse Support**: Fully clickable UI for all interactions
- ✅ **Event Handling**: Keyboard navigation and mode switching
- ✅ **Theme System**: Catppuccin Mocha theme integrated
- ✅ **State Management**: Centralized app state with panel focus tracking
- ✅ **Dynamic Content**: Chat messages update when switching between chats
- ✅ **Folder Filtering**: Chats filter by selected folder tab

### Project Structure

```
src/
├── main.rs           # Entry point with event loop
├── app.rs            # Application state & mode management
├── state.rs          # Global app state
├── telegram/
│   ├── mod.rs
│   ├── client.rs     # Telegram client trait
│   ├── mock.rs       # Mock client implementation
│   └── types.rs      # Data structures (Chat, Folder, Message)
├── config/
│   ├── mod.rs
│   ├── parser.rs     # TOML configuration parsing
│   ├── theme.rs      # Theme system
│   └── defaults.rs   # Default values
└── ui/
    ├── mod.rs
    ├── layout.rs     # Main layout orchestration
    ├── folders.rs    # Folder tabs widget
    ├── chats.rs      # Chat list widget
    ├── messages.rs   # Message view widget
    └── input.rs      # Input field widget
```

## Running the App

```bash
cargo run
```

## Interactions

### Mouse Support
- **Click folder tabs** - Switch between folder views and load chats
- **Click chat items** - Select chat and load messages
- **Click input box** - Focus input and start typing
- **Click anywhere** - Focus that panel

### Arrow Key Navigation (Primary)
- **← →** (Left/Right) - Navigate between folders OR move focus between panels
  - In Folders: Scroll through folder tabs (auto-scrolls if 20+ folders!)
  - In Chats: Move focus left to Folders
  - In Messages: Move focus left to Chats
- **↑ ↓** (Up/Down) - Navigate items in current panel
  - In Chats: Select different chat (loads messages automatically)
  - In Messages: Scroll through messages
  - In Folders: Move down to Chats panel

### Scalable Folder Display
- **Smart scrolling**: Folders auto-scroll to show current selection
- **Visual indicators**: `◀ ▶` arrows show when more folders exist
- **Works with 3 or 300 folders**: Handles any number gracefully

### Input Box Behavior
When the input box is focused (highlighted border):
- **Just start typing** - No need for special modes
- `Enter` - Send message (currently clears input)
- `Backspace` - Delete characters
- `Esc` or `↑` - Exit input box, return to messages panel

### Additional Keyboard Shortcuts
- `q` - Quit application
- `Tab` - Cycle through panels (Folders → Chats → Messages → Input → Folders)
  - *Note: Tab follows a linear left-to-right, top-to-bottom order through all UI panels*
- `</>` - Adjust split panel size

## Next Steps

See [AGENTS.md](AGENTS.md) for the full development plan and remaining phases:
- Phase 2: Real Telegram API integration
- Phase 3: Message sending/editing/replying
- Phase 4: Polish and optimization

## Architecture Highlights

- **Trait-based Client**: Easy to swap mock client for real Telegram API
- **Immutable Widget Pattern**: Following ratatui best practices
- **Mouse-First Design**: Full clickable UI with keyboard alternatives
- **Tab-based Folder Filtering**: Clean UX for folder navigation
- **Area Tracking**: UI components track their screen regions for click detection
- **Async Event Handling**: Non-blocking message loading when switching chats