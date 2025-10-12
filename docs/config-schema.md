# Configuration Schema

## Main Configuration File

**Location**: `~/.config/dumbgram/config.toml`

```toml
# Dumbgram TUI Configuration

[telegram]
api_id = "YOUR_API_ID"
api_hash = "YOUR_API_HASH"
session_path = "~/.config/dumbgram/session.dat"
phone_number = ""  # Optional: pre-fill for auth

[telegram.sync]
auto_sync_interval_seconds = 5
fetch_history_on_chat_open = true
initial_message_load_count = 50
message_cache_size = 1000

[layout]
# Active layout mode
mode = "normal"  # Options: "normal", "compact", "custom"
# Split ratio for left/right panels (0.0 to 1.0)
left_width_ratio = 0.3

# Panel configuration per mode
[panels.normal]
left = ["folders", "chats"]
right = ["messages"]
bottom = ["input", "status"]

[panels.compact]
left = ["chats"]
right = ["messages"]
bottom = ["input"]

[panels.custom]
# User-defined custom layout
left = ["folders", "chats"]
right = ["messages"]
bottom = ["input", "status"]

[folders]
enabled = true
display_mode = "tabs"  # Options: "tabs" (filter-based)
# Empty array = show all folders
# Specify folders: ["Personal", "Work", "Important"]
visible_folders = []
show_unread_count = true
default_folder = "All"  # Which folder/tab to show on startup

[chats]
show_unread_count = true
show_last_message_preview = true
sort_by = "recent"  # Options: "recent", "alphabetical", "unread-first"
max_preview_length = 50
show_typing_indicator = true
group_chat_prefix = "[G]"  # Prefix for group chats

[messages]
initial_load_count = 50
show_timestamps = true
show_edited_indicator = true
show_sender_name = true  # For group chats
date_format = "%H:%M"
time_format_24h = true
own_message_align = "right"  # Options: "left", "right"

[input]
show_reply_indicator = true
show_edit_indicator = true
multiline_support = false  # Future: Shift+Enter for newlines
max_input_length = 4096  # Telegram limit

[status_bar]
show = true
position = "bottom"  # Options: "top", "bottom"
show_connection_status = true
show_selected_folder = true
show_unread_total = true

[keybindings]
quit = ["q", "Ctrl+c"]
switch_panel = "Tab"
navigate_up = ["k", "Up"]
navigate_down = ["j", "Down"]
navigate_left = ["h", "Left"]
navigate_right = ["l", "Right"]
select = "Enter"
go_back = "Esc"
edit_message = "e"
reply_to_message = "r"
delete_message = "d"
focus_folders = "1"
focus_chats = "2"
focus_messages = "3"
focus_input = "i"
open_settings = "s"
open_config = "Ctrl+e"
adjust_split_left = "<"
adjust_split_right = ">"
scroll_up = "Ctrl+u"
scroll_down = "Ctrl+d"
page_up = "PageUp"
page_down = "PageDown"
jump_to_top = "g"
jump_to_bottom = "G"
next_folder = "Ctrl+n"
prev_folder = "Ctrl+p"

[theme]
# Reference to theme file (without .toml extension)
name = "catppuccin-mocha"
# Theme files located in: ~/.config/dumbgram/themes/

[ui]
show_borders = true
border_style = "rounded"  # Options: "plain", "rounded", "double", "thick"
use_unicode = true  # Enable unicode characters for better UI
show_help_bar = true  # Show keybinding hints at bottom
```

## Theme File Format

**Location**: `~/.config/dumbgram/themes/catppuccin-mocha.toml`

```toml
# Catppuccin Mocha Theme

[colors]
# Base colors
background = "#1e1e2e"
foreground = "#cdd6f4"
selection = "#585b70"
comment = "#6c7086"

# Accent colors (Catppuccin Mocha palette)
rosewater = "#f5e0dc"
flamingo = "#f2cdcd"
pink = "#f5c2e7"
mauve = "#cba6f7"
red = "#f38ba8"
maroon = "#eba0ac"
peach = "#fab387"
yellow = "#f9e2af"
green = "#a6e3a1"
teal = "#94e2d5"
sky = "#89dceb"
sapphire = "#74c7ec"
blue = "#89b4fa"
lavender = "#b4befe"

# UI element color mappings
[ui_colors]
# Chat list
unread_chat = "yellow"
selected_chat = "mauve"
folder_tab_active = "sapphire"
folder_tab_inactive = "comment"

# Messages
own_message = "blue"
other_message = "foreground"
reply_indicator = "teal"
edited_indicator = "comment"
timestamp = "comment"

# Status & notifications
error = "red"
success = "green"
warning = "peach"
info = "sky"

# UI elements
border = "selection"
border_focused = "mauve"
status_bar = "background"
status_bar_text = "foreground"
input_box = "background"
input_text = "foreground"
cursor = "mauve"

# Connection status
status_online = "green"
status_connecting = "yellow"
status_offline = "red"

# Special
highlight = "yellow"
search_match = "peach"
unread_badge = "red"
```

## Default Theme Files to Include

### 1. Catppuccin Mocha (Default)
Already defined above.

### 2. Catppuccin Latte (Light theme)
**Location**: `~/.config/dumbgram/themes/catppuccin-latte.toml`

### 3. Gruvbox Dark
**Location**: `~/.config/dumbgram/themes/gruvbox-dark.toml`

### 4. Tokyo Night
**Location**: `~/.config/dumbgram/themes/tokyo-night.toml`

### 5. Nord
**Location**: `~/.config/dumbgram/themes/nord.toml`

## Configuration Validation Rules

1. **Required fields**: `telegram.api_id`, `telegram.api_hash`
2. **Numeric ranges**:
   - `left_width_ratio`: 0.1 to 0.9
   - `initial_message_load_count`: 1 to 1000
   - `auto_sync_interval_seconds`: 1 to 3600
3. **Enum validation**:
   - `sort_by`: Must be one of defined options
   - `border_style`: Must be valid ratatui border type
4. **Theme validation**:
   - Theme file must exist
   - All required colors must be defined
   - Color format: `#RRGGBB` hex

## Configuration Loading Priority

1. User config: `~/.config/dumbgram/config.toml`
2. Default config: Built into binary
3. Invalid entries: Fail with error message pointing to invalid line

## Future Enhancements

- Hot reload support (watch config file for changes)
- Per-chat notification settings
- Custom color for specific chats
- Export/import configuration profiles
- Config validation command: `dumbgram --validate-config`
