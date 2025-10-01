//! Project layout: project-id derivation and on-disk `.hal/<id>/` tree.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Resolved on-disk paths for a given `.hal` source file.
#[derive(Debug, Clone)]
pub struct Project {
    /// Absolute, canonicalized path to the `.hal` source file.
    pub source: PathBuf,
    /// Short content-addressed project identifier (hex).
    pub id: String,
    /// `.hal/<id>/` root, always absolute.
    pub root: PathBuf,
}

/// Contents of `build.lock`, the cache-invalidation anchor for `hal run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildLock {
    pub source_hash: String,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub built_at: chrono::DateTime<chrono::Utc>,
    pub image: String,
    pub entrypoint: String,
}

impl Project {
    /// Resolve paths for `source`. The source file must exist.
    pub fn open(source: &Path) -> Result<Self> {
        let source = source
            .canonicalize()
            .with_context(|| format!("source file not found: {}", source.display()))?;
        let id = project_id(&source);
        let parent = source
            .parent()
            .with_context(|| format!("source has no parent directory: {}", source.display()))?;
        let root = parent.join(".hal").join(&id);
        Ok(Self { source, id, root })
    }

    /// Create the `.hal/<id>/` tree on disk (idempotent).
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.root.join("artifacts"))?;
        fs::create_dir_all(self.root.join("runs"))?;
        fs::create_dir_all(self.root.join("memory"))?;
        fs::create_dir_all(self.root.join("logs"))?;
        Ok(())
    }

    pub fn logic_md(&self) -> PathBuf {
        self.root.join("logic.md")
    }
    pub fn artifacts_dir(&self) -> PathBuf {
        self.root.join("artifacts")
    }
    pub fn runtime_yaml(&self) -> PathBuf {
        self.root.join("runtime.yaml")
    }
    pub fn build_lock(&self) -> PathBuf {
        self.root.join("build.lock")
    }
    pub fn memory_db(&self) -> PathBuf {
        self.root.join("memory").join("hal.sqlite")
    }
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }
    /// Per-project HAL log directory (`<project>/.hal/<id>/logs/`).
    /// Distinct from the cross-project log at `~/.hal/logs/hal.log`.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Create a new timestamped `runs/<ts>/` directory and return it.
    pub fn new_run_dir(&self) -> Result<PathBuf> {
        let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let p = self.runs_dir().join(&ts);
        fs::create_dir_all(&p)?;
        Ok(p)
    }

    /// Read the raw `.hal` source bytes.
    pub fn read_source(&self) -> Result<Vec<u8>> {
        fs::read(&self.source).context("reading .hal source")
    }

    /// Blake3 hex of the current source contents.
    pub fn source_hash(&self) -> Result<String> {
        let bytes = self.read_source()?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    pub fn read_build_lock(&self) -> Result<Option<BuildLock>> {
        let path = self.build_lock();
        if !path.exists() {
            return Ok(None);
        }
        let s = fs::read_to_string(&path)?;
        let lock: BuildLock =
            serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(lock))
    }

    pub fn write_build_lock(&self, lock: &BuildLock) -> Result<()> {
        let s = serde_json::to_string_pretty(lock)?;
        fs::write(self.build_lock(), s)?;
        Ok(())
    }

    /// Wipe the materialised build outputs (`logic.md`, `runtime.yaml`,
    /// `build.lock`, and the contents of `artifacts/`) so the next build
    /// starts from a clean slate. The `memory/` and `runs/` directories
    /// are preserved — they're durable across rebuilds on purpose.
    pub fn invalidate_build(&self) -> Result<()> {
        for p in [self.logic_md(), self.runtime_yaml(), self.build_lock()] {
            if p.exists() {
                fs::remove_file(&p).with_context(|| format!("removing stale {}", p.display()))?;
            }
        }
        let art = self.artifacts_dir();
        if art.exists() {
            for entry in fs::read_dir(&art).with_context(|| format!("reading {}", art.display()))? {
                let entry = entry?;
                let p = entry.path();
                if p.is_dir() {
                    fs::remove_dir_all(&p)
                        .with_context(|| format!("removing stale {}", p.display()))?;
                } else {
                    fs::remove_file(&p)
                        .with_context(|| format!("removing stale {}", p.display()))?;
                }
            }
        }
        fs::create_dir_all(&art)?;
        tracing::info!(
            target: "hal::project",
            project_id = %self.id,
            root = %self.root.display(),
            "invalidated build outputs (logic.md, runtime.yaml, build.lock, artifacts/*)"
        );
        Ok(())
    }
}

/// Project identifier: first 16 hex chars of blake3(absolute_source_path).
pub fn project_id(abs_source: &Path) -> String {
    let hex = blake3::hash(abs_source.as_os_str().as_encoded_bytes()).to_hex();
    hex[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_layout_under_sibling_dot_hal() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("demo.hal");
        fs::write(&src, b"print hi\n").unwrap();

        let p = Project::open(&src).unwrap();
        p.ensure_layout().unwrap();

        assert!(p.root.starts_with(dir.path().join(".hal")));
        assert!(p.root.ends_with(&p.id));
        assert!(p.artifacts_dir().is_dir());
        assert!(p.runs_dir().is_dir());
        assert!(p.memory_db().parent().unwrap().is_dir());
        assert_eq!(p.id.len(), 16);
    }

    #[test]
    fn source_hash_changes_when_contents_change() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("x.hal");
        fs::write(&src, b"one").unwrap();
        let p = Project::open(&src).unwrap();
        let a = p.source_hash().unwrap();
        fs::write(&src, b"two").unwrap();
        let b = p.source_hash().unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn project_id_stable_for_same_path() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("stable.hal");
        fs::write(&src, b"x").unwrap();
        let p1 = Project::open(&src).unwrap();
        let p2 = Project::open(&src).unwrap();
        assert_eq!(p1.id, p2.id);
    }

    #[test]
    fn open_rejects_missing_source() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope.hal");
        let err = Project::open(&missing).unwrap_err();
        assert!(format!("{err}").contains("source file not found"));
    }
}
