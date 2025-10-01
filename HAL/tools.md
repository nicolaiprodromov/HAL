# Tools

All three agents share a tool surface. Prompts scope who can call what.

| Tool | Interpreter | Architect | Conductor (Reflector) |
|---|:-:|:-:|:-:|
| `memory_search` | yes | yes | yes |
| `memory_write`  | yes | yes | yes |
| `files_read`    | yes | yes | yes |
| `files_write`   | yes | yes | —   |
| `web_search`    | yes | yes | yes |
| `spawn_agent`   | —   | yes | —   |
| `run_cmd`       | —   | yes | yes |

File tools canonicalize paths; anything outside the project root or
the source `.hal` file is rejected.

`spawn_agent` launches a stateless worker agent with a focused prompt
and an optional JSON schema for its return value. Workers have no
memory access and cannot spawn further agents.

`run_cmd` runs a shell command inside a one-shot Docker container
using the image of the caller's choice. The container is created with
the same permissive mounts as the real runtime (host cwd and
`artifacts/` both bind-mounted RW at their real paths), so probes
like `ls`, `which`, `cat`, `python3 --version`, `apt list --installed`
reflect reality. Output is returned as combined stdout+stderr (prefix
`O ` / `E ` per line) clipped to 16 KiB, with the exit code. Default
timeout is 30s, hard cap 120s. This is the "verify, don't guess"
tool — prefer a cheap `run_cmd` over an expensive wrong assumption.
