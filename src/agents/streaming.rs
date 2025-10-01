//! Shared helper that turns a rig `StreamingPromptRequest` into
//! `Event::AgentToken` / `AgentThinkingStart` / `AgentThinkingEnd`
//! events on our bus and returns the final concatenated assistant text.
//!
//! Also integrates with [`crate::control::PauseController`]:
//! - Each stream item awaits `pause.await_if_paused()` before emitting,
//!   so when the user hits Space the UI freezes cleanly mid-turn.
//! - If a steering message arrives for this agent, the current stream
//!   is aborted (dropped) and a [`DriveOutcome::Steered`] is returned so
//!   the caller can re-issue a fresh prompt incorporating the steer.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use rig::agent::{MultiTurnStreamItem, PromptHook, StreamingPromptRequest};
use rig::completion::CompletionModel;
use rig::streaming::StreamedAssistantContent;

use crate::control::PauseController;
use crate::events::{Agent as AgentKind, Event, EventSender};

/// Outcome of a single `drive` invocation.
#[derive(Debug, Clone)]
pub enum DriveOutcome {
    /// Stream ran to completion normally.
    Finished { text: String },
    /// Stream was interrupted by user steering. Caller should issue a
    /// fresh prompt that incorporates `steers` and the partial `text`.
    Steered { text: String, steers: Vec<String> },
}

impl DriveOutcome {
    pub fn text(&self) -> &str {
        match self {
            DriveOutcome::Finished { text } | DriveOutcome::Steered { text, .. } => text,
        }
    }
}

/// Drive a rig streaming prompt request to completion, emitting token
/// events to the event bus as text arrives.
pub async fn drive<M, P>(
    req: StreamingPromptRequest<M, P>,
    agent: AgentKind,
    events: &EventSender,
    pause: Option<&Arc<PauseController>>,
) -> Result<DriveOutcome>
where
    M: CompletionModel + 'static,
    <M as CompletionModel>::StreamingResponse: Send,
    P: PromptHook<M> + Send + 'static,
{
    let mut stream = req.await;
    let _ = events.send(Event::AgentThinkingStart { agent });
    let mut thinking_open = true;
    let mut last_final = String::new();
    let mut current_turn_text = String::new();
    let mut full_text = String::new();

    loop {
        // If paused, stall here BEFORE pulling the next stream item.
        if let Some(p) = pause {
            if p.is_paused() {
                if thinking_open {
                    let _ = events.send(Event::AgentThinkingEnd { agent });
                }
                p.await_if_paused().await;
                let steers = p.drain_steer(agent);
                if !steers.is_empty() {
                    return Ok(DriveOutcome::Steered {
                        text: full_text,
                        steers,
                    });
                }
                let _ = events.send(Event::AgentThinkingStart { agent });
                thinking_open = true;
            } else if p.has_steer(agent) {
                let steers = p.drain_steer(agent);
                if thinking_open {
                    let _ = events.send(Event::AgentThinkingEnd { agent });
                }
                return Ok(DriveOutcome::Steered {
                    text: full_text,
                    steers,
                });
            }
        }

        let Some(item) = stream.next().await else {
            break;
        };
        match item {
            Ok(MultiTurnStreamItem::StreamAssistantItem(content)) => match content {
                StreamedAssistantContent::Text(t) => {
                    if !thinking_open {
                        let _ = events.send(Event::AgentThinkingStart { agent });
                        thinking_open = true;
                    }
                    if !t.text.is_empty() {
                        current_turn_text.push_str(&t.text);
                        full_text.push_str(&t.text);
                        let _ = events.send(Event::AgentToken {
                            agent,
                            delta: t.text,
                        });
                    }
                }
                StreamedAssistantContent::ToolCall { .. } => {
                    if thinking_open {
                        let _ = events.send(Event::AgentThinkingEnd { agent });
                        thinking_open = false;
                    }
                    current_turn_text.clear();
                }
                StreamedAssistantContent::ToolCallDelta { .. } => {}
                StreamedAssistantContent::Reasoning(r) => {
                    if !thinking_open {
                        let _ = events.send(Event::AgentThinkingStart { agent });
                        thinking_open = true;
                    }
                    for c in &r.content {
                        if let rig::message::ReasoningContent::Text { text, .. } = c {
                            full_text.push_str(text);
                            let _ = events.send(Event::AgentToken {
                                agent,
                                delta: text.clone(),
                            });
                        }
                    }
                }
                StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                    if !thinking_open {
                        let _ = events.send(Event::AgentThinkingStart { agent });
                        thinking_open = true;
                    }
                    if !reasoning.is_empty() {
                        full_text.push_str(&reasoning);
                        let _ = events.send(Event::AgentToken {
                            agent,
                            delta: reasoning,
                        });
                    }
                }
                StreamedAssistantContent::Final(_) => {}
            },
            Ok(MultiTurnStreamItem::StreamUserItem(_)) => {}
            Ok(MultiTurnStreamItem::FinalResponse(fr)) => {
                let txt = fr.response().to_string();
                if !txt.is_empty() {
                    last_final = txt;
                }
            }
            Ok(_) => {}
            Err(e) => {
                if thinking_open {
                    let _ = events.send(Event::AgentThinkingEnd { agent });
                }
                return Err(anyhow!("streaming prompt failed: {e}"));
            }
        }
    }

    if thinking_open {
        let _ = events.send(Event::AgentThinkingEnd { agent });
    }

    let text = if !last_final.is_empty() {
        last_final
    } else {
        current_turn_text
    };

    Ok(DriveOutcome::Finished { text })
}

/// Convenience wrapper that swallows steering and returns just the text,
/// for call sites that don't yet implement a steering loop.
pub async fn drive_text<M, P>(
    req: StreamingPromptRequest<M, P>,
    agent: AgentKind,
    events: &EventSender,
    pause: Option<&Arc<PauseController>>,
) -> Result<String>
where
    M: CompletionModel + 'static,
    <M as CompletionModel>::StreamingResponse: Send,
    P: PromptHook<M> + Send + 'static,
{
    Ok(drive(req, agent, events, pause).await?.text().to_string())
}
