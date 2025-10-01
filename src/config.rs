//! Runtime configuration.
//!
//! Resolution order (highest priority first):
//! 1. Process environment variables (`GROK_MODEL`, `MAPERTUI_XAI_TOKEN`, …)
//! 2. Per-project TOML at `<source_parent>/.hal/config.toml`
//! 3. Built-in defaults
//!
//! The TOML file is the canonical place to pin model names, embedding
//! dimensions, image choice, and Conductor reflection behaviour.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Default chat/completion model when nothing else is set.
pub const DEFAULT_MODEL: &str = "grok-4.20-0309-reasoning";

/// Default embeddings model (production-grade, 4096-dim).
pub const DEFAULT_EMBEDDING_MODEL: &str = "embedding-beta-3-large";

/// Default embeddings dimensionality — must match [`DEFAULT_EMBEDDING_MODEL`].
pub const DEFAULT_EMBEDDING_DIMS: usize = 4096;

/// Default Docker image used by the Conductor when `runtime.yaml` does
/// not specify one. We pick `debian:stable` (not `-slim`) so that the
/// default runtime ships with `bash`, `apt`, coreutils, `curl`,
/// `ca-certificates` and friends out of the box — enough for most
/// shell scripts to run unmodified. Language-specific images
/// (`python:3.12-slim`, `node:20-slim`, etc.) are still preferred
/// when the Architect can pick one that matches the workload.
pub const DEFAULT_IMAGE: &str = "debian:stable";

/// xAI OpenAI-compatible endpoint.
pub const XAI_BASE_URL: &str = "https://api.x.ai/v1";

/// Default maximum turns for the Reflector agent.
pub const DEFAULT_REFLECT_MAX_TURNS: usize = 6;

/// Default top-k for memory searches.
pub const DEFAULT_MEMORY_SEARCH_K: usize = 5;

/// File-backed slice of [`Config`]. Every field is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HalConfigFile {
    pub model: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dims: Option<usize>,
    pub default_image: Option<String>,
    pub base_url: Option<String>,
    pub conductor_reflect: Option<bool>,
    pub conductor_reflect_max_turns: Option<usize>,
    pub memory_search_k: Option<usize>,
}

impl HalConfigFile {
    pub fn from_toml_str(s: &str) -> Result<Self> {
        toml::from_str::<Self>(s).context("parsing .hal/config.toml")
    }

    pub fn load_if_exists(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let s =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Some(Self::from_toml_str(&s)?))
    }
}

/// Resolved runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub xai_api_key: Option<String>,
    pub model: String,
    pub embedding_model: String,
    pub embedding_dims: usize,
    pub default_image: String,
    pub base_url: String,
    pub conductor_reflect: bool,
    pub conductor_reflect_max_turns: usize,
    pub memory_search_k: usize,
    /// Path of the TOML file that contributed to this config, if any.
    pub source_file: Option<PathBuf>,
}

impl Config {
    /// Defaults only — no env, no file.
    pub fn defaults() -> Self {
        Self {
            xai_api_key: None,
            model: DEFAULT_MODEL.to_string(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
            embedding_dims: DEFAULT_EMBEDDING_DIMS,
            default_image: DEFAULT_IMAGE.to_string(),
            base_url: XAI_BASE_URL.to_string(),
            conductor_reflect: true,
            conductor_reflect_max_turns: DEFAULT_REFLECT_MAX_TURNS,
            memory_search_k: DEFAULT_MEMORY_SEARCH_K,
            source_file: None,
        }
    }

    /// Load from process environment (no file).
    pub fn from_env() -> Self {
        let mut c = Self::defaults();
        c.apply_env();
        c
    }

    /// Load env overlaid on `<source_parent>/.hal/config.toml` if present.
    pub fn load_for_source(source: &Path) -> Result<Self> {
        let mut c = Self::defaults();
        if let Some(parent) = source.parent() {
            let path = parent.join(".hal").join("config.toml");
            if let Some(file) = HalConfigFile::load_if_exists(&path)? {
                c.apply_file(&file);
                c.source_file = Some(path);
            }
        }
        c.apply_env();
        Ok(c)
    }

    fn apply_file(&mut self, f: &HalConfigFile) {
        if let Some(v) = f.model.as_ref() {
            self.model = v.clone();
        }
        if let Some(v) = f.embedding_model.as_ref() {
            self.embedding_model = v.clone();
        }
        if let Some(v) = f.embedding_dims {
            self.embedding_dims = v;
        }
        if let Some(v) = f.default_image.as_ref() {
            self.default_image = v.clone();
        }
        if let Some(v) = f.base_url.as_ref() {
            self.base_url = v.clone();
        }
        if let Some(v) = f.conductor_reflect {
            self.conductor_reflect = v;
        }
        if let Some(v) = f.conductor_reflect_max_turns {
            self.conductor_reflect_max_turns = v;
        }
        if let Some(v) = f.memory_search_k {
            self.memory_search_k = v;
        }
    }

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("MAPERTUI_XAI_TOKEN") {
            if !v.is_empty() {
                self.xai_api_key = Some(v);
            }
        }
        if let Ok(v) = std::env::var("GROK_MODEL") {
            if !v.is_empty() {
                self.model = v;
            }
        }
        if let Ok(v) = std::env::var("HAL_EMBEDDING_MODEL") {
            if !v.is_empty() {
                self.embedding_model = v;
            }
        }
        if let Ok(v) = std::env::var("HAL_EMBEDDING_DIMS") {
            if let Ok(n) = v.parse::<usize>() {
                self.embedding_dims = n;
            }
        }
        if let Ok(v) = std::env::var("HAL_DEFAULT_IMAGE") {
            if !v.is_empty() {
                self.default_image = v;
            }
        }
        if let Ok(v) = std::env::var("XAI_BASE_URL") {
            if !v.is_empty() {
                self.base_url = v;
            }
        }
        if let Ok(v) = std::env::var("HAL_CONDUCTOR_REFLECT") {
            self.conductor_reflect = matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        if let Ok(v) = std::env::var("HAL_CONDUCTOR_REFLECT_MAX_TURNS") {
            if let Ok(n) = v.parse::<usize>() {
                self.conductor_reflect_max_turns = n;
            }
        }
        if let Ok(v) = std::env::var("HAL_MEMORY_SEARCH_K") {
            if let Ok(n) = v.parse::<usize>() {
                self.memory_search_k = n;
            }
        }
    }

    pub fn require_api_key(&self) -> Result<&str> {
        self.xai_api_key
            .as_deref()
            .context("MAPERTUI_XAI_TOKEN is not set; add it to HAL/.env or the environment")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }
    impl EnvGuard {
        fn remove(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prev }
        }
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn defaults_apply_when_env_missing() {
        let _g1 = EnvGuard::remove("GROK_MODEL");
        let _g2 = EnvGuard::remove("HAL_EMBEDDING_MODEL");
        let _g3 = EnvGuard::remove("HAL_EMBEDDING_DIMS");
        let _g4 = EnvGuard::remove("HAL_DEFAULT_IMAGE");
        let _g5 = EnvGuard::remove("HAL_CONDUCTOR_REFLECT");
        let c = Config::from_env();
        assert_eq!(c.model, DEFAULT_MODEL);
        assert_eq!(c.embedding_model, DEFAULT_EMBEDDING_MODEL);
        assert_eq!(c.embedding_dims, DEFAULT_EMBEDDING_DIMS);
        assert_eq!(c.default_image, DEFAULT_IMAGE);
        assert_eq!(c.base_url, XAI_BASE_URL);
        assert!(c.conductor_reflect);
    }

    #[test]
    fn parses_full_toml() {
        let s = r#"
model = "grok-test"
embedding_model = "embedding-beta-3-small"
embedding_dims = 1024
default_image = "python:3.12-slim"
conductor_reflect = false
conductor_reflect_max_turns = 3
memory_search_k = 9
"#;
        let f = HalConfigFile::from_toml_str(s).unwrap();
        assert_eq!(f.model.as_deref(), Some("grok-test"));
        assert_eq!(f.embedding_dims, Some(1024));
        assert_eq!(f.conductor_reflect, Some(false));
        assert_eq!(f.memory_search_k, Some(9));
    }

    #[test]
    fn unknown_toml_keys_are_rejected() {
        let s = r#"
model = "x"
definitely_not_a_key = 1
"#;
        assert!(HalConfigFile::from_toml_str(s).is_err());
    }

    #[test]
    fn toml_file_overrides_defaults_but_env_overrides_toml() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("prog.hal");
        std::fs::write(&src, b"noop\n").unwrap();
        let cfg_dir = dir.path().join(".hal");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("config.toml"),
            r#"
model = "from-toml"
embedding_model = "embedding-beta-3-small"
embedding_dims = 1024
conductor_reflect = false
"#,
        )
        .unwrap();

        let _g1 = EnvGuard::remove("GROK_MODEL");
        let _g2 = EnvGuard::remove("HAL_EMBEDDING_MODEL");
        let _g3 = EnvGuard::remove("HAL_EMBEDDING_DIMS");
        let _g4 = EnvGuard::remove("HAL_CONDUCTOR_REFLECT");
        let c = Config::load_for_source(&src).unwrap();
        assert_eq!(c.model, "from-toml");
        assert_eq!(c.embedding_model, "embedding-beta-3-small");
        assert_eq!(c.embedding_dims, 1024);
        assert!(!c.conductor_reflect);
        assert!(c.source_file.is_some());

        let _gm = EnvGuard::set("GROK_MODEL", "from-env");
        let c = Config::load_for_source(&src).unwrap();
        assert_eq!(c.model, "from-env");
    }

    #[test]
    fn missing_toml_is_not_an_error() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("no_cfg.hal");
        std::fs::write(&src, b"x").unwrap();
        let _g1 = EnvGuard::remove("GROK_MODEL");
        let c = Config::load_for_source(&src).unwrap();
        assert_eq!(c.model, DEFAULT_MODEL);
        assert!(c.source_file.is_none());
    }
}
