use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::config::Config;
use crate::error::Result;
use crate::interests::GlobalMemory;
use crate::watch::{AgentMemory, Change, Engine, Extraction, Reminder, Snapshot, Watch};

/// Safely convert a Unix timestamp to DateTime<Utc>, falling back to current time if invalid
fn timestamp_to_datetime(timestamp: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

/// Database connection wrapper
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create the database
    pub fn open() -> Result<Self> {
        let db_path = Config::db_path()?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut conn = Connection::open(&db_path)?;

        // Run migrations
        embedded::migrations::runner().run(&mut conn)?;

        // Enable foreign keys
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing)
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        embedded::migrations::runner().run(&mut conn)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn })
    }

    // ========== Watch operations ==========

    /// Insert a new watch
    pub fn insert_watch(&self, watch: &Watch) -> Result<()> {
        let result = self.conn.execute(
            "INSERT INTO watches (id, name, url, engine, extraction, normalization, filters,
             agent_config, interval_secs, enabled, created_at, headers, cookie_file, storage_state, notify_target, tags, use_profile)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                watch.id.to_string(),
                watch.name,
                watch.url,
                serde_json::to_string(&watch.engine)?,
                serde_json::to_string(&watch.extraction)?,
                serde_json::to_string(&watch.normalization)?,
                serde_json::to_string(&watch.filters)?,
                watch.agent_config.as_ref().map(|c| serde_json::to_string(c)).transpose()?,
                watch.interval_secs as i64,
                watch.enabled,
                watch.created_at.timestamp(),
                serde_json::to_string(&watch.headers)?,
                watch.cookie_file,
                watch.storage_state,
                watch.notify_target.as_ref().map(|t| serde_json::to_string(t)).transpose()?,
                serde_json::to_string(&watch.tags)?,
                watch.use_profile,
            ],
        );

        match result {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                Err(crate::KtoError::DuplicateWatchName(watch.name.clone()))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Check if a watch name is already in use (optionally excluding a specific watch ID)
    pub fn name_exists(&self, name: &str, exclude_id: Option<&Uuid>) -> Result<bool> {
        let count: i64 = match exclude_id {
            Some(id) => self.conn.query_row(
                "SELECT COUNT(*) FROM watches WHERE name = ?1 AND id != ?2",
                params![name, id.to_string()],
                |row| row.get(0),
            )?,
            None => self.conn.query_row(
                "SELECT COUNT(*) FROM watches WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )?,
        };
        Ok(count > 0)
    }

    /// Get a watch by ID or name
    pub fn get_watch(&self, id_or_name: &str) -> Result<Option<Watch>> {
        let row = self.conn.query_row(
            "SELECT id, name, url, engine, extraction, normalization, filters, agent_config,
             interval_secs, enabled, created_at, headers, cookie_file, storage_state, notify_target, tags, use_profile
             FROM watches WHERE id = ?1 OR name = ?1",
            params![id_or_name],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, bool>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, bool>(16)?,
                ))
            },
        ).optional()?;

        match row {
            Some((id, name, url, engine, extraction, normalization, filters, agent_config,
                  interval_secs, enabled, created_at, headers, cookie_file, storage_state, notify_target, tags, use_profile)) => {
                Ok(Some(Watch {
                    id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                    name,
                    url,
                    engine: serde_json::from_str(&engine).unwrap_or(Engine::Http),
                    extraction: serde_json::from_str(&extraction).unwrap_or(Extraction::Auto),
                    normalization: serde_json::from_str(&normalization).unwrap_or_default(),
                    filters: serde_json::from_str(&filters).unwrap_or_default(),
                    agent_config: agent_config.and_then(|s| serde_json::from_str(&s).ok()),
                    interval_secs: interval_secs as u64,
                    enabled,
                    created_at: timestamp_to_datetime(created_at),
                    headers: serde_json::from_str(&headers).unwrap_or_default(),
                    cookie_file,
                    storage_state,
                    notify_target: notify_target.and_then(|s| serde_json::from_str(&s).ok()),
                    tags: serde_json::from_str(&tags).unwrap_or_default(),
                    use_profile,
                }))
            }
            None => Ok(None),
        }
    }

    /// List all watches
    pub fn list_watches(&self) -> Result<Vec<Watch>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, url, engine, extraction, normalization, filters, agent_config,
             interval_secs, enabled, created_at, headers, cookie_file, storage_state, notify_target, tags, use_profile
             FROM watches ORDER BY created_at DESC"
        )?;

        let watches = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, bool>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, String>(15)?,
                row.get::<_, bool>(16)?,
            ))
        })?;

        let mut result = Vec::new();
        for row in watches {
            let (id, name, url, engine, extraction, normalization, filters, agent_config,
                 interval_secs, enabled, created_at, headers, cookie_file, storage_state, notify_target, tags, use_profile) = row?;
            result.push(Watch {
                id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                name,
                url,
                engine: serde_json::from_str(&engine).unwrap_or(Engine::Http),
                extraction: serde_json::from_str(&extraction).unwrap_or(Extraction::Auto),
                normalization: serde_json::from_str(&normalization).unwrap_or_default(),
                filters: serde_json::from_str(&filters).unwrap_or_default(),
                agent_config: agent_config.and_then(|s| serde_json::from_str(&s).ok()),
                interval_secs: interval_secs as u64,
                enabled,
                created_at: timestamp_to_datetime(created_at),
                headers: serde_json::from_str(&headers).unwrap_or_default(),
                cookie_file,
                storage_state,
                notify_target: notify_target.and_then(|s| serde_json::from_str(&s).ok()),
                tags: serde_json::from_str(&tags).unwrap_or_default(),
                use_profile,
            });
        }
        Ok(result)
    }

    /// Update a watch
    pub fn update_watch(&self, watch: &Watch) -> Result<()> {
        let result = self.conn.execute(
            "UPDATE watches SET name = ?2, url = ?3, engine = ?4, extraction = ?5,
             normalization = ?6, filters = ?7, agent_config = ?8, interval_secs = ?9,
             enabled = ?10, headers = ?11, cookie_file = ?12, storage_state = ?13, notify_target = ?14,
             tags = ?15, use_profile = ?16
             WHERE id = ?1",
            params![
                watch.id.to_string(),
                watch.name,
                watch.url,
                serde_json::to_string(&watch.engine)?,
                serde_json::to_string(&watch.extraction)?,
                serde_json::to_string(&watch.normalization)?,
                serde_json::to_string(&watch.filters)?,
                watch.agent_config.as_ref().map(|c| serde_json::to_string(c)).transpose()?,
                watch.interval_secs as i64,
                watch.enabled,
                serde_json::to_string(&watch.headers)?,
                watch.cookie_file,
                watch.storage_state,
                watch.notify_target.as_ref().map(|t| serde_json::to_string(t)).transpose()?,
                serde_json::to_string(&watch.tags)?,
                watch.use_profile,
            ],
        );

        match result {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                Err(crate::KtoError::DuplicateWatchName(watch.name.clone()))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Delete a watch
    pub fn delete_watch(&self, id: &Uuid) -> Result<()> {
        self.conn.execute("DELETE FROM watches WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    // ========== Snapshot operations ==========

    /// Insert a new snapshot
    pub fn insert_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        self.conn.execute(
            "INSERT INTO snapshots (id, watch_id, fetched_at, raw_html, extracted, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.id.to_string(),
                snapshot.watch_id.to_string(),
                snapshot.fetched_at.timestamp(),
                snapshot.raw_html,
                snapshot.extracted,
                snapshot.content_hash,
            ],
        )?;
        Ok(())
    }

    /// Get the latest snapshot for a watch
    pub fn get_latest_snapshot(&self, watch_id: &Uuid) -> Result<Option<Snapshot>> {
        let row = self.conn.query_row(
            "SELECT id, watch_id, fetched_at, raw_html, extracted, content_hash
             FROM snapshots WHERE watch_id = ?1 ORDER BY fetched_at DESC LIMIT 1",
            params![watch_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        ).optional()?;

        match row {
            Some((id, watch_id, fetched_at, raw_html, extracted, content_hash)) => {
                Ok(Some(Snapshot {
                    id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                    watch_id: Uuid::parse_str(&watch_id).unwrap_or_else(|_| Uuid::new_v4()),
                    fetched_at: timestamp_to_datetime(fetched_at),
                    raw_html,
                    extracted,
                    content_hash,
                }))
            }
            None => Ok(None),
        }
    }

    /// Get the most recent snapshot across all watches (for daemon health check)
    pub fn get_most_recent_snapshot(&self) -> Result<Option<Snapshot>> {
        let row = self.conn.query_row(
            "SELECT id, watch_id, fetched_at, raw_html, extracted, content_hash
             FROM snapshots ORDER BY fetched_at DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        ).optional()?;

        match row {
            Some((id, watch_id, fetched_at, raw_html, extracted, content_hash)) => {
                Ok(Some(Snapshot {
                    id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                    watch_id: Uuid::parse_str(&watch_id).unwrap_or_else(|_| Uuid::new_v4()),
                    fetched_at: timestamp_to_datetime(fetched_at),
                    raw_html,
                    extracted,
                    content_hash,
                }))
            }
            None => Ok(None),
        }
    }

    /// Clean up old snapshots (keep last N extracted, last M with raw_html)
    pub fn cleanup_snapshots(&self, watch_id: &Uuid, keep_extracted: usize, keep_raw: usize) -> Result<()> {
        // Remove raw_html from older snapshots
        self.conn.execute(
            "UPDATE snapshots SET raw_html = NULL
             WHERE watch_id = ?1 AND id NOT IN (
                 SELECT id FROM snapshots WHERE watch_id = ?1 AND raw_html IS NOT NULL
                 ORDER BY fetched_at DESC LIMIT ?2
             )",
            params![watch_id.to_string(), keep_raw as i64],
        )?;

        // First, delete changes that reference snapshots we're about to delete
        // This avoids foreign key constraint violations
        self.conn.execute(
            "DELETE FROM changes
             WHERE watch_id = ?1 AND (
                 old_snapshot_id NOT IN (
                     SELECT id FROM snapshots WHERE watch_id = ?1
                     ORDER BY fetched_at DESC LIMIT ?2
                 ) OR new_snapshot_id NOT IN (
                     SELECT id FROM snapshots WHERE watch_id = ?1
                     ORDER BY fetched_at DESC LIMIT ?2
                 )
             )",
            params![watch_id.to_string(), keep_extracted as i64],
        )?;

        // Delete very old snapshots
        self.conn.execute(
            "DELETE FROM snapshots
             WHERE watch_id = ?1 AND id NOT IN (
                 SELECT id FROM snapshots WHERE watch_id = ?1
                 ORDER BY fetched_at DESC LIMIT ?2
             )",
            params![watch_id.to_string(), keep_extracted as i64],
        )?;

        Ok(())
    }

    // ========== Change operations ==========

    /// Insert a new change record
    pub fn insert_change(&self, change: &Change) -> Result<()> {
        self.conn.execute(
            "INSERT INTO changes (id, watch_id, detected_at, old_snapshot_id, new_snapshot_id,
             diff, filter_passed, agent_response, notified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                change.id.to_string(),
                change.watch_id.to_string(),
                change.detected_at.timestamp(),
                change.old_snapshot_id.to_string(),
                change.new_snapshot_id.to_string(),
                change.diff,
                change.filter_passed,
                change.agent_response.as_ref().map(|r| serde_json::to_string(r).ok()).flatten(),
                change.notified,
            ],
        )?;
        Ok(())
    }

    /// Get recent changes for a watch
    pub fn get_recent_changes(&self, watch_id: &Uuid, limit: usize) -> Result<Vec<Change>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, watch_id, detected_at, old_snapshot_id, new_snapshot_id,
             diff, filter_passed, agent_response, notified
             FROM changes WHERE watch_id = ?1 ORDER BY detected_at DESC LIMIT ?2"
        )?;

        let changes = stmt.query_map(params![watch_id.to_string(), limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, bool>(8)?,
            ))
        })?;

        let mut result = Vec::new();
        for row in changes {
            let (id, watch_id, detected_at, old_id, new_id, diff, filter_passed, agent_resp, notified) = row?;
            result.push(Change {
                id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                watch_id: Uuid::parse_str(&watch_id).unwrap_or_else(|_| Uuid::new_v4()),
                detected_at: timestamp_to_datetime(detected_at),
                old_snapshot_id: Uuid::parse_str(&old_id).unwrap_or_else(|_| Uuid::new_v4()),
                new_snapshot_id: Uuid::parse_str(&new_id).unwrap_or_else(|_| Uuid::new_v4()),
                diff,
                filter_passed,
                agent_response: agent_resp.and_then(|s| serde_json::from_str(&s).ok()),
                notified,
            });
        }
        Ok(result)
    }

    /// Get all recent changes across all watches (for logs command)
    pub fn get_all_recent_changes(&self, limit: usize) -> Result<Vec<(Change, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.watch_id, c.detected_at, c.old_snapshot_id, c.new_snapshot_id,
             c.diff, c.filter_passed, c.agent_response, c.notified, w.name
             FROM changes c
             JOIN watches w ON c.watch_id = w.id
             ORDER BY c.detected_at DESC LIMIT ?1"
        )?;

        let changes = stmt.query_map(params![limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, bool>(8)?,
                row.get::<_, String>(9)?,
            ))
        })?;

        let mut result = Vec::new();
        for row in changes {
            let (id, watch_id, detected_at, old_id, new_id, diff, filter_passed, agent_resp, notified, watch_name) = row?;
            let change = Change {
                id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                watch_id: Uuid::parse_str(&watch_id).unwrap_or_else(|_| Uuid::new_v4()),
                detected_at: timestamp_to_datetime(detected_at),
                old_snapshot_id: Uuid::parse_str(&old_id).unwrap_or_else(|_| Uuid::new_v4()),
                new_snapshot_id: Uuid::parse_str(&new_id).unwrap_or_else(|_| Uuid::new_v4()),
                diff,
                filter_passed,
                agent_response: agent_resp.and_then(|s| serde_json::from_str(&s).ok()),
                notified,
            };
            result.push((change, watch_name));
        }
        Ok(result)
    }

    /// Mark a change as notified
    pub fn mark_notified(&self, change_id: &Uuid) -> Result<()> {
        self.conn.execute(
            "UPDATE changes SET notified = 1 WHERE id = ?1",
            params![change_id.to_string()],
        )?;
        Ok(())
    }

    // ========== Agent memory operations ==========

    /// Get agent memory for a watch
    pub fn get_agent_memory(&self, watch_id: &Uuid) -> Result<AgentMemory> {
        let row = self.conn.query_row(
            "SELECT memory FROM agent_memory WHERE watch_id = ?1",
            params![watch_id.to_string()],
            |row| row.get::<_, String>(0),
        ).optional()?;

        match row {
            Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
            None => Ok(AgentMemory::default()),
        }
    }

    /// Update agent memory for a watch
    pub fn update_agent_memory(&self, watch_id: &Uuid, memory: &AgentMemory) -> Result<()> {
        let json = serde_json::to_string(memory)?;
        let now = Utc::now().timestamp();

        self.conn.execute(
            "INSERT INTO agent_memory (watch_id, memory, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(watch_id) DO UPDATE SET memory = ?2, updated_at = ?3",
            params![watch_id.to_string(), json, now],
        )?;
        Ok(())
    }

    /// Clear agent memory for a watch
    pub fn clear_agent_memory(&self, watch_id: &Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_memory WHERE watch_id = ?1",
            params![watch_id.to_string()],
        )?;
        Ok(())
    }

    // ========== Reminder operations ==========

    /// Insert a new reminder
    pub fn insert_reminder(&self, reminder: &Reminder) -> Result<()> {
        let notify_json = reminder.notify_target.as_ref()
            .map(|t| serde_json::to_string(t).unwrap_or_default());

        self.conn.execute(
            "INSERT INTO reminders (id, name, message, trigger_at, interval_secs, enabled, notify_target, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                reminder.id.to_string(),
                reminder.name,
                reminder.message,
                reminder.trigger_at.timestamp(),
                reminder.interval_secs.map(|s| s as i64),
                reminder.enabled,
                notify_json,
                reminder.created_at.timestamp(),
            ],
        )?;
        Ok(())
    }

    /// List all reminders
    pub fn list_reminders(&self) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, message, trigger_at, interval_secs, enabled, notify_target, created_at
             FROM reminders ORDER BY trigger_at ASC"
        )?;

        let reminders = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;

        let mut result = Vec::new();
        for row in reminders {
            let (id, name, message, trigger_at, interval_secs, enabled, notify_json, created_at) = row?;
            result.push(Reminder {
                id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                name,
                message,
                trigger_at: timestamp_to_datetime(trigger_at),
                interval_secs: interval_secs.map(|s| s as u64),
                enabled,
                notify_target: notify_json.and_then(|j| serde_json::from_str(&j).ok()),
                created_at: timestamp_to_datetime(created_at),
            });
        }
        Ok(result)
    }

    /// Get a specific reminder by ID or name
    pub fn get_reminder(&self, id_or_name: &str) -> Result<Option<Reminder>> {
        let row = self.conn.query_row(
            "SELECT id, name, message, trigger_at, interval_secs, enabled, notify_target, created_at
             FROM reminders WHERE id = ?1 OR name = ?1",
            params![id_or_name],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        ).optional()?;

        Ok(row.map(|(id, name, message, trigger_at, interval_secs, enabled, notify_json, created_at)| {
            Reminder {
                id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                name,
                message,
                trigger_at: timestamp_to_datetime(trigger_at),
                interval_secs: interval_secs.map(|s| s as u64),
                enabled,
                notify_target: notify_json.and_then(|j| serde_json::from_str(&j).ok()),
                created_at: timestamp_to_datetime(created_at),
            }
        }))
    }

    /// Get reminders that are due (trigger_at <= now and enabled)
    pub fn get_due_reminders(&self) -> Result<Vec<Reminder>> {
        let now = Utc::now().timestamp();
        let mut stmt = self.conn.prepare(
            "SELECT id, name, message, trigger_at, interval_secs, enabled, notify_target, created_at
             FROM reminders WHERE trigger_at <= ?1 AND enabled = 1"
        )?;

        let reminders = stmt.query_map(params![now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;

        let mut result = Vec::new();
        for row in reminders {
            let (id, name, message, trigger_at, interval_secs, enabled, notify_json, created_at) = row?;
            result.push(Reminder {
                id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                name,
                message,
                trigger_at: timestamp_to_datetime(trigger_at),
                interval_secs: interval_secs.map(|s| s as u64),
                enabled,
                notify_target: notify_json.and_then(|j| serde_json::from_str(&j).ok()),
                created_at: timestamp_to_datetime(created_at),
            });
        }
        Ok(result)
    }

    /// Update reminder trigger time (for recurring reminders)
    pub fn update_reminder_trigger(&self, id: &Uuid, new_trigger: DateTime<Utc>) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET trigger_at = ?1 WHERE id = ?2",
            params![new_trigger.timestamp(), id.to_string()],
        )?;
        Ok(())
    }

    /// Update reminder enabled status
    pub fn set_reminder_enabled(&self, id: &Uuid, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET enabled = ?1 WHERE id = ?2",
            params![enabled, id.to_string()],
        )?;
        Ok(())
    }

    /// Update a reminder (all fields)
    pub fn update_reminder(&self, reminder: &Reminder) -> Result<()> {
        let notify_target = reminder.notify_target.as_ref().map(|t| {
            serde_json::to_string(t).unwrap_or_default()
        });
        self.conn.execute(
            "UPDATE reminders SET name = ?1, message = ?2, trigger_at = ?3, interval_secs = ?4, enabled = ?5, notify_target = ?6 WHERE id = ?7",
            params![
                reminder.name,
                reminder.message,
                reminder.trigger_at.timestamp(),
                reminder.interval_secs.map(|s| s as i64),
                reminder.enabled,
                notify_target,
                reminder.id.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Delete a reminder
    pub fn delete_reminder(&self, id: &Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM reminders WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    // ========== Global memory operations ==========

    /// Get the global memory (cross-watch learning)
    pub fn get_global_memory(&self) -> Result<GlobalMemory> {
        let row = self.conn.query_row(
            "SELECT memory_json FROM global_memory ORDER BY updated_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        ).optional()?;

        match row {
            Some(json) => GlobalMemory::from_json(&json),
            None => Ok(GlobalMemory::default()),
        }
    }

    /// Update the global memory
    pub fn update_global_memory(&self, memory: &GlobalMemory) -> Result<()> {
        let json = memory.to_json()?;
        let now = Utc::now().timestamp();

        // Use upsert pattern - there's only ever one row in global_memory
        self.conn.execute(
            "INSERT INTO global_memory (id, memory_json, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET memory_json = ?1, updated_at = ?2",
            params![json, now],
        )?;
        Ok(())
    }

    /// Clear the global memory
    pub fn clear_global_memory(&self) -> Result<()> {
        self.conn.execute("DELETE FROM global_memory", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watch_crud() {
        let db = Database::open_in_memory().unwrap();

        let watch = Watch::new("Test Watch".to_string(), "https://example.com".to_string());
        db.insert_watch(&watch).unwrap();

        let loaded = db.get_watch(&watch.id.to_string()).unwrap().unwrap();
        assert_eq!(loaded.name, "Test Watch");
        assert_eq!(loaded.url, "https://example.com");

        let watches = db.list_watches().unwrap();
        assert_eq!(watches.len(), 1);

        db.delete_watch(&watch.id).unwrap();
        let deleted = db.get_watch(&watch.id.to_string()).unwrap();
        assert!(deleted.is_none());
    }

    #[test]
    fn test_snapshot_operations() {
        let db = Database::open_in_memory().unwrap();

        let watch = Watch::new("Test".to_string(), "https://example.com".to_string());
        db.insert_watch(&watch).unwrap();

        let snapshot = Snapshot {
            id: Uuid::new_v4(),
            watch_id: watch.id,
            fetched_at: Utc::now(),
            raw_html: Some(vec![1, 2, 3]),
            extracted: "Test content".to_string(),
            content_hash: "abc123".to_string(),
        };
        db.insert_snapshot(&snapshot).unwrap();

        let latest = db.get_latest_snapshot(&watch.id).unwrap().unwrap();
        assert_eq!(latest.extracted, "Test content");
    }

    #[test]
    fn test_agent_memory() {
        let db = Database::open_in_memory().unwrap();

        let watch = Watch::new("Test".to_string(), "https://example.com".to_string());
        db.insert_watch(&watch).unwrap();

        let mut memory = AgentMemory::default();
        memory.counters.insert("price_drops".to_string(), 3);
        memory.last_values.insert("price".to_string(), serde_json::json!("$99.99"));

        db.update_agent_memory(&watch.id, &memory).unwrap();

        let loaded = db.get_agent_memory(&watch.id).unwrap();
        assert_eq!(loaded.counters.get("price_drops"), Some(&3));
        assert_eq!(loaded.last_values.get("price"), Some(&serde_json::json!("$99.99")));
    }
}
