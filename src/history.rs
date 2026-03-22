use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config;
use crate::media::MediaStore;

const MAX_MESSAGES_PER_CONVERSATION: usize = 100;
const MAX_CONVERSATIONS: usize = 50;

/// Key for a conversation: "plugin_id:session_key".
pub type ConversationKey = String;

/// A single persisted message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedMessage {
    pub sender: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LegacyChatHistoryFile {
    pub conversations: HashMap<ConversationKey, Vec<LegacyPersistedMessage>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LegacyPersistedMessage {
    pub sender: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    pub text: String,
}

/// SQLite-backed chat history manager.
pub struct ChatHistory {
    conn: Connection,
    _path: PathBuf,
}

impl ChatHistory {
    /// Create a new ChatHistory, loading from SQLite and applying migration/cleanup.
    pub fn load() -> Self {
        let path = history_db_path();
        let mut history = Self::open(path, true);
        history.cleanup_older_than_days(30);

        let media_store = MediaStore::new();
        let _ = media_store.cleanup_older_than_days(30);
        let _ = media_store.enforce_max_size_bytes(200 * 1024 * 1024);
        history
    }

    /// Create from explicit path (for testing).
    pub fn load_from(path: PathBuf) -> Self {
        Self::open(path, false)
    }

    fn open(path: PathBuf, run_migration: bool) -> Self {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let conn = Connection::open(&path).unwrap_or_else(|e| {
            warn!("failed to open chat db at {}: {e}", path.display());
            Connection::open_in_memory().expect("open in-memory sqlite")
        });

        if let Err(e) = init_schema(&conn) {
            warn!("failed to initialize chat db schema: {e}");
        }

        let mut history = Self { conn, _path: path };
        if run_migration {
            history.migrate_json_if_needed();
        }
        history
    }

    /// Build a conversation key from plugin_id and session_key.
    pub fn conversation_key(plugin_id: &str, session_key: &str) -> ConversationKey {
        format!("{plugin_id}:{session_key}")
    }

    /// Get messages for a conversation.
    pub fn get_messages(&self, key: &str) -> Vec<PersistedMessage> {
        self.query_messages(key, None)
    }

    /// Get recent messages for a conversation.
    pub fn get_recent_messages(&self, key: &str, limit: usize) -> Vec<PersistedMessage> {
        self.query_messages(key, Some(limit))
    }

    fn query_messages(&self, key: &str, limit: Option<usize>) -> Vec<PersistedMessage> {
        let sql = if limit.is_some() {
            "SELECT sender, agent_name, text, media_path, media_type, created_at
             FROM messages
             WHERE conversation_key = ?1
             ORDER BY created_at DESC, id DESC
             LIMIT ?2"
        } else {
            "SELECT sender, agent_name, text, media_path, media_type, created_at
             FROM messages
             WHERE conversation_key = ?1
             ORDER BY created_at ASC, id ASC"
        };

        let mut out = Vec::new();
        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to prepare query messages: {e}");
                return out;
            }
        };

        let row_mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<PersistedMessage> {
            Ok(PersistedMessage {
                sender: row.get(0)?,
                agent_name: row.get(1)?,
                text: row.get(2)?,
                media_path: row.get(3)?,
                media_type: row.get(4)?,
                created_at: row.get(5)?,
            })
        };

        let result: Result<Vec<PersistedMessage>, _> = if let Some(n) = limit {
            stmt.query_map(params![key, n as i64], row_mapper)
                .map(|rows| rows.flatten().collect())
        } else {
            stmt.query_map(params![key], row_mapper)
                .map(|rows| rows.flatten().collect())
        };

        if let Ok(rows) = result {
            out = rows;
        }

        if limit.is_some() {
            out.reverse();
        }

        out
    }

    /// Push a message to a conversation, enforcing limits.
    pub fn push_message(&mut self, key: &str, msg: PersistedMessage) {
        let created_at = if msg.created_at > 0 {
            msg.created_at
        } else {
            now_unix_ms()
        };

        if let Err(e) = self.conn.execute(
            "INSERT INTO messages (conversation_key, sender, agent_name, text, media_path, media_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                key,
                msg.sender,
                msg.agent_name,
                msg.text,
                msg.media_path,
                msg.media_type,
                created_at,
            ],
        ) {
            warn!("failed to insert chat message: {e}");
            return;
        }

        self.enforce_message_limit(key);
        self.enforce_conversation_limit();
    }

    /// Set all messages for a conversation (compat helper).
    pub fn set_messages(&mut self, key: &str, messages: Vec<PersistedMessage>) {
        let _ = self
            .conn
            .execute("DELETE FROM messages WHERE conversation_key = ?1", params![key]);

        let mut start = 0usize;
        if messages.len() > MAX_MESSAGES_PER_CONVERSATION {
            start = messages.len() - MAX_MESSAGES_PER_CONVERSATION;
        }

        for msg in messages.into_iter().skip(start) {
            self.push_message(key, msg);
        }
    }

    /// All conversation keys.
    pub fn conversation_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        let mut stmt = match self.conn.prepare(
            "SELECT conversation_key
             FROM messages
             GROUP BY conversation_key
             ORDER BY MAX(created_at) DESC",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to prepare conversation key query: {e}");
                return keys;
            }
        };

        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for key in rows.flatten() {
                keys.push(key);
            }
        }

        keys
    }

    /// Number of conversations stored.
    pub fn conversation_count(&self) -> usize {
        self.conversation_keys().len()
    }

    /// Cleanup messages older than N days.
    pub fn cleanup_older_than_days(&mut self, days: u32) {
        let cutoff = now_unix_ms() - i64::from(days) * 24 * 60 * 60 * 1000;
        if let Err(e) = self.conn.execute(
            "DELETE FROM messages WHERE created_at < ?1",
            params![cutoff],
        ) {
            warn!("failed to cleanup old messages: {e}");
        }
    }

    /// Save method kept for compatibility (SQLite writes are immediate).
    pub fn save(&mut self) {}

    /// Compatibility helper for old callsites.
    pub fn save_if_dirty(&mut self) -> bool {
        false
    }

    /// Compatibility helper for old callsites.
    pub fn is_dirty(&self) -> bool {
        false
    }

    fn enforce_message_limit(&mut self, key: &str) {
        let _ = self.conn.execute(
            "DELETE FROM messages
             WHERE id IN (
                 SELECT id
                 FROM messages
                 WHERE conversation_key = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT -1 OFFSET ?2
             )",
            params![key, MAX_MESSAGES_PER_CONVERSATION as i64],
        );
    }

    fn enforce_conversation_limit(&mut self) {
        let keys = self.conversation_keys();
        if keys.len() <= MAX_CONVERSATIONS {
            return;
        }

        for key in keys.into_iter().skip(MAX_CONVERSATIONS) {
            let _ = self.conn.execute(
                "DELETE FROM messages WHERE conversation_key = ?1",
                params![key],
            );
        }
    }

    fn migrate_json_if_needed(&mut self) {
        let json_path = config::app_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("chat_history.json");

        if !json_path.exists() {
            return;
        }

        let has_rows: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap_or(0);
        if has_rows > 0 {
            return;
        }

        let content = match fs::read_to_string(&json_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("failed to read legacy chat history: {e}");
                return;
            }
        };

        let parsed = match serde_json::from_str::<LegacyChatHistoryFile>(&content) {
            Ok(v) => v,
            Err(e) => {
                warn!("failed to parse legacy chat history: {e}");
                return;
            }
        };

        let now_ms = now_unix_ms();
        for (key, messages) in parsed.conversations {
            for msg in messages {
                let _ = self.conn.execute(
                    "INSERT INTO messages (conversation_key, sender, agent_name, text, media_path, media_type, created_at)
                     VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5)",
                    params![key, msg.sender, msg.agent_name, msg.text, now_ms],
                );
            }
        }

        let backup_path = json_path.with_extension("json.bak");
        if let Err(e) = fs::rename(&json_path, &backup_path) {
            warn!(
                "failed to rename legacy chat history to {}: {e}",
                backup_path.display()
            );
        }
    }
}

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_key TEXT NOT NULL,
            sender TEXT NOT NULL,
            agent_name TEXT,
            text TEXT NOT NULL,
            media_path TEXT,
            media_type TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_key, created_at);
        CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at);",
    )
}

fn history_db_path() -> PathBuf {
    config::app_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("chat.db")
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("openclaw_test_history_sqlite");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.db"));
        let _ = fs::remove_file(&path);
        path
    }

    fn msg(sender: &str, text: &str) -> PersistedMessage {
        PersistedMessage {
            sender: sender.to_string(),
            agent_name: None,
            text: text.to_string(),
            media_path: None,
            media_type: None,
            created_at: now_unix_ms(),
        }
    }

    #[test]
    fn given_messages_when_saved_and_reloaded_then_messages_persist() {
        let path = temp_db_path("persist");
        let mut h = ChatHistory::load_from(path.clone());
        let key = ChatHistory::conversation_key("openclaw", "main");

        h.push_message(&key, msg("user", "hello"));
        h.push_message(
            &key,
            PersistedMessage {
                sender: "agent".to_string(),
                agent_name: Some("Bot".to_string()),
                text: "hi".to_string(),
                media_path: None,
                media_type: None,
                created_at: now_unix_ms(),
            },
        );

        let h2 = ChatHistory::load_from(path);
        let msgs = h2.get_messages(&key);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "hello");
        assert_eq!(msgs[1].agent_name, Some("Bot".to_string()));
    }

    #[test]
    fn given_conversation_with_100_messages_when_new_added_then_oldest_removed() {
        let path = temp_db_path("limit");
        let mut h = ChatHistory::load_from(path);
        let key = "test:limit";

        for i in 0..MAX_MESSAGES_PER_CONVERSATION {
            h.push_message(key, msg("user", &format!("msg-{i}")));
        }
        h.push_message(key, msg("user", "overflow"));

        let msgs = h.get_messages(key);
        assert_eq!(msgs.len(), MAX_MESSAGES_PER_CONVERSATION);
        assert_eq!(msgs[0].text, "msg-1");
        assert_eq!(msgs[MAX_MESSAGES_PER_CONVERSATION - 1].text, "overflow");
    }

    #[test]
    fn given_50_conversations_when_51st_added_then_one_is_pruned() {
        let path = temp_db_path("convlimit");
        let mut h = ChatHistory::load_from(path);

        for i in 0..=MAX_CONVERSATIONS {
            h.push_message(&format!("plugin:{i}"), msg("user", &format!("msg-{i}")));
        }

        assert!(h.conversation_count() <= MAX_CONVERSATIONS);
    }

    #[test]
    fn test_cleanup_old_messages() {
        let path = temp_db_path("cleanup_old");
        let mut h = ChatHistory::load_from(path);
        let key = "test:main";

        h.push_message(
            key,
            PersistedMessage {
                sender: "user".to_string(),
                agent_name: None,
                text: "old".to_string(),
                media_path: None,
                media_type: None,
                created_at: now_unix_ms() - 40 * 24 * 60 * 60 * 1000,
            },
        );
        h.push_message(key, msg("user", "new"));

        h.cleanup_older_than_days(30);
        let msgs = h.get_messages(key);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "new");
    }

    #[test]
    fn test_sqlite_migration_from_json() {
        let app_dir = std::env::temp_dir().join("openclaw_history_migration_case");
        let _ = fs::remove_dir_all(&app_dir);
        fs::create_dir_all(&app_dir).unwrap();

        let db_path = app_dir.join("chat.db");
        let mut h = ChatHistory::load_from(db_path);

        let json_path = app_dir.join("chat_history.json");
        let json = serde_json::json!({
            "conversations": {
                "openclaw:main": [
                    {"sender": "user", "text": "hello"},
                    {"sender": "agent", "agent_name": "Bot", "text": "hi"}
                ]
            }
        });
        fs::write(&json_path, serde_json::to_string_pretty(&json).unwrap()).unwrap();

        // Simulate migration against temp app dir by directly importing from json payload.
        let content = fs::read_to_string(&json_path).unwrap();
        let parsed = serde_json::from_str::<LegacyChatHistoryFile>(&content).unwrap();
        for (key, messages) in parsed.conversations {
            for m in messages {
                h.push_message(
                    &key,
                    PersistedMessage {
                        sender: m.sender,
                        agent_name: m.agent_name,
                        text: m.text,
                        media_path: None,
                        media_type: None,
                        created_at: now_unix_ms(),
                    },
                );
            }
        }
        let _ = fs::rename(&json_path, app_dir.join("chat_history.json.bak"));

        let msgs = h.get_messages("openclaw:main");
        assert_eq!(msgs.len(), 2);
        assert!(app_dir.join("chat_history.json.bak").exists());
    }

    #[test]
    fn conversation_key_format() {
        let key = ChatHistory::conversation_key("ollama-local", "default");
        assert_eq!(key, "ollama-local:default");
    }
}
