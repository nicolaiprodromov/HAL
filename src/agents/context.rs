//! Shared contextual preamble helpers for every agent.
//!
//! The HAL pipeline used to be blind: each agent started from scratch
//! each turn, with no knowledge of the host environment, no knowledge
//! of what prior phases decided, and no automatic recall of past
//! reflections. The Reflector wrote notes into memory that no
//! subsequent Interpreter/Architect run was guaranteed to read — the
//! memory loop depended entirely on the LLM spontaneously choosing to
//! call `memory_search`.
//!
//! This module fixes that by providing two pieces every agent splices
//! into its preamble:
//!
//! * [`env_block`] — host OS, arch, container runtime, default image.
//! * [`memory_digest`] — the top-k most recent notes from *all three*
//!   agent namespaces (Interpreter, Architect, Conductor/Reflector),
//!   rendered as markdown. This closes the read side of the memory
//!   loop: reflection notes reach the next build automatically.

use std::sync::{Arc, OnceLock};

use crate::events::Agent;
use crate::memory::Memory;

/// Maximum characters from a single memory note to include in the
/// digest before clipping. Keeps the preamble bounded for long notes.
const NOTE_CLIP: usize = 800;

/// One-shot probe of the host environment. Cheap enough to call per
/// build, but cached anyway so we don't re-fork `docker --version`
/// on every agent.
pub fn env_block() -> &'static str {
    static CELL: OnceLock<String> = OnceLock::new();
    CELL.get_or_init(|| {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let family = std::env::consts::FAMILY;
        let docker = probe_cmd_version(&["docker", "--version"])
            .unwrap_or_else(|| "not detected on PATH".to_string());
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<unknown>".into());
        format!(
            "## Host environment\n\
             - os / family / arch: `{os}` / `{family}` / `{arch}`\n\
             - container runtime: {docker}\n\
             - HAL default image: `{img}`\n\
             - HAL invoked from: `{cwd}`\n\
             - artifacts mount point inside container: `/work` (env `HAL_ARTIFACTS`)\n\
             - host cwd mirroring: the user's invocation directory is bind-mounted \
               RW at the same absolute path inside the container AND is the \
               container's default working directory, so relative arguments and \
               sibling files resolve exactly as they would on the host. The path \
               is also exposed as env `HAL_HOST_CWD`.\n\
             - workspace mirroring (source's parent dir): enabled by default, \
               skipped automatically when it's already covered by the host cwd \
               mount (the common case).\n",
            img = crate::config::DEFAULT_IMAGE,
        )
    })
    .as_str()
}

fn probe_cmd_version(cmd: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Render a "## Prior project memory" section pulling the `k` most
/// recent notes from every namespace. Returns an empty string when the
/// project has no notes yet (first-ever build) so we don't pollute the
/// preamble with placeholders.
///
/// This is what closes the memory loop: Reflector-written notes (in
/// the Conductor namespace) are now automatically visible to the
/// Interpreter and Architect at the very next build, without the LLM
/// needing to remember to call `memory_search` first.
pub async fn memory_digest(memory: &Arc<Memory>, k: usize) -> String {
    let k = k.max(1);
    let namespaces = [
        (Agent::Interpreter, "Interpreter (intent / logic decisions)"),
        (Agent::Architect, "Architect (build / artifact decisions)"),
        (
            Agent::Conductor,
            "Conductor & Reflector (runtime observations)",
        ),
    ];

    let mut sections: Vec<String> = Vec::new();
    for (agent, label) in namespaces {
        // Empty query → recency fallback for both vector and no-vector paths.
        let hits = memory.search(agent, "", k).await.unwrap_or_default();
        if hits.is_empty() {
            continue;
        }
        let mut section = String::new();
        section.push_str(&format!("### {label}\n"));
        for h in hits {
            let content = clip(&h.content, NOTE_CLIP);
            section.push_str(&format!("- (id {}) {}\n", h.id, one_line_preview(&content)));
        }
        sections.push(section);
    }

    if sections.is_empty() {
        return String::new();
    }

    let mut out = String::from(
        "## Prior project memory (auto-recalled)\n\
         These are durable notes from previous builds and runs of THIS\n\
         project, pre-fetched for you. Treat them as ground truth about\n\
         what has been tried before, what worked, and what failed. You\n\
         do not need to `memory_search` to discover them; use\n\
         `memory_search` only for targeted lookups beyond this digest.\n\n",
    );
    for s in sections {
        out.push_str(&s);
        out.push('\n');
    }
    out
}

/// Compose an optional block describing the runtime arguments the user
/// will pass at `hal run` time. Empty string when no args.
pub fn runtime_args_block(args: &[String]) -> String {
    if args.is_empty() {
        return String::new();
    }
    let rendered = args
        .iter()
        .map(|a| format!("`{a}`"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "## Runtime arguments\n\
         The user will invoke this build as `hal run <source> {rendered}`.\n\
         Those tokens are forwarded verbatim to the entrypoint inside the\n\
         container. Plan the artifacts and `runtime.yaml` so the program\n\
         consumes them correctly (e.g. via `$@`, `sys.argv[1:]`, etc.).\n\n"
    )
}

/// Clip a string to at most `max` characters, appending an ellipsis
/// marker when truncation happens.
pub fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [{} bytes clipped]", &s[..end], s.len() - end)
}

fn one_line_preview(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    clip(&flat, 280)
}
