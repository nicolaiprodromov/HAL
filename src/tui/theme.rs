//! Centralised colour palette, embedded phase logos, and tiny style
//! helpers for the compiler pane. Keeping all of this in one place
//! makes it easy to rebalance the look without hunting through
//! `app.rs`.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::events::{Agent, Phase};

// ── embedded logos (compiled in, so we don't depend on cwd) ─────────

pub const ARCHITECT_LOGO: &str = include_str!("../../HAL/architect.logo");
pub const CONDUCTOR_LOGO: &str = include_str!("../../HAL/conductor.logo");
pub const INTERPRETER_LOGO: &str = include_str!("../../HAL/interpreter.logo");

// ── palette ─────────────────────────────────────────────────────────

pub const BORDER: Color = Color::DarkGray;
pub const DIM: Color = Color::DarkGray;
pub const MUTED: Color = Color::Gray;

/// Text colour for an actively-streaming thinking bubble. Muted on
/// purpose so in-progress tokens don't out-shout the final response.
pub const THINKING_STREAM: Color = Color::DarkGray;
/// Text colour for a closed (completed) thinking bubble.
pub const THINKING_CLOSED: Color = Color::Gray;
/// Text colour for a final agent response.
pub const RESPONSE: Color = Color::White;

pub const OK: Color = Color::Green;
pub const WARN: Color = Color::Yellow;
pub const ERR: Color = Color::Red;
pub const KEY: Color = Color::LightBlue;

/// Per-agent accent used for bubbles, bars, and name tags.
pub fn agent_accent(a: Agent) -> Color {
    match a {
        Agent::Interpreter => Color::Cyan,
        Agent::Architect => Color::Magenta,
        Agent::Conductor => Color::LightYellow,
    }
}

/// Accent colour for a phase banner. Mirrors the agent driving it.
pub fn phase_accent(p: Phase) -> Color {
    match p {
        Phase::Interpreting => Color::Cyan,
        Phase::Building => Color::Magenta,
        Phase::Running => Color::LightYellow,
        Phase::Reflecting => Color::LightGreen,
        Phase::Done => Color::Green,
        Phase::Failed => Color::Red,
    }
}

/// Embedded logo for a phase, or `None` for phases that don't have art.
pub fn phase_logo(p: Phase) -> Option<&'static str> {
    match p {
        Phase::Interpreting => Some(INTERPRETER_LOGO),
        Phase::Building => Some(ARCHITECT_LOGO),
        Phase::Running => Some(CONDUCTOR_LOGO),
        _ => None,
    }
}

// ── helpers ─────────────────────────────────────────────────────────

/// Smooth braille spinner frame for `tick`.
pub fn spinner(tick: u64) -> char {
    const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    FRAMES[(tick as usize) % FRAMES.len()]
}

/// Build centered logo lines (plus a label) for a phase banner. Falls
/// back to a single thin divider line when the pane is narrower than
/// the logo.
pub fn banner_lines(phase: Phase, inner_w: usize) -> Vec<Line<'static>> {
    let accent = phase_accent(phase);
    let title_raw = phase.to_string();
    let title = title_raw.to_uppercase();

    let Some(logo) = phase_logo(phase) else {
        // No logo → simple centered label.
        let label = format!("── {} ──", title);
        return vec![
            Line::raw(""),
            center_styled(
                &label,
                inner_w,
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
        ];
    };

    let logo_lines: Vec<&str> = logo.lines().filter(|l| !l.is_empty()).collect();
    let max_w = logo_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    if inner_w < max_w + 2 {
        // Pane too narrow for the art → thin divider fallback.
        let label = format!("── {} ──", title);
        return vec![
            Line::raw(""),
            center_styled(
                &label,
                inner_w,
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
        ];
    }

    let pad = (inner_w.saturating_sub(max_w)) / 2;
    let pad_str: String = " ".repeat(pad);
    let art_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);

    let mut out = Vec::with_capacity(logo_lines.len() + 4);
    out.push(Line::raw(""));
    for l in &logo_lines {
        out.push(Line::from(vec![
            Span::raw(pad_str.clone()),
            Span::styled((*l).to_string(), art_style),
        ]));
    }
    let label = format!("── {} ──", title);
    out.push(Line::raw(""));
    out.push(center_styled(
        &label,
        inner_w,
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    ));
    out.push(Line::raw(""));
    out
}

/// Horizontally center `text` in an `inner_w`-wide area and style it.
pub fn center_styled(text: &str, inner_w: usize, style: Style) -> Line<'static> {
    let w = text.chars().count();
    let pad = inner_w.saturating_sub(w) / 2;
    Line::from(vec![
        Span::raw(" ".repeat(pad)),
        Span::styled(text.to_string(), style),
    ])
}
