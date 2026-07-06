//! LLM Wiki — structured session summary storage
//!
//! Stores FM-generated summaries of coding sessions in a local SQLite database.
//! Used by `tokscale report` to provide task-attributed usage reports.

use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A single wiki entry representing a summarized coding session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WikiEntry {
    pub session_id: String,
    pub client: String,
    pub workspace: Option<String>,
    pub workspace_label: Option<String>,
    pub created_at: i64,
    pub last_active: i64,

    pub title: Option<String>,
    pub task_category: Option<String>,
    pub description: Option<String>,
    pub complexity: Option<String>,
    pub task_group: Option<String>,

    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read: i64,
    pub total_cost: f64,
    pub models_used: Vec<String>,
    pub message_count: i32,
    pub duration_minutes: i64,

    pub summarized_at: Option<i64>,
    pub fm_version: Option<String>,
}

/// Task categories that the FM can assign to sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TaskCategory {
    Feature,
    Bugfix,
    Refactor,
    Research,
    Debug,
    Review,
    Docs,
    Config,
    Other,
}

impl TaskCategory {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "feature" | "feat" => Self::Feature,
            "bugfix" | "bug" | "fix" => Self::Bugfix,
            "refactor" | "refactoring" => Self::Refactor,
            "research" | "explore" | "investigation" => Self::Research,
            "debug" | "debugging" => Self::Debug,
            "review" | "code review" => Self::Review,
            "docs" | "documentation" => Self::Docs,
            "config" | "configuration" | "setup" => Self::Config,
            _ => Self::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Bugfix => "bugfix",
            Self::Refactor => "refactor",
            Self::Research => "research",
            Self::Debug => "debug",
            Self::Review => "review",
            Self::Docs => "docs",
            Self::Config => "config",
            Self::Other => "other",
        }
    }
}

/// Wiki database handle.
pub struct WikiDb {
    conn: Connection,
}

impl WikiDb {
    /// Open (or create) the wiki database at the given path.
    pub fn open(path: &Path) -> Result<Self, WikiError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WikiError::Io(e.to_string()))?;
        }

        let conn = Connection::open(path).map_err(|e| WikiError::Sqlite(e.to_string()))?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        conn.execute_batch(Self::SCHEMA)
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        let has_task_group: bool = conn
            .prepare("SELECT task_group FROM wiki_entries LIMIT 0")
            .is_ok();
        if !has_task_group {
            conn.execute_batch("ALTER TABLE wiki_entries ADD COLUMN task_group TEXT")
                .map_err(|e| WikiError::Sqlite(e.to_string()))?;
        }

        Ok(Self { conn })
    }

    /// Default wiki DB path: ~/.config/tokscale/wiki.db
    pub fn default_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
            })
            .join("tokscale");
        config_dir.join("wiki.db")
    }

    const SCHEMA: &'static str = r#"
        CREATE TABLE IF NOT EXISTS wiki_entries (
            session_id TEXT PRIMARY KEY,
            client TEXT NOT NULL,
            workspace TEXT,
            workspace_label TEXT,
            created_at INTEGER NOT NULL,
            last_active INTEGER NOT NULL,
            title TEXT,
            task_category TEXT,
            description TEXT,
            complexity TEXT,
            task_group TEXT,
            total_input_tokens INTEGER NOT NULL DEFAULT 0,
            total_output_tokens INTEGER NOT NULL DEFAULT 0,
            total_cache_read INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0.0,
            models_used TEXT NOT NULL DEFAULT '[]',
            message_count INTEGER NOT NULL DEFAULT 0,
            duration_minutes INTEGER NOT NULL DEFAULT 0,
            summarized_at INTEGER,
            fm_version TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_wiki_workspace ON wiki_entries(workspace);
        CREATE INDEX IF NOT EXISTS idx_wiki_category ON wiki_entries(task_category);
        CREATE INDEX IF NOT EXISTS idx_wiki_created ON wiki_entries(created_at);
        CREATE INDEX IF NOT EXISTS idx_wiki_client ON wiki_entries(client);
        CREATE INDEX IF NOT EXISTS idx_wiki_task_group ON wiki_entries(task_group);
    "#;

    /// Get all session IDs that are already in the wiki.
    pub fn get_existing_session_ids(&self) -> Result<HashSet<String>, WikiError> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id FROM wiki_entries")
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| WikiError::Sqlite(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(ids)
    }

    /// Get session IDs that have NOT been summarized yet (title is NULL).
    pub fn get_unsummarized_session_ids(&self) -> Result<Vec<String>, WikiError> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id FROM wiki_entries WHERE title IS NULL")
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| WikiError::Sqlite(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(ids)
    }

    /// Reset all summaries (set title/category/description/complexity to NULL).
    pub fn reset_all_summaries(&self) -> Result<usize, WikiError> {
        let count = self
            .conn
            .execute(
                "UPDATE wiki_entries SET title = NULL, task_category = NULL, description = NULL, complexity = NULL, task_group = NULL, summarized_at = NULL, fm_version = NULL",
                [],
            )
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;
        Ok(count)
    }

    pub fn reset_summaries_in_range(
        &self,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<usize, WikiError> {
        let (sql, params_vec) = match (since, until) {
            (Some(s), Some(u)) => (
                "UPDATE wiki_entries SET title = NULL, task_category = NULL, description = NULL, complexity = NULL, task_group = NULL, summarized_at = NULL, fm_version = NULL WHERE created_at >= ?1 AND created_at < ?2".to_string(),
                vec![s, u],
            ),
            (Some(s), None) => (
                "UPDATE wiki_entries SET title = NULL, task_category = NULL, description = NULL, complexity = NULL, task_group = NULL, summarized_at = NULL, fm_version = NULL WHERE created_at >= ?1".to_string(),
                vec![s],
            ),
            (None, Some(u)) => (
                "UPDATE wiki_entries SET title = NULL, task_category = NULL, description = NULL, complexity = NULL, task_group = NULL, summarized_at = NULL, fm_version = NULL WHERE created_at < ?1".to_string(),
                vec![u],
            ),
            (None, None) => (
                "UPDATE wiki_entries SET title = NULL, task_category = NULL, description = NULL, complexity = NULL, task_group = NULL, summarized_at = NULL, fm_version = NULL".to_string(),
                vec![],
            ),
        };

        let count = self
            .conn
            .execute(&sql, rusqlite::params_from_iter(params_vec.iter()))
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;
        Ok(count)
    }

    pub fn get_unsummarized_session_ids_in_range(
        &self,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<String>, WikiError> {
        let (sql, params_vec) = match (since, until) {
            (Some(s), Some(u)) => (
                "SELECT session_id FROM wiki_entries WHERE title IS NULL AND created_at >= ?1 AND created_at < ?2".to_string(),
                vec![s, u],
            ),
            (Some(s), None) => (
                "SELECT session_id FROM wiki_entries WHERE title IS NULL AND created_at >= ?1".to_string(),
                vec![s],
            ),
            (None, Some(u)) => (
                "SELECT session_id FROM wiki_entries WHERE title IS NULL AND created_at < ?1".to_string(),
                vec![u],
            ),
            (None, None) => (
                "SELECT session_id FROM wiki_entries WHERE title IS NULL".to_string(),
                vec![],
            ),
        };

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        let ids = stmt
            .query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| WikiError::Sqlite(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(ids)
    }

    /// Insert or update a wiki entry (upsert).
    pub fn upsert_entry(&self, entry: &WikiEntry) -> Result<(), WikiError> {
        let models_json =
            serde_json::to_string(&entry.models_used).unwrap_or_else(|_| "[]".to_string());

        self.conn
            .execute(
                r#"
                INSERT INTO wiki_entries (
                    session_id, client, workspace, workspace_label,
                    created_at, last_active,
                    title, task_category, description, complexity, task_group,
                    total_input_tokens, total_output_tokens, total_cache_read, total_cost,
                    models_used, message_count, duration_minutes,
                    summarized_at, fm_version
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
                ON CONFLICT(session_id) DO UPDATE SET
                    last_active = excluded.last_active,
                    title = COALESCE(excluded.title, wiki_entries.title),
                    task_category = COALESCE(excluded.task_category, wiki_entries.task_category),
                    description = COALESCE(excluded.description, wiki_entries.description),
                    complexity = COALESCE(excluded.complexity, wiki_entries.complexity),
                    task_group = COALESCE(excluded.task_group, wiki_entries.task_group),
                    total_input_tokens = excluded.total_input_tokens,
                    total_output_tokens = excluded.total_output_tokens,
                    total_cache_read = excluded.total_cache_read,
                    total_cost = excluded.total_cost,
                    models_used = excluded.models_used,
                    message_count = excluded.message_count,
                    duration_minutes = excluded.duration_minutes,
                    summarized_at = COALESCE(excluded.summarized_at, wiki_entries.summarized_at),
                    fm_version = COALESCE(excluded.fm_version, wiki_entries.fm_version)
                "#,
                params![
                    entry.session_id,
                    entry.client,
                    entry.workspace,
                    entry.workspace_label,
                    entry.created_at,
                    entry.last_active,
                    entry.title,
                    entry.task_category,
                    entry.description,
                    entry.complexity,
                    entry.task_group,
                    entry.total_input_tokens,
                    entry.total_output_tokens,
                    entry.total_cache_read,
                    entry.total_cost,
                    models_json,
                    entry.message_count,
                    entry.duration_minutes,
                    entry.summarized_at,
                    entry.fm_version,
                ],
            )
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        Ok(())
    }

    /// Update only the FM-generated summary fields for a session.
    pub fn update_summary(
        &self,
        session_id: &str,
        title: &str,
        task_category: &str,
        description: &str,
        complexity: &str,
        fm_version: Option<&str>,
    ) -> Result<(), WikiError> {
        let now = chrono::Utc::now().timestamp();

        self.conn
            .execute(
                r#"
                UPDATE wiki_entries SET
                    title = ?1,
                    task_category = ?2,
                    description = ?3,
                    complexity = ?4,
                    summarized_at = ?5,
                    fm_version = ?6
                WHERE session_id = ?7
                "#,
                params![
                    title,
                    task_category,
                    description,
                    complexity,
                    now,
                    fm_version,
                    session_id
                ],
            )
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        Ok(())
    }

    pub fn update_task_group(&self, session_id: &str, task_group: &str) -> Result<(), WikiError> {
        self.conn
            .execute(
                "UPDATE wiki_entries SET task_group = ?1 WHERE session_id = ?2",
                params![task_group, session_id],
            )
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Query entries filtered by date range and optional workspace/client.
    pub fn query_entries(
        &self,
        since: Option<i64>,
        until: Option<i64>,
        workspace: Option<&str>,
        client: Option<&str>,
    ) -> Result<Vec<WikiEntry>, WikiError> {
        let mut sql = String::from("SELECT * FROM wiki_entries WHERE 1=1");
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(s) = since {
            sql.push_str(" AND created_at >= ?");
            param_values.push(Box::new(s));
        }
        if let Some(u) = until {
            sql.push_str(" AND created_at < ?");
            param_values.push(Box::new(u));
        }
        if let Some(w) = workspace {
            sql.push_str(" AND workspace = ?");
            param_values.push(Box::new(w.to_string()));
        }
        if let Some(c) = client {
            sql.push_str(" AND client = ?");
            param_values.push(Box::new(c.to_string()));
        }

        sql.push_str(" ORDER BY created_at DESC");

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let entries = stmt
            .query_map(params_refs.as_slice(), |row| {
                let models_str: String = row.get(15)?;
                let models: Vec<String> = serde_json::from_str(&models_str).unwrap_or_default();

                Ok(WikiEntry {
                    session_id: row.get(0)?,
                    client: row.get(1)?,
                    workspace: row.get(2)?,
                    workspace_label: row.get(3)?,
                    created_at: row.get(4)?,
                    last_active: row.get(5)?,
                    title: row.get(6)?,
                    task_category: row.get(7)?,
                    description: row.get(8)?,
                    complexity: row.get(9)?,
                    task_group: row.get(10)?,
                    total_input_tokens: row.get(11)?,
                    total_output_tokens: row.get(12)?,
                    total_cache_read: row.get(13)?,
                    total_cost: row.get(14)?,
                    models_used: models,
                    message_count: row.get(16)?,
                    duration_minutes: row.get(17)?,
                    summarized_at: row.get(18)?,
                    fm_version: row.get(19)?,
                })
            })
            .map_err(|e| WikiError::Sqlite(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    /// Get a single entry by session_id.
    pub fn get_entry(&self, session_id: &str) -> Result<Option<WikiEntry>, WikiError> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM wiki_entries WHERE session_id = ?1")
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        let entry = stmt
            .query_row(params![session_id], |row| {
                let models_str: String = row.get(15)?;
                let models: Vec<String> = serde_json::from_str(&models_str).unwrap_or_default();

                Ok(WikiEntry {
                    session_id: row.get(0)?,
                    client: row.get(1)?,
                    workspace: row.get(2)?,
                    workspace_label: row.get(3)?,
                    created_at: row.get(4)?,
                    last_active: row.get(5)?,
                    title: row.get(6)?,
                    task_category: row.get(7)?,
                    description: row.get(8)?,
                    complexity: row.get(9)?,
                    task_group: row.get(10)?,
                    total_input_tokens: row.get(11)?,
                    total_output_tokens: row.get(12)?,
                    total_cache_read: row.get(13)?,
                    total_cost: row.get(14)?,
                    models_used: models,
                    message_count: row.get(16)?,
                    duration_minutes: row.get(17)?,
                    summarized_at: row.get(18)?,
                    fm_version: row.get(19)?,
                })
            })
            .optional()
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;

        Ok(entry)
    }

    /// Count total entries in the wiki.
    pub fn count(&self) -> Result<usize, WikiError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM wiki_entries", [], |row| row.get(0))
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;
        Ok(count as usize)
    }

    /// Count entries that have been summarized (title IS NOT NULL).
    pub fn count_summarized(&self) -> Result<usize, WikiError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM wiki_entries WHERE title IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|e| WikiError::Sqlite(e.to_string()))?;
        Ok(count as usize)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WikiError {
    #[error("SQLite error: {0}")]
    Sqlite(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Serialization error: {0}")]
    Serde(String),
}
