//! Ratatui dashboard for `hal build` / `hal run`.
//!
//! Layout:
//!
//! ```text
//! ┌── HAL ─────────────────────────────────────────────────────────┐
//! │ Interpret ✓ 3.2s  │  Build ▶ 1.4s  │  Run ·  │  Reflect ·      │
//! ├──────────────────────────────────┬─────────────────────────────┤
//! │ Compiler                         │ Output                      │
//! │   — agent stream / tool cards    │   — container stdout/stderr │
//! │   — collapsible entries          │                             │
//! ├──────────────────────────────────┴─────────────────────────────┤
//! │ (only when paused) Chatbox → steer Interpreter/Architect/…     │
//! └────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Features:
//! - Space toggles pause. Pause halts agent streaming, pipeline phase
//!   transitions, and tool bookkeeping. It does NOT halt the container
//!   runtime.
//! - When paused, a multi-line chatbox appears. Shift+Enter = newline,
//!   Enter = send. `@path` tokens expand to full file contents before
//!   being delivered as steering to the chosen agent.
//! - `m` toggles mouse capture (off = native terminal selection / copy).
//!   Capture auto-disables on pause so the user can select text.
//! - `[` collapses all timeline entries; `]` expands all; `Enter` on a
//!   focused entry toggles it. `J`/`K` move the focus cursor.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CtEvent, KeyCode, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, MouseEvent, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use tokio::sync::broadcast::Receiver;

use crate::control::PauseController;
use crate::events::{Agent, Event, EventSender, Phase};
use crate::project::Project;
use crate::tui::ansi::AnsiParser;
use crate::tui::theme;

/// Handles needed by the TUI to drive post-pipeline chat turns and
/// slash commands (`/interpret`, `/build`, `/run`). Optional — callers
/// that don't want an interactive session after the pipeline completes
/// can pass `None` and the chatbox will stay in pause-only mode.
#[derive(Clone)]
pub struct ChatRuntime {
    pub project: Arc<Project>,
    pub memory: Arc<crate::memory::Memory>,
    pub llm: Arc<crate::llm::LlmClient>,
    pub config: crate::config::Config,
    pub runtime_args: Vec<String>,
    /// When true, the `/run` command is available (we have a container
    /// runtime to invoke). `hal build` sets this to false.
    pub can_run: bool,
}

// ──────────────────────────────────────────────────────────────────────
// state
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseState {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy)]
struct PhaseTrack {
    phase: Phase,
    label: &'static str,
    state: PhaseState,
    started: Option<Instant>,
    finished: Option<Instant>,
}

impl PhaseTrack {
    fn new(phase: Phase, label: &'static str) -> Self {
        Self {
            phase,
            label,
            state: PhaseState::Pending,
            started: None,
            finished: None,
        }
    }

    fn elapsed(&self, now: Instant) -> Option<Duration> {
        match (self.started, self.finished) {
            (Some(s), Some(f)) => Some(f.saturating_duration_since(s)),
            (Some(s), None) => Some(now.saturating_duration_since(s)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
enum EntryKind {
    PhaseBanner {
        phase: Phase,
    },
    AgentLog {
        agent: Agent,
        message: String,
    },
    AgentBubble {
        agent: Agent,
        text: String,
        closed: bool,
    },
    AgentResponse {
        agent: Agent,
        text: String,
    },
    ToolCall {
        agent: Agent,
        tool: String,
        args: String,
        result: Option<(bool, String, u64)>,
    },
    RunAction {
        image: String,
        command: String,
        cwd: Option<String>,
        args: Vec<String>,
    },
    Paused,
    Resumed,
    Steering {
        agent: Agent,
        message: String,
    },
    Failed(String),
    Done(i32),
}

impl EntryKind {
    /// Whether a collapse/expand toggle actually changes rendering for
    /// this entry. Terse entries (banners, logs, pause markers) are
    /// always rendered in their compact form.
    fn supports_collapse(&self) -> bool {
        matches!(
            self,
            EntryKind::AgentBubble { .. }
                | EntryKind::AgentResponse { .. }
                | EntryKind::ToolCall { .. }
                | EntryKind::RunAction { .. }
                | EntryKind::Steering { .. }
        )
    }
}

#[derive(Debug, Clone)]
struct TimelineEntry {
    kind: EntryKind,
    collapsed: bool,
}

impl TimelineEntry {
    fn new(kind: EntryKind) -> Self {
        Self {
            kind,
            collapsed: false,
        }
    }
}

#[derive(Debug, Default)]
struct PaneScroll {
    offset: u16,
    paused: bool,
}

impl PaneScroll {
    fn page_up(&mut self, n: u16) {
        self.offset = self.offset.saturating_add(n);
        self.paused = true;
    }
    fn page_down(&mut self, n: u16) {
        self.offset = self.offset.saturating_sub(n);
        if self.offset == 0 {
            self.paused = false;
        }
    }
    fn jump_top(&mut self) {
        self.offset = u16::MAX;
        self.paused = true;
    }
    fn jump_bottom(&mut self) {
        self.offset = 0;
        self.paused = false;
    }
}

const LINE_STEP: u16 = 3;
const WHEEL_STEP: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Transcript,
    Chat,
}

#[derive(Debug, Default, Clone)]
struct Chatbox {
    /// One line per entry; rendered as a multi-line editor.
    lines: Vec<String>,
    /// Cursor line index.
    cy: usize,
    /// Cursor column (byte index into the current line).
    cx: usize,
    /// Which agent the next send targets.
    target: Option<Agent>,
}

impl Chatbox {
    fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cy: 0,
            cx: 0,
            target: None,
        }
    }
    fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cy = 0;
        self.cx = 0;
    }
    fn joined(&self) -> String {
        self.lines.join("\n")
    }
    fn is_empty(&self) -> bool {
        self.joined().trim().is_empty()
    }
    fn insert_char(&mut self, ch: char) {
        let line = &mut self.lines[self.cy];
        line.insert(self.cx, ch);
        self.cx += ch.len_utf8();
    }
    fn backspace(&mut self) {
        if self.cx > 0 {
            let line = &mut self.lines[self.cy];
            // Find prev char boundary.
            let mut new_cx = self.cx - 1;
            while new_cx > 0 && !line.is_char_boundary(new_cx) {
                new_cx -= 1;
            }
            line.replace_range(new_cx..self.cx, "");
            self.cx = new_cx;
        } else if self.cy > 0 {
            let cur = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.lines[self.cy].len();
            self.lines[self.cy].push_str(&cur);
        }
    }
    fn newline(&mut self) {
        let tail = self.lines[self.cy].split_off(self.cx);
        self.cy += 1;
        self.lines.insert(self.cy, tail);
        self.cx = 0;
    }
    fn move_left(&mut self) {
        if self.cx > 0 {
            let line = &self.lines[self.cy];
            let mut new_cx = self.cx - 1;
            while new_cx > 0 && !line.is_char_boundary(new_cx) {
                new_cx -= 1;
            }
            self.cx = new_cx;
        } else if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.lines[self.cy].len();
        }
    }
    fn move_right(&mut self) {
        let line = &self.lines[self.cy];
        if self.cx < line.len() {
            let mut new_cx = self.cx + 1;
            while new_cx < line.len() && !line.is_char_boundary(new_cx) {
                new_cx += 1;
            }
            self.cx = new_cx;
        } else if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = 0;
        }
    }
    fn move_up(&mut self) {
        if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.cx.min(self.lines[self.cy].len());
        }
    }
    fn move_down(&mut self) {
        if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = self.cx.min(self.lines[self.cy].len());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    /// `@path` file attach picker.
    File,
    /// `/cmd` slash-command picker.
    Command,
}

/// Popup shown above the chatbox while the user is typing `@<query>`
/// or `/<query>`. Offers a list of matches filterable by typed text;
/// Arrow keys move the highlight, Tab inserts the selection, Esc
/// closes the picker (without replacing the typed token).
#[derive(Debug, Clone)]
struct Picker {
    kind: PickerKind,
    /// Everything the user typed after the trigger character so far
    /// (not including the `@` / `/` itself).
    query: String,
    /// Candidate items matched against `query`.
    items: Vec<PickerItem>,
    /// Currently-highlighted index into `items`.
    cursor: usize,
    /// Absolute (cy, cx) of the `@` / `/` sigil in the chatbox at the
    /// time the picker opened. Used to replace the token on accept.
    anchor: (usize, usize),
}

#[derive(Debug, Clone)]
struct PickerItem {
    /// Text inserted into the chatbox when the user accepts this item
    /// (not including the leading sigil).
    insert: String,
    /// Secondary descriptor shown dim next to the label.
    hint: String,
}

/// Which left-pane view is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeftView {
    /// Live agent timeline — the default.
    Compiler,
    /// Project artifacts browser — file/folder tree rooted at the
    /// project source directory.
    Artifacts,
}

/// A single flattened, currently-visible row in the artifacts tree.
#[derive(Debug, Clone)]
struct ArtifactRow {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    name: String,
}

/// State for the artifacts browser (tree + optional file viewer).
#[derive(Debug)]
struct ArtifactsState {
    root: PathBuf,
    rows: Vec<ArtifactRow>,
    cursor: usize,
    scroll: usize,
    expanded: std::collections::HashSet<PathBuf>,
    viewing: Option<ArtifactViewer>,
}

/// Inline file viewer opened by pressing Enter on a file in the tree.
#[derive(Debug, Clone)]
struct ArtifactViewer {
    path: PathBuf,
    lines: Vec<String>,
    scroll: usize,
    error: Option<String>,
}

impl ArtifactsState {
    fn new(root: PathBuf) -> Self {
        let mut s = Self {
            root,
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
            expanded: std::collections::HashSet::new(),
            viewing: None,
        };
        s.rebuild();
        s
    }

    /// Re-walk the tree from `root`, honoring `expanded`, producing the
    /// flat `rows` list.
    fn rebuild(&mut self) {
        self.rows.clear();
        // Always include the root dir as an implicit expanded node;
        // children are shown unconditionally at depth 0.
        walk_dir_into(&self.root, 0, &self.expanded, &mut self.rows);
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        let c = self.cursor as isize + delta;
        self.cursor = c.clamp(0, n - 1) as usize;
    }

    fn toggle_expand(&mut self) {
        let Some(row) = self.rows.get(self.cursor).cloned() else {
            return;
        };
        if !row.is_dir {
            return;
        }
        if self.expanded.contains(&row.path) {
            self.expanded.remove(&row.path);
        } else {
            self.expanded.insert(row.path.clone());
        }
        self.rebuild();
        // Keep cursor on the same directory entry.
        if let Some(i) = self.rows.iter().position(|r| r.path == row.path) {
            self.cursor = i;
        }
    }

    fn open_selected(&mut self) {
        let Some(row) = self.rows.get(self.cursor).cloned() else {
            return;
        };
        if row.is_dir {
            self.toggle_expand();
            return;
        }
        // Load file content (truncated to a reasonable size so we don't
        // blow up RAM on a 200 MB log).
        const MAX_BYTES: u64 = 512 * 1024;
        let meta = std::fs::metadata(&row.path).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let (lines, error) = match std::fs::read(&row.path) {
            Ok(bytes) => {
                let bytes = if size > MAX_BYTES {
                    &bytes[..MAX_BYTES as usize]
                } else {
                    &bytes[..]
                };
                match std::str::from_utf8(bytes) {
                    Ok(s) => {
                        let lines: Vec<String> = s.split('\n').map(|l| l.to_string()).collect();
                        let err = if size > MAX_BYTES {
                            Some(format!(
                                "(file truncated at {} KiB; full size {} bytes)",
                                MAX_BYTES / 1024,
                                size
                            ))
                        } else {
                            None
                        };
                        (lines, err)
                    }
                    Err(_) => (
                        vec![format!("<binary file · {} bytes>", size)],
                        Some("not valid UTF-8".into()),
                    ),
                }
            }
            Err(e) => (Vec::new(), Some(format!("read error: {}", e))),
        };
        self.viewing = Some(ArtifactViewer {
            path: row.path,
            lines,
            scroll: 0,
            error,
        });
    }
}

/// Walk `dir` and push visible children into `out`. Recurses into any
/// sub-directory whose absolute path is in `expanded`.
fn walk_dir_into(
    dir: &std::path::Path,
    depth: usize,
    expanded: &std::collections::HashSet<PathBuf>,
    out: &mut Vec<ArtifactRow>,
) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        // Directories first, then case-insensitive name.
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        (!is_dir, name.to_lowercase())
    });
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        // Skip VCS / build noise at any depth.
        if matches!(
            name.as_str(),
            ".git" | "target" | "node_modules" | "__pycache__" | ".venv"
        ) {
            continue;
        }
        let path = e.path();
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push(ArtifactRow {
            path: path.clone(),
            depth,
            is_dir,
            name,
        });
        if is_dir && expanded.contains(&path) {
            walk_dir_into(&path, depth + 1, expanded, out);
        }
    }
}

struct AppState {
    timeline: Vec<TimelineEntry>,
    call_index: HashMap<u64, usize>,
    open_bubble: HashMap<Agent, usize>,
    thinking: std::collections::HashSet<Agent>,
    output_lines: Vec<Line<'static>>,
    out_ansi: AnsiParser,
    err_ansi: AnsiParser,
    phases: [PhaseTrack; 4],
    current_phase: Option<usize>,
    exit_code: Option<i32>,
    failed: Option<String>,
    compiler_scroll: PaneScroll,
    output_scroll: PaneScroll,
    split_pct: u16,
    last_compiler_area: Rect,
    last_output_area: Rect,
    last_chatbox_area: Rect,
    dirty: bool,
    tick: u64,
    started: Instant,

    // New: pause / chat / focus / mouse.
    paused: bool,
    pause_ctrl: Option<Arc<PauseController>>,
    event_tx: Option<EventSender>,
    project: Option<Arc<Project>>,
    mode: Mode,
    chat: Chatbox,
    focus: Option<usize>,
    mouse_captured: bool,

    // Post-pipeline chat mode.
    runtime: Option<ChatRuntime>,
    /// Pipeline has finished (Done or Failed) at least once.
    pipeline_settled: bool,
    /// A chat turn or slash-command-triggered pipeline task is running
    /// in the background; block the user from spamming new ones.
    chat_busy: bool,
    /// Running transcript of post-pipeline chat exchanges.
    chat_history: Vec<crate::agents::chat::ChatTurn>,
    /// Popup state for `@path` file picker and `/cmd` command picker.
    picker: Option<Picker>,
    /// Set by `/quit` — the main loop exits on the next iteration.
    quit_requested: bool,
    /// Which view is shown in the left pane (Compiler timeline or
    /// Artifacts tree). Toggled with Ctrl+O.
    left_view: LeftView,
    /// Lazily-initialized artifacts browser state.
    artifacts: Option<ArtifactsState>,
}

impl AppState {
    fn new(
        pause: Option<Arc<PauseController>>,
        tx: Option<EventSender>,
        project: Option<Arc<Project>>,
    ) -> Self {
        Self {
            timeline: Vec::new(),
            call_index: HashMap::new(),
            open_bubble: HashMap::new(),
            thinking: std::collections::HashSet::new(),
            output_lines: Vec::new(),
            out_ansi: AnsiParser::new(Style::default().fg(Color::White)),
            err_ansi: AnsiParser::new(Style::default().fg(Color::Red)),
            phases: [
                PhaseTrack::new(Phase::Interpreting, "Interpret"),
                PhaseTrack::new(Phase::Building, "Build"),
                PhaseTrack::new(Phase::Running, "Run"),
                PhaseTrack::new(Phase::Reflecting, "Reflect"),
            ],
            current_phase: None,
            exit_code: None,
            failed: None,
            compiler_scroll: PaneScroll::default(),
            output_scroll: PaneScroll::default(),
            split_pct: 60,
            last_compiler_area: Rect::new(0, 0, 0, 0),
            last_output_area: Rect::new(0, 0, 0, 0),
            last_chatbox_area: Rect::new(0, 0, 0, 0),
            dirty: true,
            tick: 0,
            started: Instant::now(),
            paused: false,
            pause_ctrl: pause,
            event_tx: tx,
            project,
            mode: Mode::Transcript,
            chat: Chatbox::new(),
            focus: None,
            mouse_captured: true,
            runtime: None,
            pipeline_settled: false,
            chat_busy: false,
            chat_history: Vec::new(),
            picker: None,
            quit_requested: false,
            left_view: LeftView::Compiler,
            artifacts: None,
        }
    }

    fn push_output(&mut self, text: String, err: bool) {
        let parser = if err {
            &mut self.err_ansi
        } else {
            &mut self.out_ansi
        };
        for chunk in text.split_inclusive('\n') {
            let trimmed = chunk.trim_end_matches('\n');
            self.output_lines.push(parser.line(trimmed));
        }
        if self.output_lines.len() > 5000 {
            let n = self.output_lines.len() - 5000;
            self.output_lines.drain(0..n);
        }
    }

    fn enter_phase(&mut self, p: Phase) {
        let now = Instant::now();
        if let Some(idx) = self.phases.iter().position(|t| t.phase == p) {
            for (i, t) in self.phases.iter_mut().enumerate() {
                if i < idx && matches!(t.state, PhaseState::Running) {
                    t.state = PhaseState::Done;
                    t.finished = Some(now);
                }
            }
            let track = &mut self.phases[idx];
            track.state = PhaseState::Running;
            if track.started.is_none() {
                track.started = Some(now);
            }
            track.finished = None;
            self.current_phase = Some(idx);
        }
        self.timeline
            .push(TimelineEntry::new(EntryKind::PhaseBanner { phase: p }));
    }

    fn finish_all_phases(&mut self, ok: bool) {
        let now = Instant::now();
        for t in &mut self.phases {
            if matches!(t.state, PhaseState::Running) {
                t.state = if ok {
                    PhaseState::Done
                } else {
                    PhaseState::Failed
                };
                t.finished = Some(now);
            }
        }
    }

    fn steerable_agent(&self) -> Agent {
        if let Some(a) = self.chat.target {
            return a;
        }
        // Default: the agent currently running, or Interpreter.
        if let Some(a) = self.thinking.iter().next().copied() {
            return a;
        }
        match self.current_phase.map(|i| self.phases[i].phase) {
            Some(Phase::Interpreting) => Agent::Interpreter,
            Some(Phase::Building) => Agent::Architect,
            Some(Phase::Reflecting) => Agent::Reflector,
            _ => Agent::Interpreter,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// event loop
// ──────────────────────────────────────────────────────────────────────

static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);
static KBD_ENHANCED: AtomicBool = AtomicBool::new(false);
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Ask the running TUI to close (from a signal handler). The event loop
/// polls this, so the terminal is restored by the TUI's own exit path.
pub fn request_quit() {
    QUIT_REQUESTED.store(true, Ordering::SeqCst);
}

/// Put the terminal back the way the shell had it: cooked mode, main
/// screen, no mouse capture, visible cursor. Idempotent and best-effort
/// (a vanished terminal just makes the writes fail), so it is safe to
/// call from a panic hook, a signal handler and `Drop` alike.
pub fn restore_terminal() {
    if !TUI_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = io::stdout();
    if KBD_ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        out,
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = disable_raw_mode();
}

/// Restores the terminal when `run_tui` ends by any route, including `?`.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

pub async fn run_tui(
    mut rx: Receiver<Event>,
    pause: Option<Arc<PauseController>>,
    tx: EventSender,
    project: Arc<Project>,
    runtime: Option<ChatRuntime>,
) -> Result<i32> {
    enable_raw_mode()?;
    TUI_ACTIVE.store(true, Ordering::SeqCst);
    let _terminal_guard = TerminalGuard;
    // A panic must not leave the shell in raw mode on the alt screen with
    // its message unreadable: restore first, then print it.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        prev_hook(info);
    }));
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    // Ask the terminal to disambiguate modified Enter (and other) keys
    // so we can distinguish Shift+Enter from bare Enter. Silently
    // ignored on terminals that don't support the kitty protocol — we
    // accept Alt+Enter as a fallback newline binding in the chatbox.
    let kbd_enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if kbd_enhanced {
        KBD_ENHANCED.store(true, Ordering::SeqCst);
        // DISAMBIGUATE_ESCAPE_CODES is enough to distinguish Shift+Enter
        // from Enter. We deliberately avoid REPORT_ALL_KEYS_AS_ESCAPE_CODES
        // because in kitty-protocol terminals it causes printable keys to
        // arrive as their unshifted base char (e.g. Shift+2 → '2'+SHIFT
        // instead of '@'), which breaks typing @, !, #, etc.
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    let mut state = AppState::new(pause, Some(tx), Some(project));
    state.runtime = runtime;
    let mut final_exit: i32 = 0;
    let mut last_draw = Instant::now();

    'outer: loop {
        if state.quit_requested || QUIT_REQUESTED.load(Ordering::SeqCst) {
            final_exit = state.exit_code.unwrap_or(0);
            break;
        }
        loop {
            match rx.try_recv() {
                Ok(evt) => {
                    handle_event(&mut state, evt);
                    state.dirty = true;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    if let Some(c) = state.exit_code {
                        final_exit = c;
                    }
                    break 'outer;
                }
            }
        }

        let animating = !state.thinking.is_empty();
        let now = Instant::now();
        let force_anim = animating && now.duration_since(last_draw) >= Duration::from_millis(33);
        if state.dirty || force_anim {
            state.tick = state.tick.wrapping_add(1);
            term.draw(|f| render(f, &mut state))?;
            state.dirty = false;
            last_draw = now;
        }

        let pipeline_done = state.exit_code.is_some() || state.failed.is_some();
        let poll_ms = if pipeline_done { 60 } else { 10 };
        if event::poll(Duration::from_millis(poll_ms))? {
            match event::read()? {
                CtEvent::Key(k) if k.kind == KeyEventKind::Press => {
                    if handle_key(&mut state, &mut term, k)? {
                        final_exit = state.exit_code.unwrap_or(final_exit);
                        break;
                    }
                    state.dirty = true;
                }
                CtEvent::Mouse(m) => {
                    if state.mouse_captured {
                        handle_mouse(&mut state, m);
                        state.dirty = true;
                    }
                }
                CtEvent::Resize(_, _) => {
                    state.dirty = true;
                }
                _ => {}
            }
        }
    }

    restore_terminal();
    Ok(final_exit)
}

fn page_size(state: &AppState, output: bool) -> u16 {
    let area = if output {
        state.last_output_area
    } else {
        state.last_compiler_area
    };
    let inner = area.height.saturating_sub(2);
    inner.max(1)
}

fn handle_mouse(state: &mut AppState, m: MouseEvent) {
    match m.kind {
        MouseEventKind::ScrollUp => {
            if in_rect(m.column, m.row, state.last_output_area) {
                state.output_scroll.page_up(WHEEL_STEP);
            } else {
                state.compiler_scroll.page_up(WHEEL_STEP);
            }
        }
        MouseEventKind::ScrollDown => {
            if in_rect(m.column, m.row, state.last_output_area) {
                state.output_scroll.page_down(WHEEL_STEP);
            } else {
                state.compiler_scroll.page_down(WHEEL_STEP);
            }
        }
        _ => {}
    }
}

fn in_rect(x: u16, y: u16, r: Rect) -> bool {
    x >= r.x && x < r.x.saturating_add(r.width) && y >= r.y && y < r.y.saturating_add(r.height)
}

fn set_mouse_capture<B: std::io::Write>(
    term: &mut Terminal<CrosstermBackend<B>>,
    on: bool,
) -> Result<()> {
    if on {
        execute!(term.backend_mut(), EnableMouseCapture)?;
    } else {
        execute!(term.backend_mut(), DisableMouseCapture)?;
    }
    Ok(())
}

fn handle_key<B: std::io::Write>(
    state: &mut AppState,
    term: &mut Terminal<CrosstermBackend<B>>,
    k: crossterm::event::KeyEvent,
) -> Result<bool> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let ctrl_c = ctrl && matches!(k.code, KeyCode::Char('c'));

    // Chat-mode keybindings take precedence.
    if state.mode == Mode::Chat {
        // Picker intercepts navigation keys while open.
        if state.picker.is_some() {
            match k.code {
                KeyCode::Esc => {
                    state.picker = None;
                    return Ok(false);
                }
                KeyCode::Up => {
                    if let Some(p) = state.picker.as_mut() {
                        if !p.items.is_empty() {
                            p.cursor = if p.cursor == 0 {
                                p.items.len() - 1
                            } else {
                                p.cursor - 1
                            };
                        }
                    }
                    return Ok(false);
                }
                KeyCode::Down => {
                    if let Some(p) = state.picker.as_mut() {
                        if !p.items.is_empty() {
                            p.cursor = (p.cursor + 1) % p.items.len();
                        }
                    }
                    return Ok(false);
                }
                KeyCode::Tab | KeyCode::Enter => {
                    accept_picker(state);
                    return Ok(false);
                }
                _ => { /* fall through to normal chat editing */ }
            }
        }

        match k.code {
            KeyCode::Esc => {
                // Esc always defocuses the chatbox and drops back to
                // transcript mode so the rest of the shortcuts become
                // available again. In live post-pipeline mode the
                // chatbox stays *visible* (see render()) — it just
                // loses input focus. Press `i` to re-focus it.
                if state.picker.is_some() {
                    state.picker = None;
                } else {
                    state.mode = Mode::Transcript;
                }
                return Ok(false);
            }
            // Shift+Enter or Alt+Enter → newline. Plain Enter → send.
            KeyCode::Enter if shift || alt => {
                state.chat.newline();
                refresh_picker_on_edit(state);
                return Ok(false);
            }
            KeyCode::Enter => {
                if !state.chat.is_empty() {
                    submit_chat(state);
                }
                if !state.live_chat_active() {
                    state.mode = Mode::Transcript;
                }
                return Ok(false);
            }
            KeyCode::Tab => {
                // Cycle target agent.
                let cur = state.steerable_agent();
                let next = next_agent(cur);
                state.chat.target = Some(next);
                return Ok(false);
            }
            KeyCode::Backspace => {
                state.chat.backspace();
                refresh_picker_on_edit(state);
                return Ok(false);
            }
            KeyCode::Left => {
                state.chat.move_left();
                return Ok(false);
            }
            KeyCode::Right => {
                state.chat.move_right();
                return Ok(false);
            }
            KeyCode::Up => {
                state.chat.move_up();
                return Ok(false);
            }
            KeyCode::Down => {
                state.chat.move_down();
                return Ok(false);
            }
            KeyCode::Char('c') if ctrl => return Ok(true),
            KeyCode::Char(c) => {
                state.chat.insert_char(c);
                refresh_picker_on_edit(state);
                // Opening a picker on `@` or `/` at the start of a line
                // / after whitespace.
                maybe_open_picker(state, c);
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    // Transcript mode.
    // Ctrl+O toggles the left pane between the compiler timeline and
    // the artifacts tree. Works from transcript mode only — in chat
    // mode the chatbox owns the keyboard.
    if ctrl && matches!(k.code, KeyCode::Char('o')) {
        toggle_left_view(state);
        return Ok(false);
    }

    // When the artifacts view is active, it captures navigation keys
    // before the transcript bindings do.
    if state.left_view == LeftView::Artifacts && handle_artifacts_key(state, k) {
        return Ok(false);
    }

    if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) || ctrl_c {
        return Ok(true);
    }

    match k.code {
        KeyCode::Char(' ') => {
            toggle_pause(state, term)?;
            return Ok(false);
        }
        KeyCode::Char('m') => {
            state.mouse_captured = !state.mouse_captured;
            set_mouse_capture(term, state.mouse_captured)?;
            return Ok(false);
        }
        KeyCode::Char('i') if state.paused || state.live_chat_active() => {
            state.mode = Mode::Chat;
            return Ok(false);
        }
        KeyCode::Char('[') => {
            set_all_collapsed(state, true);
            return Ok(false);
        }
        KeyCode::Char(']') => {
            set_all_collapsed(state, false);
            return Ok(false);
        }
        KeyCode::Char('J') => {
            move_focus(state, 1);
            return Ok(false);
        }
        KeyCode::Char('K') => {
            move_focus(state, -1);
            return Ok(false);
        }
        KeyCode::Enter => {
            if let Some(i) = state.focus {
                if let Some(e) = state.timeline.get_mut(i) {
                    if e.kind.supports_collapse() {
                        e.collapsed = !e.collapsed;
                    }
                }
            }
            return Ok(false);
        }
        _ => {}
    }

    let step = if ctrl {
        page_size(state, shift) / 2
    } else {
        LINE_STEP
    };
    let page = page_size(state, shift);
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if shift {
                state.output_scroll.page_up(step);
            } else {
                state.compiler_scroll.page_up(step);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if shift {
                state.output_scroll.page_down(step);
            } else {
                state.compiler_scroll.page_down(step);
            }
        }
        KeyCode::PageUp => {
            if shift {
                state.output_scroll.page_up(page);
            } else {
                state.compiler_scroll.page_up(page);
            }
        }
        KeyCode::PageDown => {
            if shift {
                state.output_scroll.page_down(page);
            } else {
                state.compiler_scroll.page_down(page);
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            if shift {
                state.output_scroll.jump_top();
            } else {
                state.compiler_scroll.jump_top();
            }
        }
        KeyCode::End | KeyCode::Char('G') => {
            state.compiler_scroll.jump_bottom();
            state.output_scroll.jump_bottom();
        }
        KeyCode::Char('<') => state.split_pct = state.split_pct.saturating_sub(5).max(20),
        KeyCode::Char('>') => state.split_pct = (state.split_pct + 5).min(80),
        _ => {}
    }
    Ok(false)
}

fn toggle_pause<B: std::io::Write>(
    state: &mut AppState,
    term: &mut Terminal<CrosstermBackend<B>>,
) -> Result<()> {
    let Some(ctrl) = state.pause_ctrl.clone() else {
        return Ok(());
    };
    let new = ctrl.toggle();
    state.paused = new;
    if let Some(tx) = &state.event_tx {
        let _ = tx.send(if new { Event::Paused } else { Event::Resumed });
    }
    // Auto-disable mouse capture while paused so the user can select text
    // with the native terminal selection.
    if new && state.mouse_captured {
        state.mouse_captured = false;
        set_mouse_capture(term, false)?;
    } else if !new && !state.mouse_captured {
        state.mouse_captured = true;
        set_mouse_capture(term, true)?;
    }
    if !new && !state.live_chat_active() {
        state.mode = Mode::Transcript;
    }
    Ok(())
}

fn set_all_collapsed(state: &mut AppState, collapsed: bool) {
    for e in &mut state.timeline {
        if e.kind.supports_collapse() {
            e.collapsed = collapsed;
        }
    }
}

/// Switch the left pane between the live compiler timeline and the
/// artifacts browser. Lazily initializes the tree when first opened.
fn toggle_left_view(state: &mut AppState) {
    state.left_view = match state.left_view {
        LeftView::Compiler => {
            if state.artifacts.is_none() {
                // Prefer the project root (directory of the `.hal`
                // source). Fall back to the current working directory
                // for cases where no project is attached (build/check).
                let root = state
                    .project
                    .as_ref()
                    .and_then(|p| p.source.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| {
                        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                    });
                state.artifacts = Some(ArtifactsState::new(root));
            }
            LeftView::Artifacts
        }
        LeftView::Artifacts => LeftView::Compiler,
    };
}

/// Handle a key event while the artifacts view is active. Returns true
/// if the key was consumed (so the caller should not fall through to
/// the default transcript bindings).
fn handle_artifacts_key(state: &mut AppState, k: crossterm::event::KeyEvent) -> bool {
    let Some(art) = state.artifacts.as_mut() else {
        return false;
    };
    // File-viewer mode takes precedence when open.
    if let Some(view) = art.viewing.as_mut() {
        match k.code {
            KeyCode::Esc => {
                art.viewing = None;
                return true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                view.scroll = view.scroll.saturating_sub(1);
                return true;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                view.scroll = (view.scroll + 1).min(view.lines.len().saturating_sub(1));
                return true;
            }
            KeyCode::PageUp => {
                view.scroll = view.scroll.saturating_sub(10);
                return true;
            }
            KeyCode::PageDown => {
                view.scroll = (view.scroll + 10).min(view.lines.len().saturating_sub(1));
                return true;
            }
            KeyCode::Home | KeyCode::Char('g') => {
                view.scroll = 0;
                return true;
            }
            KeyCode::End | KeyCode::Char('G') => {
                view.scroll = view.lines.len().saturating_sub(1);
                return true;
            }
            _ => return true, // swallow everything else in viewer
        }
    }
    // Tree mode.
    match k.code {
        KeyCode::Esc => {
            // Return to the compiler view.
            state.left_view = LeftView::Compiler;
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            art.move_cursor(-1);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            art.move_cursor(1);
            true
        }
        KeyCode::PageUp => {
            art.move_cursor(-10);
            true
        }
        KeyCode::PageDown => {
            art.move_cursor(10);
            true
        }
        KeyCode::Home => {
            art.cursor = 0;
            true
        }
        KeyCode::End => {
            art.cursor = art.rows.len().saturating_sub(1);
            true
        }
        KeyCode::Right | KeyCode::Tab => {
            // Expand the selected directory, or expand if already
            // collapsed. Tab doubles as the "open" action the user
            // mentioned.
            if let Some(row) = art.rows.get(art.cursor).cloned() {
                if row.is_dir && !art.expanded.contains(&row.path) {
                    art.expanded.insert(row.path.clone());
                    art.rebuild();
                    if let Some(i) = art.rows.iter().position(|r| r.path == row.path) {
                        art.cursor = i;
                    }
                } else if row.is_dir {
                    art.toggle_expand();
                }
            }
            true
        }
        KeyCode::Left => {
            if let Some(row) = art.rows.get(art.cursor).cloned() {
                if row.is_dir && art.expanded.contains(&row.path) {
                    art.expanded.remove(&row.path);
                    art.rebuild();
                    if let Some(i) = art.rows.iter().position(|r| r.path == row.path) {
                        art.cursor = i;
                    }
                } else if row.depth > 0 {
                    // Jump to parent directory entry.
                    if let Some(parent) = row.path.parent() {
                        if let Some(i) = art.rows.iter().position(|r| r.path == parent) {
                            art.cursor = i;
                        }
                    }
                }
            }
            true
        }
        KeyCode::Enter => {
            art.open_selected();
            true
        }
        _ => false,
    }
}

fn move_focus(state: &mut AppState, delta: isize) {
    if state.timeline.is_empty() {
        state.focus = None;
        return;
    }
    let len = state.timeline.len() as isize;
    let cur = state.focus.map(|i| i as isize).unwrap_or(len - 1);
    let next = (cur + delta).clamp(0, len - 1);
    state.focus = Some(next as usize);
    state.compiler_scroll.paused = true;
}

fn send_steering(state: &mut AppState) {
    let Some(ctrl) = state.pause_ctrl.clone() else {
        return;
    };
    let text = state.chat.joined();
    let expanded = expand_file_refs(&text, state.project.as_deref());
    let agent = state.steerable_agent();
    ctrl.push_steer(agent, expanded.clone());
    if let Some(tx) = &state.event_tx {
        let _ = tx.send(Event::Steering {
            agent,
            message: expanded,
        });
    }
}

// ──────────────────────────────────────────────────────────────────────
// post-pipeline chat dispatch + slash commands
// ──────────────────────────────────────────────────────────────────────

impl AppState {
    /// True once the pipeline has settled (reached Done or Failed) AND
    /// we have a chat runtime to drive further turns. Used to decide
    /// whether the chatbox is a transient steering overlay (pause-only)
    /// or a permanent post-pipeline chat surface.
    fn live_chat_active(&self) -> bool {
        self.pipeline_settled && self.runtime.is_some()
    }
}

/// Handle a submitted chatbox: slash commands spawn pipeline phases;
/// plain messages go either to the paused-agent steering queue (during
/// pipeline execution) or to the post-pipeline chat driver.
fn submit_chat(state: &mut AppState) {
    let text = state.chat.joined();
    let trimmed = text.trim().to_string();
    state.chat.clear();
    state.picker = None;
    if trimmed.is_empty() {
        return;
    }
    if let Some(rest) = trimmed.strip_prefix('/') {
        dispatch_slash(state, rest);
        return;
    }
    // Live post-pipeline mode: start a chat turn with the targeted agent.
    if state.live_chat_active() && !state.paused {
        start_chat_turn(state, &text);
        return;
    }
    // Otherwise fall back to the legacy pause+steer flow.
    send_steering(state);
}

/// Interpret a `/command <args>` line. Unknown commands surface as a
/// red timeline note so the user can tell the chat was consumed.
fn dispatch_slash(state: &mut AppState, line: &str) {
    let mut parts = line.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
    let _rest = parts.next().unwrap_or("").trim().to_string();
    match cmd.as_str() {
        "q" | "quit" | "exit" => {
            state.quit_requested = true;
        }
        "help" | "h" | "?" => {
            push_system_note(
                state,
                "slash commands: /interpret /build /run /quit · chat with Tab to pick agent · @path to attach files · Shift+Enter for newline",
            );
        }
        "interpret" => run_slash_pipeline(state, SlashStage::Interpret),
        "build" => run_slash_pipeline(state, SlashStage::Build),
        "run" => run_slash_pipeline(state, SlashStage::Run),
        other => push_system_note(state, &format!("unknown command: /{other}")),
    }
}

#[derive(Debug, Clone, Copy)]
enum SlashStage {
    /// Re-interpret (→ re-build → re-run).
    Interpret,
    /// Re-build (→ re-run), keeping existing logic.md.
    Build,
    /// Re-run the already-built bundle, then reflect again.
    Run,
}

fn run_slash_pipeline(state: &mut AppState, stage: SlashStage) {
    if state.chat_busy {
        push_system_note(state, "pipeline is already running — wait for it to finish");
        return;
    }
    let Some(rt) = state.runtime.clone() else {
        push_system_note(state, "chat runtime unavailable — cannot re-run pipeline");
        return;
    };
    let Some(tx) = state.event_tx.clone() else {
        return;
    };
    let pause = state.pause_ctrl.clone();
    // Resume if paused so the background pipeline isn't stalled.
    if let Some(p) = pause.as_ref() {
        if p.is_paused() {
            p.set_paused(false);
            state.paused = false;
            let _ = tx.send(Event::Resumed);
        }
    }
    if matches!(stage, SlashStage::Run) && !rt.can_run {
        push_system_note(state, "/run is not available in `hal build`; try /build");
        return;
    }
    let label = match stage {
        SlashStage::Interpret => "/interpret",
        SlashStage::Build => "/build",
        SlashStage::Run => "/run",
    };
    push_system_note(state, &format!("{label} → restarting pipeline…"));
    state.chat_busy = true;
    state.pipeline_settled = false;
    // Reset phase tracks for a fresh visual progression.
    for t in &mut state.phases {
        t.state = PhaseState::Pending;
        t.started = None;
        t.finished = None;
    }
    state.current_phase = None;
    state.exit_code = None;
    state.failed = None;
    let tx_done = tx.clone();
    tokio::spawn(async move {
        let res = run_slash_stage(stage, rt.clone(), tx.clone(), pause.clone()).await;
        match res {
            Ok(code) => {
                if rt.can_run {
                    let _ = tx.send(Event::Done { exit_code: code });
                }
                let _ = tx_done.send(Event::Phase(Phase::Done));
            }
            Err(e) => {
                let _ = tx_done.send(Event::Failed {
                    message: format!("{e:#}"),
                });
            }
        }
        let _ = tx_done.send(Event::AgentLog {
            agent: Agent::Conductor,
            message: "pipeline restart complete".into(),
        });
    });
}

async fn run_slash_stage(
    stage: SlashStage,
    rt: ChatRuntime,
    tx: EventSender,
    pause: Option<Arc<PauseController>>,
) -> Result<i32> {
    use crate::pipeline;
    match stage {
        SlashStage::Interpret | SlashStage::Build => {
            // Both require a fresh build. `/interpret` also wipes the
            // bundle so the Interpreter genuinely restarts from source.
            if matches!(stage, SlashStage::Interpret) {
                rt.project.invalidate_build()?;
            }
            pipeline::run_build(
                rt.project.clone(),
                rt.memory.clone(),
                rt.llm.clone(),
                &tx,
                pause.clone(),
                &rt.runtime_args,
            )
            .await?;
            if !rt.can_run {
                return Ok(0);
            }
            // Fall through into a run so /build & /interpret end with a
            // fresh container execution.
            run_and_reflect(&rt, &tx, pause).await
        }
        SlashStage::Run => run_and_reflect(&rt, &tx, pause).await,
    }
}

async fn run_and_reflect(
    rt: &ChatRuntime,
    tx: &EventSender,
    pause: Option<Arc<PauseController>>,
) -> Result<i32> {
    use crate::agents::{conductor, reflector};
    let r = conductor::run(rt.project.clone(), rt.memory.clone(), &rt.runtime_args, tx).await?;
    if rt.config.reflector_enabled {
        let summary = reflector::RunSummary {
            image: r.image.clone(),
            command: r.command.clone(),
            cwd: r.cwd.clone(),
            args: r.args.clone(),
            exit_code: r.exit_code,
            run_dir: r.run_dir.clone(),
            stdout_tail: r.stdout_tail.clone(),
            stderr_tail: r.stderr_tail.clone(),
        };
        let _ = reflector::run(
            rt.project.clone(),
            rt.memory.clone(),
            rt.llm.clone(),
            summary,
            tx,
            rt.config.reflector_max_turns,
            pause,
        )
        .await;
    }
    Ok(r.exit_code)
}

fn start_chat_turn(state: &mut AppState, user_text: &str) {
    if state.chat_busy {
        push_system_note(state, "chat busy — wait for the current turn to finish");
        return;
    }
    let Some(rt) = state.runtime.clone() else {
        send_steering(state);
        return;
    };
    let Some(tx) = state.event_tx.clone() else {
        return;
    };
    let pause = state.pause_ctrl.clone();
    let agent = state.steerable_agent();
    let expanded = expand_file_refs(user_text, state.project.as_deref());
    let history = state.chat_history.clone();
    // Echo the user's message into the timeline so they have a record of
    // what they asked.
    if let Some(ev) = state.event_tx.clone() {
        let _ = ev.send(Event::Steering {
            agent,
            message: expanded.clone(),
        });
    }
    state.chat_busy = true;
    let tx_outer = tx.clone();
    let user_for_history = expanded.clone();
    tokio::spawn(async move {
        let res = crate::agents::chat::turn(
            agent,
            rt.project.clone(),
            rt.memory.clone(),
            rt.llm.clone(),
            &tx,
            pause,
            expanded,
            &history,
        )
        .await;
        match res {
            Ok(text) => {
                let _ = tx_outer.send(Event::AgentLog {
                    agent,
                    message: format!("chat turn complete ({} chars)", text.chars().count()),
                });
                // Stash the turn on the bus as a specialised event so the
                // render loop can push it onto `chat_history`. We reuse
                // AgentLog with a sentinel prefix since Event is frozen.
                let _ = tx_outer.send(Event::AgentLog {
                    agent,
                    message: format!(
                        "__CHAT_TURN__::{}",
                        serde_json::to_string(&(agent_key(agent), user_for_history, text,))
                            .unwrap_or_default()
                    ),
                });
            }
            Err(e) => {
                let _ = tx_outer.send(Event::AgentLog {
                    agent,
                    message: format!("chat turn failed: {e:#}"),
                });
            }
        }
    });
}

fn push_system_note(state: &mut AppState, msg: &str) {
    if let Some(tx) = &state.event_tx {
        let _ = tx.send(Event::AgentLog {
            agent: Agent::Conductor,
            message: msg.to_string(),
        });
    }
}

/// Tab-cycling order: Interpreter → Architect → Conductor → Reflector.
fn next_agent(cur: Agent) -> Agent {
    match cur {
        Agent::Interpreter => Agent::Architect,
        Agent::Architect => Agent::Conductor,
        Agent::Conductor => Agent::Reflector,
        Agent::Reflector => Agent::Interpreter,
    }
}

fn agent_key(a: Agent) -> &'static str {
    match a {
        Agent::Interpreter => "interpreter",
        Agent::Architect => "architect",
        Agent::Conductor => "conductor",
        Agent::Reflector => "reflector",
    }
}

fn parse_agent_key(s: &str) -> Agent {
    match s {
        "architect" => Agent::Architect,
        "conductor" => Agent::Conductor,
        "reflector" => Agent::Reflector,
        _ => Agent::Interpreter,
    }
}

fn on_pipeline_settled(state: &mut AppState) {
    state.chat_busy = false;
    if state.pipeline_settled {
        return;
    }
    state.pipeline_settled = true;
    // Post-pipeline: if we have a chat runtime, auto-enable chat mode
    // so the user can immediately talk to an agent or run /interpret.
    if state.runtime.is_some() {
        state.mode = Mode::Chat;
    }
}

// ──────────────────────────────────────────────────────────────────────
// picker: @files and /commands
// ──────────────────────────────────────────────────────────────────────

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("interpret", "re-run Interpret → Build → Run"),
    ("build", "re-run Build (+ Run) reusing logic.md"),
    ("run", "re-run Conductor + Reflector"),
    ("help", "show available commands"),
    ("quit", "exit the TUI"),
];

/// If the cursor sits right after a freshly-typed `@` or `/` sigil (at
/// line start or after whitespace), open the corresponding picker.
fn maybe_open_picker(state: &mut AppState, ch: char) {
    if ch != '@' && ch != '/' {
        return;
    }
    // Position we just inserted at: (cy, cx - 1) is the sigil.
    let (cy, cx) = (state.chat.cy, state.chat.cx);
    if cx == 0 {
        return;
    }
    let line = &state.chat.lines[cy];
    // The char preceding the sigil must be whitespace (or line start).
    let prev_byte = line.as_bytes().get(cx.saturating_sub(2));
    let at_word_start = cx == 1
        || prev_byte
            .map(|b| (*b as char).is_whitespace())
            .unwrap_or(true);
    if !at_word_start {
        return;
    }
    let kind = if ch == '@' {
        PickerKind::File
    } else {
        PickerKind::Command
    };
    let anchor = (cy, cx - 1);
    let items = build_picker_items(state, kind, "");
    state.picker = Some(Picker {
        kind,
        query: String::new(),
        items,
        cursor: 0,
        anchor,
    });
}

/// Keep the picker's query and item list in sync with whatever the
/// user typed after the sigil. Closes the picker if the cursor moves
/// off the picker's token (e.g. whitespace typed, or sigil erased).
fn refresh_picker_on_edit(state: &mut AppState) {
    let kind = match &state.picker {
        Some(p) => p.kind,
        None => return,
    };
    let anchor = state.picker.as_ref().unwrap().anchor;
    let (cy, cx) = (state.chat.cy, state.chat.cx);
    if cy != anchor.0 || cx <= anchor.1 {
        state.picker = None;
        return;
    }
    let line = &state.chat.lines[cy];
    let sigil_idx = anchor.1;
    let expected_sigil = match kind {
        PickerKind::File => '@',
        PickerKind::Command => '/',
    };
    if !line[sigil_idx..].starts_with(expected_sigil) {
        state.picker = None;
        return;
    }
    let query_bytes = &line[sigil_idx + 1..cx];
    if query_bytes.chars().any(|c| c.is_whitespace()) {
        state.picker = None;
        return;
    }
    let query = query_bytes.to_string();
    let items = build_picker_items(state, kind, &query);
    if let Some(p) = state.picker.as_mut() {
        p.query = query;
        p.items = items;
        if p.cursor >= p.items.len() {
            p.cursor = 0;
        }
    }
}

fn build_picker_items(state: &AppState, kind: PickerKind, query: &str) -> Vec<PickerItem> {
    match kind {
        PickerKind::Command => SLASH_COMMANDS
            .iter()
            .filter(|(name, _)| name.starts_with(query))
            .map(|(name, hint)| PickerItem {
                insert: (*name).to_string(),
                hint: (*hint).to_string(),
            })
            .collect(),
        PickerKind::File => state
            .project
            .as_deref()
            .map(|proj| list_project_files(proj, query, 100))
            .unwrap_or_default(),
    }
}

fn list_project_files(project: &Project, query: &str, limit: usize) -> Vec<PickerItem> {
    let base = project
        .source
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| project.root.clone());
    let q_lower = query.to_ascii_lowercase();
    let mut hits: Vec<(String, PathBuf)> = Vec::new();
    let walker = walkdir::WalkDir::new(&base)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            // Skip obvious build / vcs / cache directories.
            !(name == ".git"
                || name == "target"
                || name == "node_modules"
                || name == ".hal"
                || name == "__pycache__"
                || name == ".venv")
        });
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(&base) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().to_string();
        if q_lower.is_empty() || rel_str.to_ascii_lowercase().contains(&q_lower) {
            hits.push((rel_str, path.to_path_buf()));
            if hits.len() >= limit * 3 {
                break;
            }
        }
    }
    // Prefer prefix matches, then shorter paths (more specific).
    hits.sort_by(|a, b| {
        let ap = a.0.to_ascii_lowercase().starts_with(&q_lower);
        let bp = b.0.to_ascii_lowercase().starts_with(&q_lower);
        bp.cmp(&ap)
            .then(a.0.len().cmp(&b.0.len()))
            .then(a.0.cmp(&b.0))
    });
    hits.into_iter()
        .take(limit)
        .map(|(rel, full)| PickerItem {
            insert: rel,
            hint: full.display().to_string(),
        })
        .collect()
}

fn accept_picker(state: &mut AppState) {
    let Some(picker) = state.picker.take() else {
        return;
    };
    let Some(item) = picker.items.get(picker.cursor).cloned() else {
        return;
    };
    // Replace text from sigil+1 .. cx with item.insert.
    let (ay, ax) = picker.anchor;
    if ay >= state.chat.lines.len() {
        return;
    }
    let line = &mut state.chat.lines[ay];
    let start = ax + 1; // keep the sigil itself
    let end = state.chat.cx.min(line.len());
    if start > end {
        return;
    }
    line.replace_range(start..end, &item.insert);
    state.chat.cx = start + item.insert.len();
    // Add a trailing space for convenience unless already there.
    if state.chat.lines[ay].as_bytes().get(state.chat.cx).copied() != Some(b' ') {
        state.chat.lines[ay].insert(state.chat.cx, ' ');
        state.chat.cx += 1;
    }
}

/// Expand `@<path>` tokens in `text` into
/// `\n<file path="…">\n<contents>\n</file>\n` blocks. Paths are resolved
/// relative to the source's parent directory first, then the project
/// root. Missing or unreadable files are left as the literal `@path`.
fn expand_file_refs(text: &str, project: Option<&Project>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut attachments: Vec<(String, String)> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '@' {
            let mut tok = String::new();
            while let Some(&nc) = chars.peek() {
                if nc.is_whitespace() || matches!(nc, '"' | '\'' | ',' | ')' | ']' | '}' | ';') {
                    break;
                }
                tok.push(nc);
                chars.next();
            }
            if tok.is_empty() {
                out.push('@');
                continue;
            }
            if let Some(proj) = project {
                match read_workspace_file(&tok, proj) {
                    Some((resolved, body)) => {
                        out.push('@');
                        out.push_str(&tok);
                        attachments.push((resolved, body));
                    }
                    None => {
                        out.push('@');
                        out.push_str(&tok);
                    }
                }
            } else {
                out.push('@');
                out.push_str(&tok);
            }
        } else {
            out.push(c);
        }
    }
    if !attachments.is_empty() {
        out.push_str("\n\n---\n## Attached files\n");
        for (path, body) in attachments {
            out.push_str(&format!("\n<file path=\"{}\">\n{}\n</file>\n", path, body));
        }
    }
    out
}

fn read_workspace_file(token: &str, project: &Project) -> Option<(String, String)> {
    let raw = PathBuf::from(token);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if raw.is_absolute() {
        candidates.push(raw.clone());
    } else {
        if let Some(parent) = project.source.parent() {
            candidates.push(parent.join(&raw));
        }
        candidates.push(project.root.join(&raw));
    }
    for c in candidates {
        if c.is_file() {
            if let Ok(body) = std::fs::read_to_string(&c) {
                return Some((c.display().to_string(), body));
            }
        }
    }
    None
}

fn handle_event(state: &mut AppState, evt: Event) {
    match evt {
        Event::Phase(p) => match p {
            Phase::Done => {
                state.finish_all_phases(true);
                on_pipeline_settled(state);
            }
            Phase::Failed => {
                state.finish_all_phases(false);
                on_pipeline_settled(state);
            }
            other => state.enter_phase(other),
        },
        Event::AgentLog { agent, message } => {
            // Specialised sentinel emitted by post-pipeline chat turns
            // so the render loop can update `chat_history` without
            // extending the wire-level Event enum.
            if let Some(rest) = message.strip_prefix("__CHAT_TURN__::") {
                if let Ok((key, user, assistant)) =
                    serde_json::from_str::<(String, String, String)>(rest)
                {
                    state.chat_history.push(crate::agents::chat::ChatTurn {
                        agent: parse_agent_key(&key),
                        user,
                        assistant,
                    });
                }
                state.chat_busy = false;
                return;
            }
            state
                .timeline
                .push(TimelineEntry::new(EntryKind::AgentLog { agent, message }));
        }
        Event::AgentResponse { agent, text } => {
            close_bubble(state, agent);
            state
                .timeline
                .push(TimelineEntry::new(EntryKind::AgentResponse { agent, text }));
        }
        Event::AgentThinkingStart { agent } => {
            state.thinking.insert(agent);
            let idx = state.timeline.len();
            state
                .timeline
                .push(TimelineEntry::new(EntryKind::AgentBubble {
                    agent,
                    text: String::new(),
                    closed: false,
                }));
            state.open_bubble.insert(agent, idx);
        }
        Event::AgentToken { agent, delta } => {
            if let Some(&idx) = state.open_bubble.get(&agent) {
                if let Some(entry) = state.timeline.get_mut(idx) {
                    if let EntryKind::AgentBubble { text, .. } = &mut entry.kind {
                        text.push_str(&delta);
                    }
                }
            } else {
                let idx = state.timeline.len();
                state
                    .timeline
                    .push(TimelineEntry::new(EntryKind::AgentBubble {
                        agent,
                        text: delta,
                        closed: false,
                    }));
                state.open_bubble.insert(agent, idx);
                state.thinking.insert(agent);
            }
        }
        Event::AgentThinkingEnd { agent } => {
            state.thinking.remove(&agent);
            close_bubble(state, agent);
        }
        Event::ToolCallStart {
            agent,
            call_id,
            tool,
            args_preview,
        } => {
            close_bubble(state, agent);
            let idx = state.timeline.len();
            state.timeline.push(TimelineEntry::new(EntryKind::ToolCall {
                agent,
                tool,
                args: args_preview,
                result: None,
            }));
            state.call_index.insert(call_id, idx);
        }
        Event::ToolCallEnd {
            call_id,
            ok,
            output_preview,
            duration_ms,
            ..
        } => {
            if let Some(&idx) = state.call_index.get(&call_id) {
                if let Some(entry) = state.timeline.get_mut(idx) {
                    if let EntryKind::ToolCall { result, .. } = &mut entry.kind {
                        *result = Some((ok, output_preview, duration_ms));
                    }
                    // Auto-collapse completed tool calls to keep the
                    // scrollback manageable. User can expand individually.
                    entry.collapsed = true;
                }
            }
        }
        Event::ConductorStdout(s) => state.push_output(s, false),
        Event::ConductorStderr(s) => state.push_output(s, true),
        Event::ConductorRunAction {
            image,
            command,
            cwd,
            args,
        } => {
            state
                .timeline
                .push(TimelineEntry::new(EntryKind::RunAction {
                    image,
                    command,
                    cwd,
                    args,
                }));
        }
        Event::Done { exit_code } => {
            state.finish_all_phases(exit_code == 0);
            state.exit_code = Some(exit_code);
            state
                .timeline
                .push(TimelineEntry::new(EntryKind::Done(exit_code)));
            on_pipeline_settled(state);
        }
        Event::Failed { message } => {
            state.finish_all_phases(false);
            state.failed = Some(message.clone());
            state
                .timeline
                .push(TimelineEntry::new(EntryKind::Failed(message)));
            on_pipeline_settled(state);
        }
        Event::Paused => {
            state.paused = true;
            state.timeline.push(TimelineEntry::new(EntryKind::Paused));
        }
        Event::Resumed => {
            state.paused = false;
            state.timeline.push(TimelineEntry::new(EntryKind::Resumed));
        }
        Event::Steering { agent, message } => {
            state
                .timeline
                .push(TimelineEntry::new(EntryKind::Steering { agent, message }));
        }
    }
}

fn close_bubble(state: &mut AppState, agent: Agent) {
    if let Some(idx) = state.open_bubble.remove(&agent) {
        let mut drop_idx: Option<usize> = None;
        if let Some(entry) = state.timeline.get_mut(idx) {
            if let EntryKind::AgentBubble { closed, text, .. } = &mut entry.kind {
                *closed = true;
                if text.is_empty() {
                    drop_idx = Some(idx);
                }
            }
        }
        if let Some(idx) = drop_idx {
            state.timeline.remove(idx);
            for v in state.call_index.values_mut() {
                if *v > idx {
                    *v -= 1;
                }
            }
            for v in state.open_bubble.values_mut() {
                if *v > idx {
                    *v -= 1;
                }
            }
            if let Some(f) = state.focus.as_mut() {
                if *f > idx {
                    *f -= 1;
                }
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// rendering
// ──────────────────────────────────────────────────────────────────────

fn render(f: &mut ratatui::Frame, state: &mut AppState) {
    // Chatbox is visible whenever the user is actively in chat mode —
    // either the legacy "pause and steer" overlay, or the post-pipeline
    // live chat. It lives *inside* the compiler pane (left column), not
    // across the full width, so the Conductor output on the right stays
    // fully readable.
    // Chatbox stays visible whenever the user is in an interactive
    // chat-capable state (pause-and-steer or post-pipeline live chat).
    // It does *not* require `Mode::Chat` anymore — that flag only
    // controls whether the chatbox owns keyboard input. This lets the
    // user press Esc to "unfocus" the box, use any of the normal
    // shortcuts (Ctrl+O, scrolling, artifacts, etc.), and re-focus
    // with `i` without losing the chatbox contents.
    let show_chat = state.paused || state.live_chat_active();
    let chat_h: u16 = if show_chat {
        // Give the chatbox a generous minimum height (8 rows inner
        // editor) and let it grow with the content up to ~18 rows.
        let lines = state.chat.lines.len().max(1) as u16;
        (lines + 2).clamp(10, 20)
    } else {
        0
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(f.area());

    render_phase_bar(f, chunks[0], state);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(state.split_pct),
            Constraint::Percentage(100 - state.split_pct),
        ])
        .split(chunks[1]);

    state.last_output_area = body[1];

    // Left column: optionally split vertically so the chatbox
    // occupies its bottom. The right column (output) is untouched.
    if show_chat {
        let left = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(chat_h)])
            .split(body[0]);
        state.last_compiler_area = left[0];
        render_left_pane(f, left[0], state);
        render_chatbox(f, left[1], state);
        // Picker pops up above the chatbox, anchored to its top edge.
        render_picker_popup(f, left[1], state);
    } else {
        state.last_compiler_area = body[0];
        state.last_chatbox_area = Rect::new(0, 0, 0, 0);
        render_left_pane(f, body[0], state);
    }

    render_output(f, body[1], state);
    render_hint(f, chunks[2], state);
}

fn render_phase_bar(f: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let now = Instant::now();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let hal_bg = if state.paused { theme::WARN } else { theme::OK };
    let hal_label = if state.paused { " HAL ⏸ " } else { " HAL " };
    spans.push(Span::styled(
        hal_label,
        Style::default()
            .fg(Color::Black)
            .bg(hal_bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" ▸ ", Style::default().fg(theme::DIM)));
    for (i, t) in state.phases.iter().enumerate() {
        let spin = theme::spinner(state.tick);
        let icon: String = match t.state {
            PhaseState::Pending => "·".to_string(),
            PhaseState::Running => spin.to_string(),
            PhaseState::Done => "✓".to_string(),
            PhaseState::Failed => "✗".to_string(),
        };
        let elapsed = t
            .elapsed(now)
            .map(|d| format!(" {:.1}s", d.as_secs_f32()))
            .unwrap_or_default();
        let chip = format!(" {} {}{} ", icon, t.label, elapsed);
        let style = match t.state {
            PhaseState::Running => Style::default()
                .fg(Color::Black)
                .bg(theme::WARN)
                .add_modifier(Modifier::BOLD),
            PhaseState::Done => Style::default().fg(theme::OK),
            PhaseState::Failed => Style::default()
                .fg(Color::White)
                .bg(theme::ERR)
                .add_modifier(Modifier::BOLD),
            PhaseState::Pending => Style::default().fg(theme::DIM),
        };
        spans.push(Span::styled(chip, style));
        if i + 1 < state.phases.len() {
            spans.push(Span::styled(" » ", Style::default().fg(theme::DIM)));
        }
    }
    spans.push(Span::raw("   "));
    spans.push(Span::styled(
        format!("{:.1}s elapsed", state.started.elapsed().as_secs_f32()),
        Style::default().fg(theme::DIM),
    ));
    if !state.mouse_captured {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            "[select mode]",
            Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
        ));
    }

    let p = Paragraph::new(Line::from(spans))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER)),
        )
        .alignment(Alignment::Left);
    f.render_widget(p, area);
}

fn render_hint(f: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let sep = Span::styled("  ·  ", Style::default().fg(theme::DIM));
    let pairs: Vec<(&str, &str)> = if state.mode == Mode::Chat {
        vec![
            ("Enter", "send"),
            ("Sh+Enter", "newline"),
            ("Tab", "agent"),
            ("Esc", "cancel"),
            ("@", "file"),
            ("/", "cmd"),
        ]
    } else if state.left_view == LeftView::Artifacts {
        if state
            .artifacts
            .as_ref()
            .map(|a| a.viewing.is_some())
            .unwrap_or(false)
        {
            vec![
                ("Esc", "back"),
                ("↑↓", "scroll"),
                ("PgUp/PgDn", "page"),
                ("g/G", "top/bot"),
            ]
        } else {
            vec![
                ("↑↓", "move"),
                ("Tab/→", "expand"),
                ("←", "collapse"),
                ("Enter", "open"),
                ("Ctrl+O", "compiler"),
                ("Esc", "back"),
            ]
        }
    } else if state.paused {
        vec![
            ("Space", "resume"),
            ("i", "steer"),
            ("Ctrl+O", "artifacts"),
            ("m", "mouse"),
            ("[ ]", "collapse/expand"),
            ("J/K", "focus"),
            ("Enter", "toggle"),
            ("q", "quit"),
        ]
    } else {
        vec![
            ("q", "quit"),
            ("Space", "pause"),
            ("Ctrl+O", "artifacts"),
            ("m", "mouse"),
            ("↑↓", "scroll"),
            ("Sh", "pane"),
            ("[ ]", "collapse/expand"),
            ("< >", "resize"),
        ]
    };
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(pairs.len() * 3);
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(sep.clone());
        }
        spans.push(Span::styled(
            (*k).to_string(),
            Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            (*v).to_string(),
            Style::default().fg(theme::MUTED),
        ));
    }
    let p = Paragraph::new(Line::from(spans)).alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_chatbox(f: &mut ratatui::Frame, area: Rect, state: &mut AppState) {
    state.last_chatbox_area = area;
    let agent = state.steerable_agent();
    let live = state.live_chat_active();
    let focused = state.mode == Mode::Chat;
    let label = if !focused {
        format!(" ○ chat · {} · press i to focus ", agent_name(agent))
    } else if live {
        if state.chat_busy {
            format!(" ⟳ chat · {} · working… ", agent_name(agent))
        } else {
            format!(" ● chat · {} ", agent_name(agent))
        }
    } else {
        format!(" ✎ steering · {} ", agent_name(agent))
    };
    let hint = if !focused {
        " i: focus · Esc/↑↓: scroll timeline · Ctrl+O: artifacts "
    } else if live {
        " Tab: agent · Enter: send · Shift+Enter: newline · @file · /cmd · Esc: unfocus "
    } else {
        " Tab: agent · Enter: send · Shift+Enter: newline · @file · Esc: unfocus "
    };
    let accent = if focused {
        theme::agent_accent(agent)
    } else {
        theme::DIM
    };
    let title_parts = vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(hint, Style::default().fg(theme::DIM)),
    ];
    let text_style_dim = if focused {
        None
    } else {
        Some(Style::default().fg(theme::DIM))
    };
    let lines: Vec<Line<'static>> = if state.chat.lines.iter().all(|l| l.is_empty()) {
        // Placeholder text for empty chatbox.
        let placeholder = if !focused {
            "press i to focus the chatbox"
        } else if live {
            "type to chat · /help for commands · /interpret /build /run · @file to attach"
        } else {
            "type a steering message · Enter to send · Esc to unfocus"
        };
        vec![Line::from(vec![
            Span::styled(
                if focused { "▏" } else { " " },
                Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                placeholder.to_string(),
                Style::default()
                    .fg(theme::DIM)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])]
    } else {
        state
            .chat
            .lines
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if focused && i == state.chat.cy {
                    let (before, after) = s.split_at(state.chat.cx.min(s.len()));
                    Line::from(vec![
                        Span::raw(before.to_string()),
                        Span::styled(
                            "▏",
                            Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(after.to_string()),
                    ])
                } else if let Some(st) = text_style_dim {
                    Line::from(Span::styled(s.clone(), st))
                } else {
                    Line::from(Span::raw(s.clone()))
                }
            })
            .collect()
    };
    let title_line = Line::from(title_parts);
    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title_line)
                .border_style(Style::default().fg(accent)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

/// Render the `@` or `/` picker overlay just above the chatbox area.
fn render_picker_popup(f: &mut ratatui::Frame, chat_area: Rect, state: &AppState) {
    let Some(picker) = state.picker.as_ref() else {
        return;
    };
    let desired_rows = picker.items.len().clamp(1, 8) as u16;
    let popup_h = desired_rows.saturating_add(2); // borders
    let popup_w = chat_area.width.clamp(30, 80);
    // Position the popup directly above the chatbox so it feels
    // anchored to it, like agentic chat UIs. Clamp to frame.
    let frame = f.area();
    let y = chat_area.y.saturating_sub(popup_h);
    let y = y.max(frame.y);
    let x = chat_area.x;
    let popup = Rect::new(x, y, popup_w.min(frame.width.saturating_sub(x)), popup_h);
    let (label, accent) = match picker.kind {
        PickerKind::File => (" @ attach file ", theme::KEY),
        PickerKind::Command => (" / command ", theme::WARN),
    };
    let title = Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  {} matches · ↑↓ select · Tab insert · Esc cancel ",
                picker.items.len()
            ),
            Style::default().fg(theme::DIM),
        ),
    ]);
    let rows: Vec<Line<'static>> = if picker.items.is_empty() {
        vec![Line::from(Span::styled(
            "  (no matches)".to_string(),
            Style::default()
                .fg(theme::DIM)
                .add_modifier(Modifier::ITALIC),
        ))]
    } else {
        picker
            .items
            .iter()
            .enumerate()
            .take(8)
            .map(|(i, it)| {
                let marker = if i == picker.cursor { "▸ " } else { "  " };
                let marker_style = if i == picker.cursor {
                    Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::DIM)
                };
                let insert_style = if i == picker.cursor {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::MUTED)
                };
                Line::from(vec![
                    Span::styled(marker.to_string(), marker_style),
                    Span::styled(it.insert.clone(), insert_style),
                    Span::raw("  "),
                    Span::styled(
                        it.hint.clone(),
                        Style::default()
                            .fg(theme::DIM)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ])
            })
            .collect()
    };
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(accent));
    let p = Paragraph::new(rows).block(block);
    f.render_widget(p, popup);
}

fn render_left_pane(f: &mut ratatui::Frame, area: Rect, state: &mut AppState) {
    match state.left_view {
        LeftView::Compiler => render_compiler(f, area, state),
        LeftView::Artifacts => render_artifacts(f, area, state),
    }
}

fn render_artifacts(f: &mut ratatui::Frame, area: Rect, state: &mut AppState) {
    let Some(art) = state.artifacts.as_mut() else {
        return;
    };
    // File viewer takes over the entire pane when open.
    if let Some(view) = art.viewing.as_ref() {
        let rel = view
            .path
            .strip_prefix(&art.root)
            .unwrap_or(&view.path)
            .display()
            .to_string();
        let title_parts = vec![
            Span::styled(
                format!(" 📄 {} ", rel),
                Style::default()
                    .fg(Color::Black)
                    .bg(theme::KEY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} lines · Esc back · ↑↓ scroll ", view.lines.len()),
                Style::default().fg(theme::DIM),
            ),
        ];
        let inner_h = area.height.saturating_sub(2) as usize;
        let start = view.scroll.min(view.lines.len().saturating_sub(1));
        let end = (start + inner_h).min(view.lines.len());
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(end.saturating_sub(start) + 1);
        if let Some(err) = view.error.as_ref() {
            lines.push(Line::from(Span::styled(
                err.clone(),
                Style::default()
                    .fg(theme::WARN)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
        let width = (area.width as usize).saturating_sub(6).max(10);
        for (i, raw) in view.lines[start..end].iter().enumerate() {
            let n = start + i + 1;
            let num = Span::styled(format!("{:>4} ", n), Style::default().fg(theme::DIM));
            // Hard-truncate to prevent wrap so line numbers stay aligned.
            let text = if raw.chars().count() > width {
                let truncated: String = raw.chars().take(width.saturating_sub(1)).collect();
                format!("{}…", truncated)
            } else {
                raw.clone()
            };
            lines.push(Line::from(vec![num, Span::raw(text)]));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(title_parts))
            .border_style(Style::default().fg(theme::KEY));
        let p = Paragraph::new(lines).block(block);
        f.render_widget(p, area);
        return;
    }

    // Tree view.
    let root_name = art
        .root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| art.root.display().to_string());
    let title_parts = vec![
        Span::styled(
            format!(" 📁 artifacts · {} ", root_name),
            Style::default()
                .fg(Color::Black)
                .bg(theme::OK)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " ↑↓ move · Tab/→ expand · ← collapse · Enter open · Ctrl+O back ",
            Style::default().fg(theme::DIM),
        ),
    ];

    let inner_h = area.height.saturating_sub(2) as usize;
    // Keep cursor in view with a simple scroll window.
    if art.cursor < art.scroll {
        art.scroll = art.cursor;
    } else if art.cursor >= art.scroll + inner_h {
        art.scroll = art.cursor + 1 - inner_h;
    }
    let start = art.scroll;
    let end = (start + inner_h).min(art.rows.len());

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(end.saturating_sub(start));
    if art.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (empty directory)".to_string(),
            Style::default()
                .fg(theme::DIM)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    for (i, row) in art.rows[start..end].iter().enumerate() {
        let idx = start + i;
        let selected = idx == art.cursor;
        let indent = "  ".repeat(row.depth);
        let glyph = if row.is_dir {
            if art.expanded.contains(&row.path) {
                "▾ "
            } else {
                "▸ "
            }
        } else {
            "  "
        };
        let name_style = if row.is_dir {
            Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let (marker, marker_style) = if selected {
            (
                "▌ ",
                Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
            )
        } else {
            ("  ", Style::default().fg(theme::DIM))
        };
        let display_name = if row.is_dir {
            format!("{}/", row.name)
        } else {
            row.name.clone()
        };
        let content_style = if selected {
            name_style.add_modifier(Modifier::REVERSED)
        } else {
            name_style
        };
        lines.push(Line::from(vec![
            Span::styled(marker.to_string(), marker_style),
            Span::raw(indent),
            Span::styled(
                glyph.to_string(),
                Style::default().fg(if row.is_dir { theme::OK } else { theme::DIM }),
            ),
            Span::styled(display_name, content_style),
        ]));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(title_parts))
        .border_style(Style::default().fg(theme::OK));
    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn render_compiler(f: &mut ratatui::Frame, area: Rect, state: &mut AppState) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(state.timeline.len() * 3);
    for (i, entry) in state.timeline.iter().enumerate() {
        let is_focus = state.focus == Some(i);
        let n_before = lines.len();
        push_entry_lines(&mut lines, entry, inner_w, is_focus);
        // Tight stacking for collapsed entries; standard entries get a
        // trailing blank separator.
        let added = lines.len() - n_before;
        if added > 0 && !entry.collapsed {
            lines.push(Line::raw(""));
        }
    }
    if !state.thinking.is_empty() {
        lines.push(thinking_line(state));
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let total = lines.len();
    let max_offset = total.saturating_sub(inner_height) as u16;
    if state.compiler_scroll.offset > max_offset {
        state.compiler_scroll.offset = max_offset;
        if max_offset == 0 {
            state.compiler_scroll.paused = false;
        }
    }
    let offset = state.compiler_scroll.offset as usize;
    let end = total.saturating_sub(offset).min(total);
    let start = end.saturating_sub(inner_height);
    let visible: Vec<Line> = lines.into_iter().skip(start).take(end - start).collect();

    let title = if state.paused {
        format!(
            " Compiler — {} events  ⏸ PAUSED (Space to resume) ",
            state.timeline.len()
        )
    } else if state.compiler_scroll.paused {
        format!(
            " Compiler — {} events  (scroll paused — End to follow) ",
            state.timeline.len()
        )
    } else if !state.thinking.is_empty() {
        let mut agents: Vec<Agent> = state.thinking.iter().copied().collect();
        agents.sort_by_key(|a| *a as u8);
        let names: Vec<&str> = agents.iter().map(|a| agent_name(*a)).collect();
        format!(
            " Compiler · {} {} — {} events ",
            names.join(" + "),
            theme::spinner(state.tick),
            state.timeline.len()
        )
    } else {
        format!(" Compiler — {} events ", state.timeline.len())
    };
    let border_col = if state.paused {
        theme::WARN
    } else {
        Color::DarkGray
    };
    let p = Paragraph::new(visible)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(border_col)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_output(f: &mut ratatui::Frame, area: Rect, state: &mut AppState) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let total = state.output_lines.len();
    let max_offset = total.saturating_sub(inner_height) as u16;
    if state.output_scroll.offset > max_offset {
        state.output_scroll.offset = max_offset;
        if max_offset == 0 {
            state.output_scroll.paused = false;
        }
    }
    let offset = state.output_scroll.offset as usize;
    let end = total.saturating_sub(offset).min(total);
    let start = end.saturating_sub(inner_height);
    let slice: Vec<Line> = state.output_lines[start..end].to_vec();

    let title = if total == 0 {
        " Output — (waiting for container) ".to_string()
    } else if state.output_scroll.paused {
        format!(
            " Output — {} lines  (scroll paused — End to follow) ",
            total
        )
    } else {
        format!(" Output — {} lines ", total)
    };

    let p = Paragraph::new(slice)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

// ──────────────────────────────────────────────────────────────────────
// timeline rendering
// ──────────────────────────────────────────────────────────────────────

fn focus_prefix(is_focus: bool) -> Span<'static> {
    if is_focus {
        Span::styled(
            "▸ ",
            Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    }
}

fn push_entry_lines(
    out: &mut Vec<Line<'static>>,
    entry: &TimelineEntry,
    inner_w: usize,
    is_focus: bool,
) {
    let collapse = entry.collapsed;
    match &entry.kind {
        EntryKind::PhaseBanner { phase } => {
            for l in theme::banner_lines(*phase, inner_w) {
                out.push(l);
            }
        }
        EntryKind::AgentLog { agent, message } => {
            out.push(Line::from(vec![
                focus_prefix(is_focus),
                Span::styled("·  ", Style::default().fg(theme::DIM)),
                agent_tag(*agent),
                Span::raw(" "),
                Span::styled(
                    message.clone(),
                    Style::default()
                        .fg(theme::MUTED)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
        EntryKind::AgentBubble {
            agent,
            text,
            closed,
        } => {
            if collapse {
                push_collapsed_bubble(out, *agent, text, *closed, false, is_focus);
            } else {
                push_bubble(out, *agent, text, *closed, inner_w, false, is_focus);
            }
        }
        EntryKind::AgentResponse { agent, text } => {
            if collapse {
                push_collapsed_bubble(out, *agent, text, true, true, is_focus);
            } else {
                push_bubble(out, *agent, text, true, inner_w, true, is_focus);
            }
        }
        EntryKind::ToolCall {
            agent,
            tool,
            args,
            result,
        } => {
            if collapse {
                push_collapsed_tool(out, *agent, tool, args, result.as_ref(), is_focus);
            } else {
                push_expanded_tool(out, *agent, tool, args, result.as_ref(), inner_w, is_focus);
            }
        }
        EntryKind::RunAction {
            image,
            command,
            cwd,
            args,
        } => {
            if collapse {
                let argv = if args.is_empty() {
                    String::new()
                } else {
                    format!(" {}", shell_join_preview(args))
                };
                out.push(Line::from(vec![
                    focus_prefix(is_focus),
                    Span::styled(
                        "▸ run action  ",
                        Style::default()
                            .fg(theme::agent_accent(Agent::Conductor))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{} · {}{}", image, command, argv),
                        Style::default().fg(theme::MUTED),
                    ),
                ]));
            } else {
                push_run_action(out, image, command, cwd.as_deref(), args, inner_w, is_focus);
            }
        }
        EntryKind::Steering { agent, message } => {
            let first = message.lines().next().unwrap_or("");
            if collapse {
                out.push(Line::from(vec![
                    focus_prefix(is_focus),
                    Span::styled(
                        "▸ steering → ",
                        Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        agent_name(*agent),
                        Style::default()
                            .fg(theme::agent_accent(*agent))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(first.to_string(), Style::default().fg(theme::MUTED)),
                ]));
            } else {
                out.push(Line::from(vec![
                    focus_prefix(is_focus),
                    Span::styled(
                        "✎ steering → ",
                        Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        agent_name(*agent),
                        Style::default()
                            .fg(theme::agent_accent(*agent))
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                let avail = inner_w.saturating_sub(4).max(20);
                for raw in message.lines() {
                    for w in wrap_text(raw, avail) {
                        out.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled("│ ", Style::default().fg(theme::KEY)),
                            Span::styled(w, Style::default().fg(theme::RESPONSE)),
                        ]));
                    }
                }
            }
        }
        EntryKind::Paused => {
            out.push(Line::from(vec![
                focus_prefix(is_focus),
                Span::styled(
                    "⏸ paused",
                    Style::default()
                        .fg(theme::WARN)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        EntryKind::Resumed => {
            out.push(Line::from(vec![
                focus_prefix(is_focus),
                Span::styled(
                    "▶ resumed",
                    Style::default().fg(theme::OK).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        EntryKind::Failed(msg) => {
            out.push(Line::from(Span::styled(
                format!("✗ FAILED — {}", msg),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
        }
        EntryKind::Done(code) => {
            let colour = if *code == 0 { Color::Green } else { Color::Red };
            out.push(Line::from(Span::styled(
                format!("● pipeline done — exit {}", code),
                Style::default().fg(colour).add_modifier(Modifier::BOLD),
            )));
        }
    }
}

fn push_expanded_tool(
    out: &mut Vec<Line<'static>>,
    agent: Agent,
    tool: &str,
    args: &str,
    result: Option<&(bool, String, u64)>,
    inner_w: usize,
    is_focus: bool,
) {
    let (icon, colour) = tool_icon(tool);
    let accent = theme::agent_accent(agent);
    let (status_text, status_style) = match result {
        None => (
            "● running".to_string(),
            Style::default()
                .fg(theme::WARN)
                .add_modifier(Modifier::BOLD),
        ),
        Some((true, _, ms)) => (
            format!("✓ ok · {} ms", ms),
            Style::default().fg(theme::OK).add_modifier(Modifier::BOLD),
        ),
        Some((false, _, ms)) => (
            format!("✗ err · {} ms", ms),
            Style::default().fg(theme::ERR).add_modifier(Modifier::BOLD),
        ),
    };

    let head_left = format!("╭─ {} ", icon);
    let head_mid_name = tool.to_string();
    let head_mid_sep = " · ";
    let head_mid_agent = agent_name(agent).to_string();
    let used: usize = display_width(&head_left)
        + display_width(&head_mid_name)
        + display_width(head_mid_sep)
        + display_width(&head_mid_agent)
        + 2
        + display_width(&status_text);
    let fill_w = inner_w.saturating_sub(used).max(2);
    let fill = "─".repeat(fill_w);
    out.push(Line::from(vec![
        focus_prefix(is_focus),
        Span::styled(head_left, Style::default().fg(theme::BORDER)),
        Span::styled(
            head_mid_name,
            Style::default().fg(colour).add_modifier(Modifier::BOLD),
        ),
        Span::styled(head_mid_sep, Style::default().fg(theme::DIM)),
        Span::styled(
            head_mid_agent,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {} ", fill), Style::default().fg(theme::BORDER)),
        Span::styled(status_text, status_style),
    ]));

    let bar = Span::styled("│ ", Style::default().fg(theme::BORDER));
    let label_w = 7usize;
    let body_w = inner_w.saturating_sub(label_w + 3).max(20);

    let mut first = true;
    for raw in args.lines() {
        for line in wrap_text(raw, body_w) {
            let lbl = if first { "args   " } else { "       " };
            first = false;
            out.push(Line::from(vec![
                Span::raw("  "),
                bar.clone(),
                Span::styled(lbl, Style::default().fg(theme::DIM)),
                Span::styled(line, Style::default().fg(Color::Cyan)),
            ]));
        }
    }

    if let Some((ok, output, _)) = result {
        let res_colour = if *ok { theme::OK } else { theme::ERR };
        let mut first = true;
        for raw in output.lines() {
            for line in wrap_text(raw, body_w) {
                let lbl = if first {
                    if *ok {
                        "ok     "
                    } else {
                        "err    "
                    }
                } else {
                    "       "
                };
                first = false;
                out.push(Line::from(vec![
                    Span::raw("  "),
                    bar.clone(),
                    Span::styled(lbl, Style::default().fg(theme::DIM)),
                    Span::styled(line, Style::default().fg(res_colour)),
                ]));
            }
        }
        let foot_fill = "─".repeat(inner_w.saturating_sub(1).max(2));
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("╰{}", foot_fill),
                Style::default().fg(theme::BORDER),
            ),
        ]));
    } else {
        out.push(Line::from(vec![
            Span::raw("  "),
            bar.clone(),
            Span::styled("       ", Style::default().fg(theme::DIM)),
            Span::styled(
                "(running…)",
                Style::default()
                    .fg(theme::WARN)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }
}

fn push_collapsed_tool(
    out: &mut Vec<Line<'static>>,
    agent: Agent,
    tool: &str,
    args: &str,
    result: Option<&(bool, String, u64)>,
    is_focus: bool,
) {
    let (icon, colour) = tool_icon(tool);
    let accent = theme::agent_accent(agent);
    let (status, style) = match result {
        None => (
            "●".to_string(),
            Style::default()
                .fg(theme::WARN)
                .add_modifier(Modifier::BOLD),
        ),
        Some((true, _, ms)) => (format!("✓{}ms", ms), Style::default().fg(theme::OK)),
        Some((false, _, ms)) => (
            format!("✗{}ms", ms),
            Style::default().fg(theme::ERR).add_modifier(Modifier::BOLD),
        ),
    };
    let summary = args.lines().next().unwrap_or("").to_string();
    out.push(Line::from(vec![
        focus_prefix(is_focus),
        Span::styled("▸ ", Style::default().fg(theme::BORDER)),
        Span::styled(format!("{icon} "), Style::default().fg(colour)),
        Span::styled(
            tool.to_string(),
            Style::default().fg(colour).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(theme::DIM)),
        Span::styled(
            agent_name(agent),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(status, style),
        Span::raw("  "),
        Span::styled(summary, Style::default().fg(theme::MUTED)),
    ]));
}

fn push_run_action(
    out: &mut Vec<Line<'static>>,
    image: &str,
    command: &str,
    cwd: Option<&str>,
    args: &[String],
    inner_w: usize,
    is_focus: bool,
) {
    let accent = theme::agent_accent(Agent::Conductor);
    let title = "▶ run action";
    let used: usize = 3 + display_width(title) + 2;
    let fill_w = inner_w.saturating_sub(used).max(2);
    let fill = "─".repeat(fill_w);
    out.push(Line::from(vec![
        focus_prefix(is_focus),
        Span::styled("╭─ ", Style::default().fg(theme::BORDER)),
        Span::styled(
            title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {} ", fill), Style::default().fg(theme::BORDER)),
    ]));

    let bar = Span::styled("│ ", Style::default().fg(theme::BORDER));
    let label_w = 8usize;
    let body_w = inner_w.saturating_sub(label_w + 3).max(20);

    let row = |out: &mut Vec<Line<'static>>, label: &'static str, value: String, style: Style| {
        let mut first = true;
        for line in wrap_text(&value, body_w) {
            let lbl = if first { label } else { "        " };
            first = false;
            out.push(Line::from(vec![
                Span::raw("  "),
                bar.clone(),
                Span::styled(lbl, Style::default().fg(theme::DIM)),
                Span::styled(line, style),
            ]));
        }
    };

    row(
        out,
        "image   ",
        image.to_string(),
        Style::default().fg(Color::Cyan),
    );
    row(
        out,
        "cwd     ",
        cwd.unwrap_or("/work").to_string(),
        Style::default().fg(Color::Cyan),
    );
    row(
        out,
        "command ",
        command.to_string(),
        Style::default().fg(Color::White),
    );
    let (argv_display, argv_color) = if args.is_empty() {
        ("(none)".to_string(), theme::DIM)
    } else {
        (shell_join_preview(args), Color::Yellow)
    };
    row(
        out,
        "argv    ",
        argv_display,
        Style::default().fg(argv_color).add_modifier(Modifier::BOLD),
    );
    let invocation = if args.is_empty() {
        command.to_string()
    } else {
        format!("{command} {}", shell_join_preview(args))
    };
    row(out, "exec    ", invocation, Style::default().fg(theme::OK));

    let foot_fill = "─".repeat(inner_w.saturating_sub(1).max(2));
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("╰{}", foot_fill),
            Style::default().fg(theme::BORDER),
        ),
    ]));
}

fn push_bubble(
    out: &mut Vec<Line<'static>>,
    agent: Agent,
    text: &str,
    closed: bool,
    inner_w: usize,
    final_response: bool,
    is_focus: bool,
) {
    let accent = theme::agent_accent(agent);
    let (header_label, header_style, body_color, body_italic) = if final_response {
        (
            format!(" ⟵ response · {} ", agent_name(agent)),
            Style::default()
                .fg(Color::Black)
                .bg(theme::OK)
                .add_modifier(Modifier::BOLD),
            theme::RESPONSE,
            false,
        )
    } else if closed {
        (
            format!(" thoughts · {} ", agent_name(agent)),
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            theme::THINKING_CLOSED,
            false,
        )
    } else {
        (
            format!(" streaming… · {} ", agent_name(agent)),
            Style::default()
                .fg(Color::Black)
                .bg(theme::WARN)
                .add_modifier(Modifier::BOLD),
            theme::THINKING_STREAM,
            true,
        )
    };
    out.push(Line::from(vec![
        focus_prefix(is_focus),
        Span::styled(header_label, header_style),
    ]));

    let mut body_style = Style::default().fg(body_color);
    if body_italic {
        body_style = body_style.add_modifier(Modifier::ITALIC);
    }
    let bar_color = if final_response { theme::OK } else { accent };
    let bar = Span::styled("  ▎ ", Style::default().fg(bar_color));

    if text.is_empty() {
        out.push(Line::from(vec![
            bar.clone(),
            Span::styled(
                "(no text yet)",
                Style::default()
                    .fg(theme::DIM)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
        return;
    }
    let avail = inner_w.saturating_sub(4).max(20);
    for raw in text.lines() {
        let wrapped = wrap_text(raw, avail);
        if wrapped.is_empty() {
            out.push(Line::from(vec![bar.clone()]));
        } else {
            for w in wrapped {
                out.push(Line::from(vec![bar.clone(), Span::styled(w, body_style)]));
            }
        }
    }
}

fn push_collapsed_bubble(
    out: &mut Vec<Line<'static>>,
    agent: Agent,
    text: &str,
    _closed: bool,
    final_response: bool,
    is_focus: bool,
) {
    let accent = theme::agent_accent(agent);
    let (marker, marker_style) = if final_response {
        (
            "▸ response",
            Style::default().fg(theme::OK).add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "▸ thoughts",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )
    };
    let summary = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string();
    out.push(Line::from(vec![
        focus_prefix(is_focus),
        Span::styled(marker, marker_style),
        Span::styled(" · ", Style::default().fg(theme::DIM)),
        Span::styled(
            agent_name(agent),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(summary, Style::default().fg(theme::MUTED)),
    ]));
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.is_empty() {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    for word in text.split_whitespace() {
        if buf.is_empty() {
            if word.chars().count() > width {
                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() + 1 > width {
                        out.push(std::mem::take(&mut chunk));
                    }
                    chunk.push(ch);
                }
                buf = chunk;
            } else {
                buf.push_str(word);
            }
        } else if buf.chars().count() + 1 + word.chars().count() <= width {
            buf.push(' ');
            buf.push_str(word);
        } else {
            out.push(std::mem::take(&mut buf));
            buf.push_str(word);
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn thinking_line(state: &AppState) -> Line<'static> {
    let mut agents: Vec<Agent> = state.thinking.iter().copied().collect();
    agents.sort_by_key(|a| *a as u8);
    let accent = agents
        .first()
        .copied()
        .map(theme::agent_accent)
        .unwrap_or(theme::WARN);
    let names: Vec<&str> = agents.iter().map(|a| agent_name(*a)).collect();
    let label = if names.len() == 1 {
        format!("{} thinking", names[0])
    } else {
        format!("{} thinking", names.join(" + "))
    };
    let spin = theme::spinner(state.tick);
    let noise = thinking_noise(state.tick, 24);
    Line::from(vec![
        Span::styled(
            format!("  {} ", spin),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            label,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(noise, Style::default().fg(theme::DIM)),
    ])
}

fn thinking_noise(seed: u64, width: usize) -> String {
    const GLYPHS: &[u8] = b"!@#$%^&*+=?/\\|<>~`[]{}()ABCDEFGHJKLMNPQRSTUVWXYZ0123456789";
    let mut s = String::with_capacity(width);
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    for _ in 0..width {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let idx = ((x >> 33) as usize) % GLYPHS.len();
        s.push(GLYPHS[idx] as char);
    }
    s
}

fn agent_tag(a: Agent) -> Span<'static> {
    let col = theme::agent_accent(a);
    let txt = match a {
        Agent::Interpreter => "[Interpreter]",
        Agent::Architect => "[Architect] ",
        Agent::Conductor => "[Conductor] ",
        Agent::Reflector => "[Reflector] ",
    };
    Span::styled(txt, Style::default().fg(col).add_modifier(Modifier::BOLD))
}

fn agent_name(a: Agent) -> &'static str {
    match a {
        Agent::Interpreter => "Interpreter",
        Agent::Architect => "Architect",
        Agent::Conductor => "Conductor",
        Agent::Reflector => "Reflector",
    }
}

fn shell_join_preview(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.is_empty() {
                "''".to_string()
            } else if a
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_./:=@+-".contains(c))
            {
                a.clone()
            } else {
                let escaped = a.replace('\'', "'\\''");
                format!("'{escaped}'")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tool_icon(tool: &str) -> (&'static str, Color) {
    match tool {
        "files_read" => ("📂", Color::Blue),
        "files_write" => ("✍ ", Color::Green),
        "memory_search" => ("🔎", Color::Cyan),
        "memory_write" => ("🧠", Color::Magenta),
        "web_search" => ("🌐", Color::LightBlue),
        "spawn_agent" => ("🤖", Color::LightMagenta),
        "run_cmd" => ("⚡", Color::Yellow),
        _ => ("⚙ ", Color::Gray),
    }
}

/// Visible (terminal-column) width of a string, matching what ratatui
/// uses when laying out cells. Using char count alone under-counts
/// East-Asian-wide and emoji glyphs and causes header/fill calculations
/// to over-fill the line, which then wraps (e.g. tool cards where
/// `… ms` spilled onto a second line).
fn display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    UnicodeWidthStr::width(s)
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    #[test]
    fn agent_key_round_trips_for_every_agent() {
        for a in [
            Agent::Interpreter,
            Agent::Architect,
            Agent::Conductor,
            Agent::Reflector,
        ] {
            assert_eq!(parse_agent_key(agent_key(a)), a);
        }
        assert_eq!(parse_agent_key("reflector"), Agent::Reflector);
    }

    #[test]
    fn tab_cycle_visits_all_four_agents_in_order() {
        let mut a = Agent::Interpreter;
        let mut seen = vec![a];
        for _ in 0..4 {
            a = next_agent(a);
            seen.push(a);
        }
        assert_eq!(
            seen,
            vec![
                Agent::Interpreter,
                Agent::Architect,
                Agent::Conductor,
                Agent::Reflector,
                Agent::Interpreter
            ]
        );
    }

    #[test]
    fn reflector_has_distinct_label_accent_and_logo() {
        assert_eq!(agent_name(Agent::Reflector), "Reflector");
        assert_eq!(agent_tag(Agent::Reflector).content.trim(), "[Reflector]");
        assert_eq!(
            agent_tag(Agent::Reflector).content.len(),
            agent_tag(Agent::Conductor).content.len()
        );
        assert_ne!(
            theme::agent_accent(Agent::Reflector),
            theme::agent_accent(Agent::Conductor)
        );
        assert_eq!(
            theme::agent_accent(Agent::Reflector),
            theme::phase_accent(Phase::Reflecting)
        );
        assert!(theme::phase_logo(Phase::Reflecting).is_some());
    }
}
