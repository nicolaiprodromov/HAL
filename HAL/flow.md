# Flow

```
user edits script.hal
        │
        ▼
  hal build / hal run
        │
        ▼
  Interpreter ── memory_search ── prior logic.md?
        │              │
        │         reuse │ draft
        ▼              ▼
             logic.md (YAML + prose)
        │
        ▼
  Architect ── spawn_agent (≤4) ──► artifacts/
        │                           runtime.yaml
        ▼
  build.lock written { source_hash, image, entrypoint, built_at }
        │
        ▼  (only on `hal run`)
  Conductor ── bollard ──► Docker container
        │                   /work  ← artifacts/ (ro)
        │                   runs command, streams stdout/stderr
        ▼
  runs/<ts>/ { stdout.log, stderr.log, meta.json }
        │
        ▼
  TUI renders Done / Failed; exit code propagates to shell
```

Caching: a `hal run` whose source hash matches `build.lock` skips
Interpreter + Architect. The Interpreter may still be consulted via a
memory query when ambiguous intent is suspected.
