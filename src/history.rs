use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config;

const MAX_MESSAGES_PER_CONVERSATION: usize = 100;
const MAX_CONVERSATIONS: usize = 50;

/// Key for a conversation: "plugin_id:session_key".
pub type ConversationKey = String;

/// A single persisted message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedMessage {
    pub sender: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    pub text: String,
}

/// Root structure written to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistoryFile {
    pub conversations: HashMap<ConversationKey, Vec<PersistedMessage>>,
}

impl Default for ChatHistoryFile {
    fn default() -> Self {
        Self {
            conversations: HashMap::new(),
        }
    }
}

/// In-memory chat history manager with disk persistence.
pub struct ChatHistory {
    data: ChatHistoryFile,
    path: PathBuf,
    dirty: bool,
}

impl ChatHistory {
    /// Create a new ChatHistory, loading from disk if the file exists.
    pub fn load() -> Self {
        let path = history_path();
        let data = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => serde_json::from_str::<ChatHistoryFile>(&content)
                    .unwrap_or_else(|e| {
                        warn!("failed to parse chat history: {e}");
                        ChatHistoryFile::default()
                    }),
                Err(e) => {
                    warn!("failed to read chat history: {e}");
                    ChatHistoryFile::default()
                }
            }
        } else {
            ChatHistoryFile::default()
        };
        Self {
            data,
            path,
            dirty: false,
        }
    }

    /// Create from explicit path (for testing).
    pub fn load_from(path: PathBuf) -> Self {
        let data = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => serde_json::from_str::<ChatHistoryFile>(&content)
                    .unwrap_or_default(),
                Err(_) => ChatHistoryFile::default(),
            }
        } else {
            ChatHistoryFile::default()
        };
        Self {
            data,
            path,
            dirty: false,
        }
    }

    /// Build a conversation key from plugin_id and session_key.
    pub fn conversation_key(plugin_id: &str, session_key: &str) -> ConversationKey {
        format!("{plugin_id}:{session_key}")
    }

    /// Get messages for a conversation.
    pub fn get_messages(&self, key: &str) -> &[PersistedMessage] {
        self.data
            .conversations
            .get(key)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Push a message to a conversation, enforcing limits.
    pub fn push_message(&mut self, key: &str, msg: PersistedMessage) {
        let messages = self
            .data
            .conversations
            .entry(key.to_string())
            .or_default();
        messages.push(msg);
        while messages.len() > MAX_MESSAGES_PER_CONVERSATION {
            messages.remove(0);
        }
        self.enforce_conversation_limit();
        self.dirty = true;
    }

    /// Set all messages for a conversation (e.g. when loading from state).
    pub fn set_messages(&mut self, key: &str, messages: Vec<PersistedMessage>) {
        let mut msgs = messages;
        while msgs.len() > MAX_MESSAGES_PER_CONVERSATION {
            msgs.remove(0);
        }
        self.data.conversations.insert(key.to_string(), msgs);
        self.enforce_conversation_limit();
        self.dirty = true;
    }

    /// All conversation keys.
    pub fn conversation_keys(&self) -> Vec<&str> {
        self.data.conversations.keys().map(|k| k.as_str()).collect()
    }

    /// Number of conversations stored.
    pub fn conversation_count(&self) -> usize {
        self.data.conversations.len()
    }

    /// Save to disk if dirty. Returns true if a write happened.
    pub fn save_if_dirty(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        self.save();
        true
    }

    /// Force save to disk.
    pub fn save(&mut self) {
        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        match serde_json::to_string_pretty(&self.data) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.path, json) {
                    warn!("failed to write chat history: {e}");
                }
            }
            Err(e) => {
                warn!("failed to serialize chat history: {e}");
            }
        }
        self.dirty = false;
    }

    /// Is there unsaved data?
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Remove oldest conversations to stay under MAX_CONVERSATIONS.
    fn enforce_conversation_limit(&mut self) {
        while self.data.conversations.len() > MAX_CONVERSATIONS {
            // Remove the first key we find (HashMap order is arbitrary,
            // but this prevents unbounded growth)
            if let Some(key) = self.data.conversations.keys().next().cloned() {
                self.data.conversations.remove(&key);
            }
        }
    }
}

fn history_path() -> PathBuf {
    config::app_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("chat_history.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_history(name: &str) -> (ChatHistory, PathBuf) {
        let dir = std::env::temp_dir().join("openclaw_test_history");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.json"));
        let _ = fs::remove_file(&path);
        (ChatHistory::load_from(path.clone()), path)
    }

    // ── Persistence ─────────────────────────────────────────────────

    #[test]
    fn given_messages_when_saved_and_reloaded_then_messages_persist() {
        let (mut h, path) = temp_history("persist");
        let key = ChatHistory::conversation_key("openclaw", "main");

        h.push_message(
            &key,
            PersistedMessage {
                sender: "user".to_string(),
                agent_name: None,
                text: "hello".to_string(),
            },
        );
        h.push_message(
            &key,
            PersistedMessage {
                sender: "agent".to_string(),
                agent_name: Some("Bot".to_string()),
                text: "hi".to_string(),
            },
        );
        h.save();

        let h2 = ChatHistory::load_from(path);
        let msgs = h2.get_messages(&key);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "hello");
        assert_eq!(msgs[0].sender, "user");
        assert_eq!(msgs[1].text, "hi");
        assert_eq!(msgs[1].agent_name, Some("Bot".to_string()));
    }

    // ── Message limit ───────────────────────────────────────────────

    #[test]
    fn given_conversation_with_100_messages_when_new_added_then_oldest_removed() {
        let (mut h, _path) = temp_history("limit");
        let key = "test:limit";

        for i in 0..MAX_MESSAGES_PER_CONVERSATION {
            h.push_message(
                key,
                PersistedMessage {
                    sender: "user".to_string(),
                    agent_name: None,
                    text: format!("msg-{i}"),
                },
            );
        }
        assert_eq!(h.get_messages(key).len(), MAX_MESSAGES_PER_CONVERSATION);

        h.push_message(
            key,
            PersistedMessage {
                sender: "user".to_string(),
                agent_name: None,
                text: "overflow".to_string(),
            },
        );

        let msgs = h.get_messages(key);
        assert_eq!(msgs.len(), MAX_MESSAGES_PER_CONVERSATION);
        assert_eq!(msgs[0].text, "msg-1", "msg-0 should be evicted");
        assert_eq!(
            msgs[MAX_MESSAGES_PER_CONVERSATION - 1].text,
            "overflow"
        );
    }

    // ── Multiple conversations ──────────────────────────────────────

    #[test]
    fn given_multiple_conversations_then_each_has_own_history() {
        let (mut h, path) = temp_history("multi");
        let key_a = ChatHistory::conversation_key("openclaw", "main");
        let key_b = ChatHistory::conversation_key("ollama", "default");

        h.push_message(
            &key_a,
            PersistedMessage {
                sender: "user".to_string(),
                agent_name: None,
                text: "hello openclaw".to_string(),
            },
        );
        h.push_message(
            &key_b,
            PersistedMessage {
                sender: "user".to_string(),
                agent_name: None,
                text: "hello ollama".to_string(),
            },
        );
        h.save();

        let h2 = ChatHistory::load_from(path);
        assert_eq!(h2.get_messages(&key_a).len(), 1);
        assert_eq!(h2.get_messages(&key_b).len(), 1);
        assert_eq!(h2.get_messages(&key_a)[0].text, "hello openclaw");
        assert_eq!(h2.get_messages(&key_b)[0].text, "hello ollama");
    }

    // ── Conversation limit ──────────────────────────────────────────

    #[test]
    fn given_50_conversations_when_51st_added_then_one_is_pruned() {
        let (mut h, _path) = temp_history("convlimit");

        for i in 0..=MAX_CONVERSATIONS {
            let key = format!("plugin:{i}");
            h.push_message(
                &key,
                PersistedMessage {
                    sender: "user".to_string(),
                    agent_name: None,
                    text: format!("msg-{i}"),
                },
            );
        }

        assert!(
            h.conversation_count() <= MAX_CONVERSATIONS,
            "should not exceed {} conversations, got {}",
            MAX_CONVERSATIONS,
            h.conversation_count()
        );
    }

    // ── Dirty tracking ──────────────────────────────────────────────

    #[test]
    fn given_new_history_then_not_dirty() {
        let (h, _path) = temp_history("dirty");
        assert!(!h.is_dirty());
    }

    #[test]
    fn given_message_pushed_then_dirty() {
        let (mut h, _path) = temp_history("dirty2");
        h.push_message(
            "test:x",
            PersistedMessage {
                sender: "user".to_string(),
                agent_name: None,
                text: "hi".to_string(),
            },
        );
        assert!(h.is_dirty());
    }

    #[test]
    fn given_dirty_when_saved_then_not_dirty() {
        let (mut h, _path) = temp_history("dirty3");
        h.push_message(
            "test:x",
            PersistedMessage {
                sender: "user".to_string(),
                agent_name: None,
                text: "hi".to_string(),
            },
        );
        h.save();
        assert!(!h.is_dirty());
    }

    // ── Conversation key format ─────────────────────────────────────

    #[test]
    fn conversation_key_format() {
        let key = ChatHistory::conversation_key("ollama-local", "default");
        assert_eq!(key, "ollama-local:default");
    }

    // ── Empty / missing file ────────────────────────────────────────

    #[test]
    fn given_no_file_then_empty_history() {
        let path = std::env::temp_dir().join("openclaw_test_history/nonexistent.json");
        let _ = std::fs::remove_file(&path);
        let h = ChatHistory::load_from(path);
        assert_eq!(h.conversation_count(), 0);
    }

    #[test]
    fn given_corrupt_file_then_empty_history() {
        let dir = std::env::temp_dir().join("openclaw_test_history");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("corrupt.json");
        fs::write(&path, "not valid json {{{").unwrap();
        let h = ChatHistory::load_from(path);
        assert_eq!(h.conversation_count(), 0);
    }

    // ── save_if_dirty ───────────────────────────────────────────────

    #[test]
    fn save_if_dirty_returns_false_when_clean() {
        let (mut h, _path) = temp_history("clean_save");
        assert!(!h.save_if_dirty());
    }

    #[test]
    fn save_if_dirty_returns_true_when_dirty() {
        let (mut h, _path) = temp_history("dirty_save");
        h.push_message(
            "test:x",
            PersistedMessage {
                sender: "user".to_string(),
                agent_name: None,
                text: "hi".to_string(),
            },
        );
        assert!(h.save_if_dirty());
        assert!(!h.is_dirty());
    }
}
