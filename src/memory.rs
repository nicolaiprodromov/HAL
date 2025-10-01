//! Per-project memory store backed by SQLite with vector search.
//!
//! Each note carries an embedding vector (f32 little-endian blob). Search
//! embeds the query, then ranks notes within the same agent namespace by
//! cosine similarity. When no embedder is configured (e.g. offline tests
//! that don't want to mock one), writes skip the embedding and search
//! falls back to most-recent-first with a LIKE substring filter.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio_rusqlite::{params, Connection};

use crate::events::Agent;
use crate::llm::Embedder;

/// A per-agent note retrieved from memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: i64,
    pub agent: Agent,
    pub content: String,
    pub created_at: i64,
    /// Cosine similarity of the note against the query, when search
    /// produced it via vector ranking. `None` for recency / LIKE hits.
    pub score: Option<f32>,
}

/// SQLite-backed memory store, partitioned by `Agent`.
#[derive(Clone)]
pub struct Memory {
    conn: Connection,
    embedder: Option<Arc<dyn Embedder>>,
}

impl std::fmt::Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memory")
            .field("embedder", &self.embedder.as_ref().map(|e| e.model()))
            .finish_non_exhaustive()
    }
}

fn agent_tag(a: Agent) -> &'static str {
    match a {
        Agent::Interpreter => "interpreter",
        Agent::Architect => "architect",
        Agent::Conductor => "conductor",
    }
}

fn parse_agent(tag: &str) -> rusqlite::Result<Agent> {
    Ok(match tag {
        "interpreter" => Agent::Interpreter,
        "architect" => Agent::Architect,
        "conductor" => Agent::Conductor,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown agent tag: {other}").into(),
            ));
        }
    })
}

fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn bytes_to_vec(b: &[u8]) -> Option<Vec<f32>> {
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let (chunks, _) = b.as_chunks::<4>();
    Some(chunks.iter().map(|c| f32::from_le_bytes(*c)).collect())
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

impl Memory {
    /// Open (or create) the memory database at `path`, without embeddings.
    pub async fn open(path: &Path) -> Result<Self> {
        Self::open_with_embedder(path, None).await
    }

    /// Open (or create) the memory database, attaching an embedder for
    /// future writes and vector searches.
    pub async fn open_with_embedder(
        path: &Path,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating memory dir")?;
        }
        let conn = Connection::open(path.to_path_buf())
            .await
            .context("opening memory sqlite")?;
        Self::bootstrap(conn, embedder).await
    }

    /// In-memory database (tests).
    pub async fn in_memory() -> Result<Self> {
        Self::in_memory_with_embedder(None).await
    }

    pub async fn in_memory_with_embedder(embedder: Option<Arc<dyn Embedder>>) -> Result<Self> {
        let conn = Connection::open(":memory:")
            .await
            .context("opening in-memory sqlite")?;
        Self::bootstrap(conn, embedder).await
    }

    async fn bootstrap(conn: Connection, embedder: Option<Arc<dyn Embedder>>) -> Result<Self> {
        conn.call(|c| {
            c.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS notes (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent       TEXT NOT NULL,
                    content     TEXT NOT NULL,
                    created_at  INTEGER NOT NULL,
                    embedding   BLOB,
                    embed_model TEXT,
                    embed_dims  INTEGER
                );
                CREATE INDEX IF NOT EXISTS idx_notes_agent ON notes(agent);
                "#,
            )?;
            // Backwards-compat: earlier versions of this crate used an FTS5
            // virtual table + plain `notes` schema. If we find an old DB,
            // add any missing columns so opens don't fail.
            let cols: Vec<String> = {
                let mut stmt = c.prepare("PRAGMA table_info(notes)")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            if !cols.iter().any(|s| s == "embedding") {
                c.execute("ALTER TABLE notes ADD COLUMN embedding BLOB", [])?;
            }
            if !cols.iter().any(|s| s == "embed_model") {
                c.execute("ALTER TABLE notes ADD COLUMN embed_model TEXT", [])?;
            }
            if !cols.iter().any(|s| s == "embed_dims") {
                c.execute("ALTER TABLE notes ADD COLUMN embed_dims INTEGER", [])?;
            }
            // Drop the stale FTS5 table if present — vector search supersedes it.
            let _ = c.execute("DROP TABLE IF EXISTS notes_fts", []);
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("bootstrap memory schema: {e}"))?;
        Ok(Self { conn, embedder })
    }

    /// Whether this store has a configured embedder.
    pub fn has_embedder(&self) -> bool {
        self.embedder.is_some()
    }

    /// Append a note to `agent`'s namespace. Returns the note id.
    pub async fn write(&self, agent: Agent, content: &str) -> Result<i64> {
        let tag = agent_tag(agent).to_string();
        let content_s = content.to_string();
        let ts = chrono::Utc::now().timestamp();
        let (blob, model, dims) = match self.embedder.as_ref() {
            Some(e) => {
                let v = crate::llm::embed_one(e.as_ref(), content).await?;
                (
                    Some(vec_to_bytes(&v)),
                    Some(e.model().to_string()),
                    Some(v.len() as i64),
                )
            }
            None => (None, None, None),
        };
        let id = self
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO notes(agent, content, created_at, embedding, embed_model, embed_dims)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![tag, content_s, ts, blob, model, dims],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .map_err(|e| anyhow::anyhow!("memory write: {e}"))?;
        Ok(id)
    }

    /// Return up to `k` notes scoped to `agent`, ranked by cosine
    /// similarity against `query`'s embedding. When there is no embedder
    /// or the query is empty, falls back to recency (+ LIKE filter if a
    /// non-empty query is given).
    pub async fn search(&self, agent: Agent, query: &str, k: usize) -> Result<Vec<Note>> {
        let tag = agent_tag(agent).to_string();
        let q = query.trim().to_string();
        let k = k.max(1);

        // No embedder → recency (+ LIKE) fallback.
        let Some(embedder) = self.embedder.clone() else {
            return self.search_no_vector(tag, q, k).await;
        };
        if q.is_empty() {
            return self.search_no_vector(tag, q, k).await;
        }

        let qvec = crate::llm::embed_one(embedder.as_ref(), &q).await?;
        // Pull all embedded rows for this agent; rank in-process.
        let rows = self
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT id, agent, content, created_at, embedding
                     FROM notes
                     WHERE agent = ?1 AND embedding IS NOT NULL",
                )?;
                let it = stmt.query_map(params![tag], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Vec<u8>>(4)?,
                    ))
                })?;
                let mut out = Vec::new();
                for row in it {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
            .map_err(|e| anyhow::anyhow!("memory search fetch: {e}"))?;

        let mut scored: Vec<(f32, Note)> = rows
            .into_iter()
            .filter_map(|(id, tag, content, ts, blob)| {
                let v = bytes_to_vec(&blob)?;
                if v.len() != qvec.len() {
                    return None;
                }
                let score = cosine(&qvec, &v);
                let agent = parse_agent(&tag).ok()?;
                Some((
                    score,
                    Note {
                        id,
                        agent,
                        content,
                        created_at: ts,
                        score: Some(score),
                    },
                ))
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored.into_iter().map(|(_, n)| n).collect())
    }

    async fn search_no_vector(&self, tag: String, q: String, k: usize) -> Result<Vec<Note>> {
        let k = k as i64;
        let rows = self
            .conn
            .call(move |c| {
                let mut out = Vec::<Note>::new();
                if q.is_empty() {
                    let mut stmt = c.prepare(
                        "SELECT id, agent, content, created_at
                         FROM notes
                         WHERE agent = ?1
                         ORDER BY created_at DESC, id DESC
                         LIMIT ?2",
                    )?;
                    let it = stmt.query_map(params![tag, k], row_to_note_basic)?;
                    for n in it {
                        out.push(n?);
                    }
                } else {
                    let pat = format!("%{}%", q);
                    let mut stmt = c.prepare(
                        "SELECT id, agent, content, created_at
                         FROM notes
                         WHERE agent = ?1 AND content LIKE ?2
                         ORDER BY created_at DESC, id DESC
                         LIMIT ?3",
                    )?;
                    let it = stmt.query_map(params![tag, pat, k], row_to_note_basic)?;
                    for n in it {
                        out.push(n?);
                    }
                }
                Ok(out)
            })
            .await
            .map_err(|e| anyhow::anyhow!("memory search (no-vec): {e}"))?;
        Ok(rows)
    }

    /// Count of notes for an agent (diagnostics / tests).
    pub async fn count(&self, agent: Agent) -> Result<i64> {
        let tag = agent_tag(agent).to_string();
        let n = self
            .conn
            .call(move |c| {
                let n: i64 = c.query_row(
                    "SELECT COUNT(*) FROM notes WHERE agent = ?1",
                    params![tag],
                    |r| r.get(0),
                )?;
                Ok(n)
            })
            .await
            .map_err(|e| anyhow::anyhow!("memory count: {e}"))?;
        Ok(n)
    }
}

fn row_to_note_basic(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let agent: String = row.get(1)?;
    Ok(Note {
        id: row.get(0)?,
        agent: parse_agent(&agent)?,
        content: row.get(2)?,
        created_at: row.get(3)?,
        score: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockEmbedder;

    fn emb(dims: usize) -> Arc<dyn Embedder> {
        Arc::new(MockEmbedder::new(dims))
    }

    #[tokio::test]
    async fn vector_search_ranks_semantic_match_first() {
        let m = Memory::in_memory_with_embedder(Some(emb(128)))
            .await
            .unwrap();
        m.write(Agent::Architect, "use bash script to print fizzbuzz")
            .await
            .unwrap();
        m.write(Agent::Architect, "prefer python for numerics")
            .await
            .unwrap();
        m.write(Agent::Architect, "debian slim boots quickly")
            .await
            .unwrap();

        let hits = m
            .search(Agent::Architect, "bash fizzbuzz", 3)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].content.contains("bash"));
        assert!(hits[0].score.unwrap() > 0.0);
    }

    #[tokio::test]
    async fn agents_are_partitioned() {
        let m = Memory::in_memory_with_embedder(Some(emb(64)))
            .await
            .unwrap();
        m.write(Agent::Interpreter, "interp-only note")
            .await
            .unwrap();
        m.write(Agent::Architect, "arch-only note").await.unwrap();
        let hits = m.search(Agent::Architect, "interp", 5).await.unwrap();
        assert!(hits.iter().all(|n| n.content.contains("arch")));
    }

    #[tokio::test]
    async fn empty_query_returns_most_recent() {
        let m = Memory::in_memory_with_embedder(Some(emb(64)))
            .await
            .unwrap();
        m.write(Agent::Architect, "first").await.unwrap();
        m.write(Agent::Architect, "second").await.unwrap();
        let hits = m.search(Agent::Architect, "", 10).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].content, "second");
    }

    #[tokio::test]
    async fn persists_across_reopen_with_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mem.sqlite");
        {
            let m = Memory::open_with_embedder(&path, Some(emb(32)))
                .await
                .unwrap();
            m.write(Agent::Conductor, "pin this bash script")
                .await
                .unwrap();
        }
        let m2 = Memory::open_with_embedder(&path, Some(emb(32)))
            .await
            .unwrap();
        let hits = m2.search(Agent::Conductor, "bash", 5).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn without_embedder_falls_back_to_recency_and_like() {
        let m = Memory::in_memory().await.unwrap();
        m.write(Agent::Architect, "alpha bravo").await.unwrap();
        m.write(Agent::Architect, "charlie delta").await.unwrap();
        let recent = m.search(Agent::Architect, "", 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        let like = m.search(Agent::Architect, "alpha", 10).await.unwrap();
        assert_eq!(like.len(), 1);
        assert!(like[0].content.contains("alpha"));
    }

    #[tokio::test]
    async fn migrates_old_schema_without_embedding_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sqlite");
        // Create an old-shape DB: no embedding columns.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE notes(
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                INSERT INTO notes(agent, content, created_at)
                VALUES('architect', 'legacy note', 1);",
            )
            .unwrap();
        }
        let m = Memory::open_with_embedder(&path, Some(emb(16)))
            .await
            .unwrap();
        // New writes embed; old note is still reachable via LIKE (no embedding).
        m.write(Agent::Architect, "new note with bash")
            .await
            .unwrap();
        let hits = m.search(Agent::Architect, "bash", 5).await.unwrap();
        assert!(hits.iter().any(|n| n.content.contains("bash")));
    }
}
