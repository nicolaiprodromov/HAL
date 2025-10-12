# Tools

The LLM agents share a tool surface. Prompts scope who can call what.
The Conductor is deterministic Rust and has no LLM tools.

| Tool | Interpreter | Architect | Conductor | Reflector |
|---|:-:|:-:|:-:|:-:|
| `memory_search` | yes | yes | — | yes |
| `memory_write`  | yes | yes | — | yes |
| `files_read`    | yes | yes | — | yes (read-only) |
| `files_write`   | yes | yes | — | —   |
| `web_search`    | yes | yes | — | yes |
| `spawn_agent`   | —   | yes | — | —   |
| `run_cmd`       | —   | yes | — | yes |

## Memory namespaces

`memory_write` always lands in the caller's own namespace.
`memory_search` takes `namespace`: `self` (default, the caller's own),
`interpreter`, `architect`, `conductor`, `reflector`.

- `interpreter`, `architect`: build-time notes from those agents.
- `conductor`: bare run facts (exit code, image, command, log tails)
  written by the runtime after each run. Not written by any LLM.
- `reflector`: the Reflector's observations and concrete fixes.
  Interpreter and Architect should search this for past runtime
  problems.

Every build's preamble includes a digest of recent notes from all four
namespaces.

File tools canonicalize paths; anything outside the project root or
the source `.hal` file is rejected.

`spawn_agent` launches a stateless worker agent with a focused prompt
and an optional JSON schema for its return value. Workers have no
memory access and cannot spawn further agents.

The Reflector's `files_read` is read-only: artifacts, `runtime.yaml`,
the run's `stdout.log` / `stderr.log` / `meta.json`, and the source
`.hal`. It has no `files_write` and no `spawn_agent`.

`run_cmd` runs a shell command inside a one-shot Docker container
using the image of the caller's choice. The container is created with
the same permissive mounts as the real runtime (host cwd and
`artifacts/` both bind-mounted RW at their real paths), so probes
like `ls`, `which`, `cat`, `python3 --version`, `apt list --installed`
reflect reality. Output is returned as combined stdout+stderr (prefix
`O ` / `E ` per line) clipped to 16 KiB, with the exit code. Default
timeout is 30s, hard cap 120s. This is the "verify, don't guess"
tool — prefer a cheap `run_cmd` over an expensive wrong assumption.
