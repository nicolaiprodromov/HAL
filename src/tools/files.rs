//! `files_read` / `files_write` tools.
//!
//! Both tools sandbox paths to the project's `.hal/<id>/` root plus the
//! original `.hal` source file (read-only). Path traversal is blocked by
//! [`super::resolve_sandboxed`].

use std::path::PathBuf;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};

use super::{resolve_sandboxed, ToolContext};
use crate::events::Agent;

#[derive(Debug, thiserror::Error)]
pub enum FileToolError {
    #[error("{0}")]
    Anyhow(String),
}

impl From<anyhow::Error> for FileToolError {
    fn from(e: anyhow::Error) -> Self {
        Self::Anyhow(format!("{e:#}"))
    }
}
impl From<std::io::Error> for FileToolError {
    fn from(e: std::io::Error) -> Self {
        Self::Anyhow(e.to_string())
    }
}

#[derive(Debug, Deserialize)]
pub struct FilesReadArgs {
    /// Absolute or project-relative path. Must stay inside the project's
    /// `.hal/<id>/` root or match the source `.hal` file.
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct FilesReadOut {
    pub path: String,
    pub contents: String,
}

#[derive(Clone)]
pub struct FilesRead {
    pub ctx: ToolContext,
}

impl FilesRead {
    fn roots(&self) -> Vec<PathBuf> {
        vec![
            self.ctx.project.root.clone(),
            self.ctx
                .project
                .source
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| self.ctx.project.root.clone()),
        ]
    }
}

impl Tool for FilesRead {
    const NAME: &'static str = "files_read";
    type Error = FileToolError;
    type Args = FilesReadArgs;
    type Output = FilesReadOut;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description:
                "Read a file from the project's .hal/<id>/ bundle or the source .hal file.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute or project-relative path"}
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let guard = self
            .ctx
            .begin_call(Self::NAME, format!("path={}", args.path));
        let roots = self.roots();
        // Resolve relative paths by trying, in order:
        //   1. verbatim (relative to project root)
        //   2. under artifacts/ (where most generated files live)
        //   3. under the source .hal file's directory
        // The first candidate that actually exists wins; otherwise we
        // fall back to (1) so errors point at the sensible default.
        let candidates: Vec<std::path::PathBuf> = {
            let raw = std::path::Path::new(&args.path);
            if raw.is_absolute() {
                vec![raw.to_path_buf()]
            } else {
                let mut v = Vec::new();
                v.push(self.ctx.project.root.join(raw));
                v.push(self.ctx.project.artifacts_dir().join(raw));
                if let Some(parent) = self.ctx.project.source.parent() {
                    v.push(parent.join(raw));
                }
                v
            }
        };
        let pick = candidates
            .iter()
            .find(|c| c.exists())
            .cloned()
            .unwrap_or_else(|| candidates[0].clone());
        let p = match resolve_sandboxed(&pick, &roots) {
            Ok(p) => p,
            Err(e) => {
                guard.finish_err(format!("{e:#}"));
                return Err(e.into());
            }
        };
        // Extra rule: outside project root, only the source file itself is readable.
        let is_project = p.starts_with(&self.ctx.project.root);
        let is_source = p == self.ctx.project.source;
        if !is_project && !is_source {
            let msg = format!(
                "read refused: {} is not inside the project bundle and is not the source file",
                p.display()
            );
            guard.finish_err(msg.clone());
            return Err(FileToolError::Anyhow(msg));
        }
        match std::fs::read_to_string(&p) {
            Ok(contents) => {
                guard.finish_ok(format!(
                    "{} ({} bytes)\n{}",
                    p.display(),
                    contents.len(),
                    contents
                ));
                Ok(FilesReadOut {
                    path: p.display().to_string(),
                    contents,
                })
            }
            Err(e) => {
                let msg = e.to_string();
                guard.finish_err(msg.clone());
                Err(FileToolError::Anyhow(msg))
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct FilesWriteArgs {
    /// Absolute or project-relative path. Must stay inside `.hal/<id>/`.
    pub path: String,
    pub contents: String,
}

#[derive(Debug, Serialize)]
pub struct FilesWriteOut {
    pub path: String,
    pub bytes_written: usize,
}

#[derive(Clone)]
pub struct FilesWrite {
    pub ctx: ToolContext,
}

impl Tool for FilesWrite {
    const NAME: &'static str = "files_write";
    type Error = FileToolError;
    type Args = FilesWriteArgs;
    type Output = FilesWriteOut;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Write a file inside the project's .hal/<id>/ bundle. Creates parent directories as needed.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "contents": {"type": "string"}
                },
                "required": ["path", "contents"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let guard = self.ctx.begin_call(
            Self::NAME,
            format!("path={}\ncontents=\n{}", args.path, args.contents),
        );
        // Conductor and Reflector are not allowed to write artifacts.
        if matches!(self.ctx.agent, Agent::Conductor | Agent::Reflector) {
            let msg = format!(
                "files_write is not available to the {} (artifacts are read-only at run time)",
                self.ctx.agent
            );
            guard.finish_err(msg.clone());
            return Err(FileToolError::Anyhow(msg));
        }
        let roots = vec![self.ctx.project.root.clone()];
        // Resolve relative paths. Anything that isn't obviously a
        // top-level well-known file (`logic.md`, `runtime.yaml`,
        // `build.lock`) defaults to the `artifacts/` subdir — the
        // Architect overwhelmingly writes scripts/configs there.
        let raw = std::path::Path::new(&args.path);
        let pick = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            let s = args.path.as_str();
            let top_level = matches!(s, "logic.md" | "runtime.yaml" | "build.lock")
                || s.starts_with(".hal/")
                || s.starts_with("artifacts/")
                || s.contains('/');
            if top_level {
                self.ctx.project.root.join(raw)
            } else {
                self.ctx.project.artifacts_dir().join(raw)
            }
        };
        let p = match resolve_sandboxed(&pick, &roots) {
            Ok(p) => p,
            Err(e) => {
                guard.finish_err(format!("{e:#}"));
                return Err(e.into());
            }
        };
        if !p.starts_with(&self.ctx.project.root) {
            let msg = format!(
                "write refused: {} is outside the project bundle",
                p.display()
            );
            guard.finish_err(msg.clone());
            return Err(FileToolError::Anyhow(msg));
        }
        if let Some(parent) = p.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                guard.finish_err(e.to_string());
                return Err(FileToolError::from(e));
            }
        }
        match std::fs::write(&p, &args.contents) {
            Ok(()) => {
                let bytes = args.contents.len();
                guard.finish_ok(format!(
                    "wrote {} ({} bytes, {} lines)",
                    p.display(),
                    bytes,
                    args.contents.lines().count()
                ));
                Ok(FilesWriteOut {
                    path: p.display().to_string(),
                    bytes_written: bytes,
                })
            }
            Err(e) => {
                guard.finish_err(e.to_string());
                Err(FileToolError::from(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Memory;
    use crate::project::Project;
    use std::sync::Arc;
    use tempfile::tempdir;

    async fn ctx(agent: Agent) -> (tempfile::TempDir, ToolContext) {
        let dir = tempdir().unwrap();
        let src = dir.path().join("demo.hal");
        std::fs::write(&src, b"sleep 1\n").unwrap();
        let project = Project::open(&src).unwrap();
        project.ensure_layout().unwrap();
        let memory = Memory::in_memory().await.unwrap();
        let ctx = ToolContext::new(Arc::new(project), Arc::new(memory), agent);
        (dir, ctx)
    }

    #[tokio::test]
    async fn write_then_read_roundtrips() {
        let (_d, ctx) = ctx(Agent::Architect).await;
        let w = FilesWrite { ctx: ctx.clone() };
        let rel = "artifacts/run.sh";
        w.call(FilesWriteArgs {
            path: rel.into(),
            contents: "#!/bin/bash\necho hi\n".into(),
        })
        .await
        .unwrap();

        let r = FilesRead { ctx };
        let out = r.call(FilesReadArgs { path: rel.into() }).await.unwrap();
        assert!(out.contents.contains("echo hi"));
    }

    #[tokio::test]
    async fn read_refuses_outside_project() {
        let (dir, ctx) = ctx(Agent::Architect).await;
        let other = dir.path().parent().unwrap().join("not-the-project.txt");
        let _ = std::fs::write(&other, b"x");
        let r = FilesRead { ctx };
        let err = r
            .call(FilesReadArgs {
                path: other.display().to_string(),
            })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("escapes") || format!("{err}").contains("refused"));
        let _ = std::fs::remove_file(&other);
    }

    #[tokio::test]
    async fn conductor_cannot_write() {
        let (_d, ctx) = ctx(Agent::Conductor).await;
        let w = FilesWrite { ctx };
        let err = w
            .call(FilesWriteArgs {
                path: "artifacts/x".into(),
                contents: "y".into(),
            })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("Conductor"));
    }

    #[tokio::test]
    async fn reflector_cannot_write_but_can_read() {
        let (_d, ctx) = ctx(Agent::Reflector).await;
        let w = FilesWrite { ctx: ctx.clone() };
        let err = w
            .call(FilesWriteArgs {
                path: "artifacts/x".into(),
                contents: "y".into(),
            })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("Reflector"));
        let src = ctx.project.source.display().to_string();
        let out = FilesRead { ctx }
            .call(FilesReadArgs { path: src })
            .await
            .unwrap();
        assert!(out.contents.contains("sleep"));
    }

    #[tokio::test]
    async fn read_allows_the_source_hal_file() {
        let (_d, ctx) = ctx(Agent::Interpreter).await;
        let src = ctx.project.source.display().to_string();
        let r = FilesRead { ctx };
        let out = r.call(FilesReadArgs { path: src }).await.unwrap();
        assert!(out.contents.contains("sleep"));
    }
}
