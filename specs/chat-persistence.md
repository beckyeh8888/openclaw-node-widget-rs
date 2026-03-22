# Chat Persistence — SQLite + Media Storage

## Goal
Replace the current JSON-based chat history (`history.rs`) with SQLite for durability,
and add local media file storage with automatic cleanup.

## Current State
- `src/history.rs`: JSON file at `{app_dir}/chat_history.json`
- Structs: `PersistedMessage { sender, agent_name, text }`, `ChatHistoryFile { conversations }`
- `ChatHistory` loads on startup, saves on dirty flag
- No media/attachment support
- No timestamps on messages
- No automatic cleanup/rotation

## Requirements

### 1. SQLite Storage (`src/history.rs` rewrite)

Replace JSON with SQLite using `rusqlite` (with `bundled` feature for zero system deps).

**Schema:**
```sql
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_key TEXT NOT NULL,  -- "plugin_id:session_key"
    sender TEXT NOT NULL,            -- "user" or "agent"
    agent_name TEXT,                 -- nullable
    text TEXT NOT NULL,
    media_path TEXT,                 -- relative path under media/ dir, nullable
    media_type TEXT,                 -- MIME type, nullable
    created_at INTEGER NOT NULL      -- unix timestamp ms
);

CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_key, created_at);
CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at);
```

**Migration:** On first startup with SQLite, if `chat_history.json` exists:
1. Read JSON, insert all messages into SQLite (set created_at to now)
2. Rename JSON to `chat_history.json.bak`

**Public API** (keep compatible with current `ChatHistory` interface):
```rust
impl ChatHistory {
    pub fn load() -> Self;  // opens/creates SQLite DB
    pub fn get_messages(&self, key: &str) -> Vec<PersistedMessage>;  // returns owned vec now
    pub fn get_recent_messages(&self, key: &str, limit: usize) -> Vec<PersistedMessage>;
    pub fn push_message(&mut self, key: &str, msg: PersistedMessage);
    pub fn conversation_keys(&self) -> Vec<String>;
    pub fn conversation_count(&self) -> usize;
    pub fn cleanup_older_than_days(&mut self, days: u32);  // NEW
    pub fn save(&mut self);  // no-op for SQLite (writes are immediate), keep for compat
}
```

**PersistedMessage** updated:
```rust
pub struct PersistedMessage {
    pub sender: String,
    pub agent_name: Option<String>,
    pub text: String,
    pub media_path: Option<String>,   // NEW
    pub media_type: Option<String>,   // NEW
    pub created_at: i64,              // NEW: unix ms
}
```

### 2. Media Storage

**Directory structure:**
```
{app_dir}/
├── chat.db
└── media/
    └── YYYY-MM/
        ├── {uuid}.png
        ├── {uuid}.jpg
        └── {uuid}.pdf
```

**New module** `src/media.rs`:
```rust
pub struct MediaStore { base_dir: PathBuf }

impl MediaStore {
    pub fn new() -> Self;  // base = {app_dir}/media
    pub fn store_file(&self, data: &[u8], mime: &str) -> Result<String>;  // returns relative path
    pub fn store_from_url(&self, url: &str) -> Result<String>;  // download + store
    pub fn get_full_path(&self, relative: &str) -> PathBuf;
    pub fn cleanup_older_than_days(&self, days: u32) -> Result<u64>;  // returns files deleted
    pub fn total_size_bytes(&self) -> Result<u64>;
}
```

- Files named `{uuid7}.{ext}` (uuid7 for time-sortable)
- Max single file: 10MB; larger files store a placeholder message "File too large"
- Images > 2MB: store a resized thumbnail (max 1920px wide) alongside original? No — just store original, let WebView handle display.

### 3. Automatic Cleanup

On startup (`ChatHistory::load`):
1. Delete messages older than 30 days from SQLite
2. Delete media files older than 30 days
3. If media dir > 200MB, delete oldest files until under limit

Make retention configurable later via Settings (not in this PR).

### 4. Integration Points

**`src/chat.rs`** changes:
- `ChatState` holds `MediaStore` alongside `ChatHistory`
- When receiving `ChatInbound` with attachments → `media_store.store_file()` → persist path in message
- When user sends image via chat → same flow
- WebView HTML renders `<img src="file:///{full_path}">` for image messages

**`src/main.rs`** changes:
- Initialize `MediaStore` alongside `ChatHistory`
- Pass to `ChatState`

**`Cargo.toml`** additions:
```toml
rusqlite = { version = "0.32", features = ["bundled"] }
uuid = { version = "1", features = ["v7"] }
```

### 5. WebView Rendering

In the chat HTML/JS (inside `chat.rs` webview code):
- Messages with `media_path` ending in image extensions → render `<img>` with click-to-fullscreen
- Other file types → render as download link with filename + size
- Lazy load: only render images in viewport

### 6. Tests

Update existing tests in `history.rs` to work with SQLite.
Add:
- `test_sqlite_migration_from_json`
- `test_cleanup_old_messages`
- `test_media_store_and_retrieve`
- `test_media_cleanup_by_age`
- `test_media_cleanup_by_size`

### 7. What NOT to do
- Don't change the plugin architecture
- Don't change Gateway WebSocket protocol
- Don't add new tray menu items
- Don't change the chat UI layout (just add image rendering capability)
- Keep all existing tests passing
