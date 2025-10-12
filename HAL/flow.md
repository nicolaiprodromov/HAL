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
        │                   /work  ← artifacts/ (rw by default)
        │                   runs command, streams stdout/stderr
        ▼
  runs/<ts>/ { stdout.log, stderr.log, meta.json }
        │
        ▼
  Conductor writes bare run facts ──► memory: conductor
        │
        ▼  (skipped if interrupted or reflection is off)
  Reflector ── memory_search, files_read, run_cmd probes ──►
        │      memory_write ──► memory: reflector
        │      (read-only; cannot re-run; ends with REFLECTION_DONE)
        ▼
  TUI renders Done / Failed; exit code propagates to shell

  next build: preamble digest + memory_search("reflector")
              feed Interpreter and Architect  ──► (loop to top)
```

Caching: a `hal run` whose source hash matches `build.lock` skips
Interpreter + Architect. The Interpreter may still be consulted via a
memory query when ambiguous intent is suspected.

Memory namespaces: `interpreter`, `architect`, `conductor` (run facts),
`reflector` (observations and fixes).
