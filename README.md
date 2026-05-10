# Dumbgram TUI

A minimalist Telegram TUI client built with Rust, focusing on essential features without the bloat.

## 🎉 Current Status: Phase 3 Complete - REAL TELEGRAM INTEGRATION! 🎉

### What's Working

**Core Features:**
- ✅ **Real Telegram API**: Fully integrated with grammers-client v0.7.0
- ✅ **Authentication**: CLI-based login (phone → code → 2FA)
- ✅ **Session Persistence**: Login once, reuse forever
- ✅ **Send Messages**: Type and send to real Telegram chats
- ✅ **Edit Messages**: Press `e` to edit your messages
- ✅ **Delete Messages**: Press `d` then `y` to delete
- ✅ **Reply to Messages**: Press `r` to reply
- ✅ **Real-Time Updates**: Receive messages from other users instantly
- ✅ **Chat Cache**: Smart caching for optimal performance

**UI Features:**
- ✅ **Modular Architecture**: Clean separation of concerns across modules
- ✅ **Split-Panel Design**: Folders, chats, messages, and input
- ✅ **Mouse Support**: Fully clickable UI for all interactions
- ✅ **Keyboard Navigation**: Full arrow key and vim-like support
- ✅ **Theme System**: Catppuccin Mocha color scheme
- ✅ **Dynamic Content**: Real chat messages from Telegram
- ✅ **Folder Filtering**: Single "All" folder (MVP approach)

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

## Setup

### 1. Get Telegram API Credentials

1. Go to https://my.telegram.org
2. Log in with your phone number
3. Go to "API development tools"
4. Create a new application
5. Copy your `api_id` and `api_hash`

### 2. Configure the App

```bash
# Copy the example config
cp config.example.toml config.toml

# Edit config.toml and add your credentials
[telegram]
api_id = YOUR_API_ID
api_hash = "YOUR_API_HASH"
session_file = "session.dat"
```

### 3. Run the App

```bash
cargo run
```

**First run:** You'll be prompted for:
- Phone number (with country code, e.g., +1234567890)
- Verification code (sent via SMS or Telegram app)
- 2FA password (if enabled)

**Subsequent runs:** Auto-login using saved session!

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

### Message Operations
- **Select a message** - Use arrow keys to navigate
- **`e`** - Edit selected message (your messages only)
- **`r`** - Reply to selected message
- **`d`** - Delete selected message (confirmation required)
- **`y`** - Confirm deletion
- **`n` or `Esc`** - Cancel deletion

### Input Box Behavior
When the input box is focused (highlighted border):
- **Just start typing** - No need for special modes
- `Enter` - Send message to current chat (sends to real Telegram!)
- `Backspace` - Delete characters
- `Esc` or `↑` - Exit input box, return to messages panel

### Additional Keyboard Shortcuts
- `q` - Quit application
- `Tab` - Cycle through panels (Folders → Chats → Messages → Input → Folders)
- `</>` - Adjust split panel size

## Development Status

**Completed Phases:**
- ✅ **Phase 1**: Foundation (TUI architecture, mock data)
- ✅ **Phase 2**: Message Operations (send, edit, reply, delete with optimistic UI)
- ✅ **Phase 3**: Real Telegram Integration (grammers-client, authentication, real-time updates)

**Future Enhancements:**
- Custom folder support (beyond "All")
- Unread message counts
- Media preview/download
- Search functionality
- Multiple account support

See [PHASE3_PLAN.md](PHASE3_PLAN.md) for detailed implementation notes.

## Architecture Highlights

- **Trait-based Client**: Clean abstraction over grammers-client
- **Chat Caching**: Smart HashMap-based cache for grammers API requirements
- **Real-Time Updates**: Background tokio task with mpsc channel
- **Optimistic UI**: Immediate feedback for message operations
- **Session Persistence**: Login once, auto-reconnect on subsequent runs
- **Immutable Widget Pattern**: Following ratatui best practices
- **Mouse-First Design**: Full clickable UI with keyboard alternatives
- **Area Tracking**: UI components track their screen regions for click detection
- **Non-blocking Event Loop**: Uses `event::poll()` for responsive updates

## Known Limitations

- **Unread Counts**: Not available (grammers Dialog API limitation)
- **Custom Folders**: Only "All" folder supported (MVP approach)
- **Delete Message Chat ID**: Always 0 due to grammers MessageDeletion limitation

These are API limitations, not bugs. The app works perfectly within these constraints!