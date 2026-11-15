//! Local curation library (SQLite).
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::model::{SearchQuery, Status, Wallpaper};

/// User-side metadata about a wallpaper.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub status: Status,
    pub rating: u8,
    pub notes: String,
    pub tags: Vec<String>,
    pub local_path: Option<PathBuf>,
    pub added_at: i64,
}

pub struct Library {
    conn: Connection,
}

const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE wallpapers (
    key        TEXT PRIMARY KEY,
    provider   TEXT NOT NULL,
    remote_id  TEXT NOT NULL,
    data       TEXT NOT NULL,
    status     INTEGER NOT NULL DEFAULT 0,
    rating     INTEGER NOT NULL DEFAULT 0,
    notes      TEXT NOT NULL DEFAULT '',
    local_path TEXT,
    added_at   INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX idx_wallpapers_status ON wallpapers(status);
CREATE TABLE tags (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);
CREATE TABLE wallpaper_tags (
    key    TEXT NOT NULL REFERENCES wallpapers(key) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (key, tag_id)
);
CREATE TABLE history (
    id     INTEGER PRIMARY KEY,
    key    TEXT NOT NULL,
    set_at INTEGER NOT NULL
);
"#,
    r#"
CREATE INDEX idx_wallpapers_provider ON wallpapers(provider);
CREATE INDEX idx_history_key ON history(key);
CREATE TABLE saved_searches (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    provider   TEXT NOT NULL,
    query      TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
"#,
    r#"
CREATE TABLE hashes (
    key  TEXT PRIMARY KEY REFERENCES wallpapers(key) ON DELETE CASCADE,
    hash INTEGER NOT NULL
);
"#,
];

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Sort orders for library listings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LibrarySort {
    #[default]
    Updated,
    Added,
    Rating,
    Resolution,
}

impl LibrarySort {
    pub const ALL: [LibrarySort; 4] = [Self::Updated, Self::Added, Self::Rating, Self::Resolution];
    pub fn label(self) -> &'static str {
        match self {
            Self::Updated => "Recently changed",
            Self::Added => "Recently added",
            Self::Rating => "Rating",
            Self::Resolution => "Resolution",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OrientationFilter {
    #[default]
    Any,
    Landscape,
    Portrait,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibraryFilter {
    /// `None` = everything except rejected.
    pub status: Option<Status>,
    pub text: String,
    pub tag: Option<String>,
    pub provider: Option<String>,
    pub orientation: OrientationFilter,
    pub min_rating: u8,
    pub sort: LibrarySort,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedSearch {
    pub id: i64,
    pub name: String,
    pub provider: String,
    pub query: SearchQuery,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub total: u32,
    pub on_disk: u32,
    pub bytes: u64,
    pub missing_files: u32,
}

#[derive(Serialize, Deserialize)]
struct ExportItem {
    wallpaper: Wallpaper,
    status: Status,
    rating: u8,
    notes: String,
    tags: Vec<String>,
    local_path: Option<PathBuf>,
    added_at: i64,
}

#[derive(Serialize, Deserialize)]
struct ExportFile {
    version: u32,
    exported_at: i64,
    wallpapers: Vec<ExportItem>,
}

impl Library {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Safe with WAL (a power cut can lose the last commits, never corrupt) and avoids
        // an fsync on the UI thread for every rating and status change.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut lib = Self { conn };
        lib.migrate()?;
        Ok(lib)
    }

    /// Start a transaction for many writes, which is much faster than one commit each.
    /// Returns false if none could be started (the writes then simply commit one by one).
    pub fn begin(&self) -> bool {
        self.conn.execute_batch("BEGIN").is_ok()
    }

    /// End `begin`. Failures inside are reported by the individual writes; what succeeded stays.
    pub fn commit(&self) {
        if let Err(e) = self.conn.execute_batch("COMMIT") {
            tracing::warn!("library commit failed: {e}");
        }
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut lib = Self { conn };
        lib.migrate()?;
        Ok(lib)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let tx = self.conn.transaction()?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert or refresh the provider data for a wallpaper, keeping user metadata.
    /// Provider tags already stored are kept when the new data has none
    /// (search results usually lack tags; details add them).
    pub fn upsert(&self, wp: &Wallpaper) -> Result<()> {
        let data = serde_json::to_string(wp)?;
        self.conn.execute(
            "INSERT INTO wallpapers (key, provider, remote_id, data, added_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(key) DO UPDATE SET data =
                CASE WHEN json_array_length(excluded.data, '$.tags') = 0
                     THEN json_set(excluded.data, '$.tags', json(wallpapers.data -> '$.tags'))
                     ELSE excluded.data END",
            params![wp.key(), wp.provider, wp.remote_id, data, now()],
        )?;
        Ok(())
    }

    pub fn wallpaper(&self, key: &str) -> Result<Option<Wallpaper>> {
        let d: Option<String> =
            self.conn.query_row("SELECT data FROM wallpapers WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        Ok(d.and_then(|d| serde_json::from_str(&d).ok()))
    }

    pub fn entry(&self, key: &str) -> Result<Option<Entry>> {
        let row = self
            .conn
            .query_row(
                "SELECT status, rating, notes, local_path, added_at FROM wallpapers WHERE key = ?1",
                [key],
                |r| {
                    Ok(Entry {
                        status: Status::from_i64(r.get(0)?),
                        rating: r.get::<_, i64>(1)? as u8,
                        notes: r.get(2)?,
                        local_path: r.get::<_, Option<String>>(3)?.map(PathBuf::from),
                        added_at: r.get(4)?,
                        tags: vec![],
                    })
                },
            )
            .optional()?;
        Ok(match row {
            Some(mut e) => {
                e.tags = self.tags_of(key)?;
                Some(e)
            }
            None => None,
        })
    }

    pub fn tags_of(&self, key: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare_cached(
            "SELECT t.name FROM tags t JOIN wallpaper_tags wt ON wt.tag_id = t.id
             WHERE wt.key = ?1 ORDER BY t.name",
        )?;
        let v = st.query_map([key], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    fn touch(&self, key: &str) -> Result<()> {
        self.conn.execute("UPDATE wallpapers SET updated_at = ?2 WHERE key = ?1", params![key, now()])?;
        Ok(())
    }

    pub fn set_status(&self, wp: &Wallpaper, status: Status) -> Result<()> {
        self.upsert(wp)?;
        self.conn.execute(
            "UPDATE wallpapers SET status = ?2 WHERE key = ?1",
            params![wp.key(), status.to_i64()],
        )?;
        self.touch(&wp.key())
    }

    pub fn set_rating(&self, wp: &Wallpaper, rating: u8) -> Result<()> {
        self.upsert(wp)?;
        self.conn.execute(
            "UPDATE wallpapers SET rating = ?2 WHERE key = ?1",
            params![wp.key(), rating.min(5) as i64],
        )?;
        self.touch(&wp.key())
    }

    pub fn set_notes(&self, wp: &Wallpaper, notes: &str) -> Result<()> {
        self.upsert(wp)?;
        self.conn.execute("UPDATE wallpapers SET notes = ?2 WHERE key = ?1", params![wp.key(), notes])?;
        self.touch(&wp.key())
    }

    pub fn set_local_path(&self, wp: &Wallpaper, path: Option<&Path>) -> Result<()> {
        self.upsert(wp)?;
        self.conn.execute(
            "UPDATE wallpapers SET local_path = ?2 WHERE key = ?1",
            params![wp.key(), path.map(|p| p.to_string_lossy().to_string())],
        )?;
        Ok(())
    }

    pub fn add_tag(&self, wp: &Wallpaper, tag: &str) -> Result<()> {
        let tag = tag.trim();
        if tag.is_empty() {
            return Ok(());
        }
        self.upsert(wp)?;
        self.conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [tag])?;
        self.conn.execute(
            "INSERT OR IGNORE INTO wallpaper_tags (key, tag_id)
             SELECT ?1, id FROM tags WHERE name = ?2",
            params![wp.key(), tag],
        )?;
        self.touch(&wp.key())
    }

    pub fn remove_tag(&self, key: &str, tag: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM wallpaper_tags WHERE key = ?1 AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
            params![key, tag],
        )?;
        self.touch(key)
    }

    /// Delete everything the library knows about a wallpaper (tags, rating, history).
    pub fn forget(&self, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM history WHERE key = ?1", [key])?;
        self.conn.execute("DELETE FROM wallpapers WHERE key = ?1", [key])?;
        Ok(())
    }

    /// All tags with usage counts, most used first.
    pub fn all_tags(&self) -> Result<Vec<(String, u32)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT t.name, COUNT(wt.key) c FROM tags t LEFT JOIN wallpaper_tags wt ON wt.tag_id = t.id
             GROUP BY t.id HAVING c > 0 ORDER BY c DESC, t.name",
        )?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u32)))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Providers present in the library with counts.
    pub fn providers(&self) -> Result<Vec<(String, u32)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT provider, COUNT(*) FROM wallpapers WHERE status != 3 GROUP BY provider ORDER BY 2 DESC",
        )?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u32)))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn record_set(&self, wp: &Wallpaper) -> Result<()> {
        self.upsert(wp)?;
        self.conn.execute("INSERT INTO history (key, set_at) VALUES (?1, ?2)", params![wp.key(), now()])?;
        self.touch(&wp.key())
    }

    pub fn history(&self, limit: u32) -> Result<Vec<(Wallpaper, i64)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT w.data, h.set_at FROM history h JOIN wallpapers w ON w.key = h.key
             ORDER BY h.set_at DESC, h.id DESC LIMIT ?1",
        )?;
        let rows = st
            .query_map([limit], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .filter_map(|(d, t)| serde_json::from_str(&d).ok().map(|w| (w, t)))
            .collect();
        Ok(rows)
    }

    pub fn clear_history(&self) -> Result<()> {
        self.conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    pub fn list(&self, f: &LibraryFilter) -> Result<Vec<Wallpaper>> {
        let mut sql = String::from("SELECT w.data FROM wallpapers w WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![];
        match f.status {
            Some(s) => {
                sql += " AND w.status = ?";
                args.push(Box::new(s.to_i64()));
            }
            None => sql += " AND w.status != 3",
        }
        if f.min_rating > 0 {
            sql += " AND w.rating >= ?";
            args.push(Box::new(f.min_rating as i64));
        }
        if let Some(p) = &f.provider {
            sql += " AND w.provider = ?";
            args.push(Box::new(p.clone()));
        }
        match f.orientation {
            OrientationFilter::Any => {}
            OrientationFilter::Landscape => {
                sql += " AND json_extract(w.data, '$.width') >= json_extract(w.data, '$.height')"
            }
            OrientationFilter::Portrait => {
                sql += " AND json_extract(w.data, '$.width') < json_extract(w.data, '$.height')"
            }
        }
        for word in f.text.split_whitespace() {
            // Search the values, not the raw JSON (every row contains the word "title").
            sql += " AND ((w.remote_id
                        || ' ' || COALESCE(json_extract(w.data, '$.title'), '')
                        || ' ' || COALESCE(json_extract(w.data, '$.author'), '')
                        || ' ' || COALESCE(json_extract(w.data, '$.category'), '')
                        || ' ' || COALESCE(json_extract(w.data, '$.source'), '')
                        || ' ' || COALESCE(json_extract(w.data, '$.tags'), '')
                        || ' ' || w.notes) LIKE ? ESCAPE '\\'
                     OR EXISTS (SELECT 1 FROM wallpaper_tags wt JOIN tags t ON t.id = wt.tag_id
                                WHERE wt.key = w.key AND t.name LIKE ? ESCAPE '\\'))";
            let escaped = word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            let pat = format!("%{escaped}%");
            for _ in 0..2 {
                args.push(Box::new(pat.clone()));
            }
        }
        if let Some(tag) = &f.tag {
            sql += " AND EXISTS (SELECT 1 FROM wallpaper_tags wt JOIN tags t ON t.id = wt.tag_id
                     WHERE wt.key = w.key AND t.name = ?)";
            args.push(Box::new(tag.clone()));
        }
        sql += match f.sort {
            LibrarySort::Updated => " ORDER BY w.updated_at DESC",
            LibrarySort::Added => " ORDER BY w.added_at DESC",
            LibrarySort::Rating => " ORDER BY w.rating DESC, w.updated_at DESC",
            LibrarySort::Resolution => {
                " ORDER BY json_extract(w.data, '$.width') * json_extract(w.data, '$.height') DESC"
            }
        };
        let mut st = self.conn.prepare(&sql)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())), |r| {
                r.get::<_, String>(0)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|d| serde_json::from_str(&d).ok())
            .collect();
        Ok(rows)
    }

    /// Number of wallpapers per status: [none, considering, kept, rejected].
    pub fn counts(&self) -> Result<[u32; 4]> {
        let mut out = [0u32; 4];
        let mut st = self.conn.prepare_cached("SELECT status, COUNT(*) FROM wallpapers GROUP BY status")?;
        for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (s, c) = row?;
            if (0..4).contains(&s) {
                out[s as usize] = c as u32;
            }
        }
        Ok(out)
    }

    /// Keys of everything the user rejected (to hide in browse results).
    pub fn rejected_keys(&self) -> Result<std::collections::HashSet<String>> {
        let mut st = self.conn.prepare("SELECT key FROM wallpapers WHERE status = 3")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Size/presence of downloaded files for kept wallpapers.
    pub fn stats(&self) -> Result<Stats> {
        let mut st = self.conn.prepare_cached("SELECT local_path FROM wallpapers WHERE status = 2")?;
        let mut s = Stats::default();
        for p in st.query_map([], |r| r.get::<_, Option<String>>(0))? {
            s.total += 1;
            if let Some(p) = p? {
                match std::fs::metadata(&p) {
                    Ok(m) => {
                        s.on_disk += 1;
                        s.bytes += m.len();
                    }
                    Err(_) => s.missing_files += 1,
                }
            }
        }
        Ok(s)
    }

    /// Downloaded files of wallpapers that are not Kept (local-folder originals excluded).
    pub fn downloads_not_kept(&self) -> Result<Vec<(String, PathBuf)>> {
        let mut st = self.conn.prepare(
            "SELECT key, local_path FROM wallpapers WHERE status != 2 AND local_path IS NOT NULL AND provider != 'local'",
        )?;
        let v = st.query_map([], |r| Ok((r.get::<_, String>(0)?, PathBuf::from(r.get::<_, String>(1)?))))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn clear_local_path(&self, key: &str) -> Result<()> {
        self.conn.execute("UPDATE wallpapers SET local_path = NULL WHERE key = ?1", [key])?;
        Ok(())
    }

    /// Store the perceptual hash of a wallpaper that is in the library (no-op otherwise).
    pub fn set_hash(&self, key: &str, hash: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO hashes (key, hash) SELECT key, ?2 FROM wallpapers WHERE key = ?1
             ON CONFLICT(key) DO UPDATE SET hash = excluded.hash",
            params![key, hash as i64],
        )?;
        Ok(())
    }

    /// (key, hash, status) for every hashed library wallpaper.
    pub fn hashes(&self) -> Result<Vec<(String, u64, Status)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT h.key, h.hash, w.status FROM hashes h JOIN wallpapers w ON w.key = h.key",
        )?;
        let v = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, Status::from_i64(r.get(2)?))))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Library key of the wallpaper stored at `path` (to recognise the current desktop wallpaper).
    pub fn key_for_path(&self, path: &Path) -> Result<Option<String>> {
        let p = path.to_string_lossy().to_string();
        let k = self
            .conn
            .query_row("SELECT key FROM wallpapers WHERE local_path = ?1 COLLATE NOCASE LIMIT 1", [p], |r| r.get(0))
            .optional()?;
        Ok(k)
    }

    /// Kept wallpapers that have no file on disk (to (re)download).
    pub fn kept_without_file(&self) -> Result<Vec<Wallpaper>> {
        let mut st = self.conn.prepare("SELECT data, local_path FROM wallpapers WHERE status = 2")?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?
            .filter_map(|r| r.ok())
            .filter(|(_, p)| p.as_ref().is_none_or(|p| !Path::new(p).exists()))
            .filter_map(|(d, _)| serde_json::from_str(&d).ok())
            .collect();
        Ok(rows)
    }

    // ----- saved searches -----

    pub fn save_search(&self, name: &str, provider: &str, query: &SearchQuery) -> Result<()> {
        self.conn.execute(
            "INSERT INTO saved_searches (name, provider, query, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![name, provider, serde_json::to_string(query)?, now()],
        )?;
        Ok(())
    }

    pub fn saved_searches(&self) -> Result<Vec<SavedSearch>> {
        let mut st = self.conn.prepare_cached("SELECT id, name, provider, query FROM saved_searches ORDER BY name")?;
        let v = st
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id, name, provider, q)| {
                serde_json::from_str(&q).ok().map(|query| SavedSearch { id, name, provider, query })
            })
            .collect();
        Ok(v)
    }

    pub fn delete_search(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM saved_searches WHERE id = ?1", [id])?;
        Ok(())
    }

    // ----- export / import -----

    pub fn export_json(&self, path: &Path) -> Result<usize> {
        let mut st = self
            .conn
            .prepare("SELECT key, data, status, rating, notes, local_path, added_at FROM wallpapers ORDER BY added_at")?;
        let rows: Vec<(String, String, i64, i64, String, Option<String>, i64)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
            .collect::<Result<_, _>>()?;
        let mut items = vec![];
        for (key, data, status, rating, notes, local_path, added_at) in rows {
            let Ok(wallpaper) = serde_json::from_str(&data) else { continue };
            items.push(ExportItem {
                wallpaper,
                status: Status::from_i64(status),
                rating: rating as u8,
                notes,
                tags: self.tags_of(&key)?,
                local_path: local_path.map(PathBuf::from),
                added_at,
            });
        }
        let n = items.len();
        let file = ExportFile { version: 1, exported_at: now(), wallpapers: items };
        std::fs::write(path, serde_json::to_vec_pretty(&file)?).with_context(|| format!("writing {}", path.display()))?;
        Ok(n)
    }

    /// Merge an export into this library. Existing entries get the imported status/rating/notes
    /// only where the local value is empty; tags are unioned.
    pub fn import_json(&mut self, path: &Path) -> Result<usize> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let file: ExportFile = serde_json::from_slice(&bytes).context("not a walle export file")?;
        self.conn.execute_batch("BEGIN")?;
        let result = self.import_items(&file.wallpapers);
        self.conn.execute_batch(if result.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
        result.map(|_| file.wallpapers.len())
    }

    fn import_items(&self, items: &[ExportItem]) -> Result<()> {
        {
            for it in items {
                let key = it.wallpaper.key();
                self.upsert(&it.wallpaper)?;
                self.conn.execute(
                    "UPDATE wallpapers SET
                        status = CASE WHEN status = 0 THEN ?2 ELSE status END,
                        rating = CASE WHEN rating = 0 THEN ?3 ELSE rating END,
                        notes = CASE WHEN notes = '' THEN ?4 ELSE notes END,
                        local_path = COALESCE(local_path, ?5),
                        added_at = MIN(added_at, ?6)
                     WHERE key = ?1",
                    params![
                        key,
                        it.status.to_i64(),
                        it.rating as i64,
                        it.notes,
                        it.local_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                        it.added_at
                    ],
                )?;
                for t in &it.tags {
                    self.add_tag(&it.wallpaper, t)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp(id: &str) -> Wallpaper {
        Wallpaper { provider: "test".into(), remote_id: id.into(), width: 1920, height: 1080, ..Default::default() }
    }

    #[test]
    fn curate_roundtrip() -> Result<()> {
        let lib = Library::in_memory()?;
        let a = wp("a");
        assert!(lib.entry(&a.key())?.is_none());
        lib.set_status(&a, Status::Considering)?;
        lib.set_rating(&a, 4)?;
        lib.add_tag(&a, "Mountains")?;
        lib.add_tag(&a, "mountains")?; // case-insensitive duplicate
        lib.add_tag(&a, "blue")?;
        let e = lib.entry(&a.key())?.unwrap();
        assert_eq!(e.status, Status::Considering);
        assert_eq!(e.rating, 4);
        assert_eq!(e.tags, vec!["blue", "Mountains"]);

        lib.remove_tag(&a.key(), "blue")?;
        assert_eq!(lib.tags_of(&a.key())?, vec!["Mountains"]);

        let b = wp("b");
        lib.set_status(&b, Status::Rejected)?;
        assert_eq!(lib.rejected_keys()?.len(), 1);

        let all = lib.list(&LibraryFilter::default())?;
        assert_eq!(all.len(), 1, "rejected hidden by default");
        let tagged = lib.list(&LibraryFilter { tag: Some("mountains".into()), ..Default::default() })?;
        assert_eq!(tagged.len(), 1);
        let by_text = lib.list(&LibraryFilter { text: "mount".into(), ..Default::default() })?;
        assert_eq!(by_text.len(), 1, "text search matches local tags");
        let portrait = lib.list(&LibraryFilter { orientation: OrientationFilter::Portrait, ..Default::default() })?;
        assert!(portrait.is_empty());

        lib.record_set(&a)?;
        assert_eq!(lib.history(10)?.len(), 1);

        lib.forget(&a.key())?;
        assert!(lib.entry(&a.key())?.is_none());
        assert!(lib.history(10)?.is_empty());
        Ok(())
    }

    #[test]
    fn text_search_matches_values_only() -> Result<()> {
        let lib = Library::in_memory()?;
        let mut a = wp("a");
        a.title = Some("Mountain 100% lake_view".into());
        lib.set_status(&a, Status::Kept)?;
        let mut b = wp("b");
        b.author = Some("Ada".into());
        b.tags = vec!["forest".into()];
        lib.set_status(&b, Status::Kept)?;
        let find = |t: &str| -> Result<Vec<String>> {
            Ok(lib.list(&LibraryFilter { text: t.into(), ..Default::default() })?.iter().map(|w| w.remote_id.clone()).collect())
        };
        assert!(find("title")?.is_empty(), "JSON field names are not content");
        assert!(find("https")?.is_empty());
        assert_eq!(find("mountain")?, ["a"]);
        assert_eq!(find("ada")?, ["b"]);
        assert_eq!(find("forest")?, ["b"]);
        assert_eq!(find("100%")?, ["a"]);
        assert!(find("1%0")?.is_empty(), "% is literal");
        assert_eq!(find("lake_view")?, ["a"]);
        assert!(find("lake_vie_")?.is_empty(), "_ is literal");
        Ok(())
    }

    #[test]
    fn batch_commits_everything() -> Result<()> {
        let lib = Library::in_memory()?;
        assert!(lib.begin());
        for id in ["a", "b", "c"] {
            lib.set_status(&wp(id), Status::Kept)?;
        }
        lib.commit();
        assert_eq!(lib.counts()?[2], 3);
        Ok(())
    }

    #[test]
    fn upsert_keeps_detail_tags() -> Result<()> {
        let lib = Library::in_memory()?;
        let mut a = wp("a");
        a.tags = vec!["sky".into()];
        lib.upsert(&a)?;
        a.tags.clear();
        a.title = Some("new".into());
        lib.upsert(&a)?;
        let stored = lib.wallpaper(&a.key())?.unwrap();
        assert_eq!(stored.tags, vec!["sky"]);
        assert_eq!(stored.title.as_deref(), Some("new"));
        Ok(())
    }

    #[test]
    fn saved_searches_and_export() -> Result<()> {
        let mut lib = Library::in_memory()?;
        lib.save_search("calm", "wallhaven", &SearchQuery::default())?;
        assert_eq!(lib.saved_searches()?.len(), 1);

        let a = wp("a");
        lib.set_status(&a, Status::Kept)?;
        lib.add_tag(&a, "fav")?;
        let dir = std::env::temp_dir().join(format!("walle-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let f = dir.join("export.json");
        assert_eq!(lib.export_json(&f)?, 1);

        let mut other = Library::in_memory()?;
        assert_eq!(other.import_json(&f)?, 1);
        let e = other.entry(&a.key())?.unwrap();
        assert_eq!(e.status, Status::Kept);
        assert_eq!(e.tags, vec!["fav"]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = &mut lib;
        Ok(())
    }
}
