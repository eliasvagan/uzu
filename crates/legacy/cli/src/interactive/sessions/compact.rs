use shoji::types::{
    basic::ReasoningEffort,
    session::chat::{ChatMessage, ChatRole},
};

use crate::common::thinking::ThinkingSupport;

const SUMMARY_RESERVE_TOKENS: u32 = 4_096;
const MIN_TRANSCRIPT_TOKENS: u32 = 1_024;
const CHARS_PER_TOKEN: u32 = 3;
const OMISSION: &str = "[earlier conversation omitted]\n\n";

pub(super) fn format_token_count(count: u32) -> String {
    if count < 1_000 {
        return count.to_string();
    }
    if count < 10_000 {
        let tenths = (count as f64 / 100.0).round() / 10.0;
        if tenths >= 10.0 {
            return "10k".to_string();
        }
        if (tenths - tenths.floor()).abs() < f64::EPSILON {
            return format!("{}k", tenths as u32);
        }
        return format!("{tenths:.1}k");
    }
    format!("{}k", (count as f64 / 1_000.0).round() as u32)
}

pub(super) fn format_context(
    used: u32,
    limit: Option<u32>,
    exact: bool,
) -> String {
    let used_text = format_token_count(used);
    let used_text = if exact {
        used_text
    } else {
        format!("~{used_text}")
    };
    match limit {
        Some(limit) => format!("context {used_text}/{}", format_token_count(limit)),
        None => format!("context {used_text}"),
    }
}

pub(super) fn has_conversation(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|message| message_transcript(message).is_some())
}

/// Short reasoning for the summary turn when the model can do it.
/// The kept conversation goes back to the effort it had before.
pub(super) fn compact_effort(thinking: ThinkingSupport) -> Option<ReasoningEffort> {
    if matches!(thinking, ThinkingSupport::Toggle(_)) {
        return thinking.fulfill_requested_effort(ReasoningEffort::Disabled).ok().flatten();
    }
    thinking
        .fulfill_requested_effort(ReasoningEffort::Low)
        .ok()
        .flatten()
        .or_else(|| thinking.fulfill_requested_effort(ReasoningEffort::Disabled).ok().flatten())
}

pub(super) fn transcript_char_budget(limit: Option<u32>) -> usize {
    let tokens = limit.unwrap_or(16_384).saturating_sub(SUMMARY_RESERVE_TOKENS).max(MIN_TRANSCRIPT_TOKENS);
    tokens.saturating_mul(CHARS_PER_TOKEN) as usize
}

pub(super) fn compact_request(
    messages: &[ChatMessage],
    limit: Option<u32>,
) -> String {
    let transcript = transcript_within(messages, transcript_char_budget(limit));
    format!(
        "Summarize the conversation below so it can be continued from the summary alone. \
Keep decisions, names, numbers, paths, constraints, and open questions. \
Omit greetings and filler. Do not call tools. Reply with the summary only.\n\n\
{transcript}"
    )
}

pub(super) fn compacted_messages(
    original: &[ChatMessage],
    summary: &str,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    if let Some(effort) = original
        .iter()
        .find_map(|message| (message.role == ChatRole::System {}).then(|| message.reasoning_effort()).flatten())
    {
        messages.push(ChatMessage::system().with_reasoning_effort(effort));
    }
    messages.push(ChatMessage::user().with_text(format!(
        "The earlier conversation was compacted into this summary. Continue from it.\n\n{summary}"
    )));
    messages
}

fn transcript_within(
    messages: &[ChatMessage],
    max_chars: usize,
) -> String {
    let mut parts = Vec::new();
    let mut used = 0usize;
    let mut omitted = false;
    for message in messages.iter().rev() {
        let Some(part) = message_transcript(message) else {
            continue;
        };
        let separator = if parts.is_empty() {
            0
        } else {
            2
        };
        if used + separator + part.chars().count() > max_chars {
            omitted = true;
            break;
        }
        used += separator + part.chars().count();
        parts.push(part);
    }
    parts.reverse();

    if parts.is_empty() {
        let Some(part) = messages.iter().rev().find_map(message_transcript) else {
            return String::new();
        };
        let marker = OMISSION.chars().count();
        let tail = take_tail(&part, max_chars.saturating_sub(marker));
        return format!("{OMISSION}{tail}");
    }

    let body = parts.join("\n\n");
    if omitted {
        format!("{OMISSION}{body}")
    } else {
        body
    }
}

fn message_transcript(message: &ChatMessage) -> Option<String> {
    match &message.role {
        ChatRole::User {} => message.text().map(|text| format!("User:\n{text}")),
        ChatRole::Assistant {} => {
            let text = message.text().unwrap_or_default();
            let tools = message.tool_calls().into_iter().map(|call| call.name).collect::<Vec<_>>();
            if text.is_empty() && tools.is_empty() {
                return None;
            }
            let mut body = String::new();
            if !text.is_empty() {
                body.push_str(&format!("Assistant:\n{text}"));
            }
            if !tools.is_empty() {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&format!("Assistant called: {}", tools.join(", ")));
            }
            Some(body)
        },
        ChatRole::Tool {} => {
            let results = message.tool_call_results();
            if results.is_empty() {
                return None;
            }
            let lines = results
                .into_iter()
                .map(|(_, name, value)| {
                    let name = name.unwrap_or_else(|| "tool".to_string());
                    format!("Tool {name}: {}", take_tail(&value.json, 400))
                })
                .collect::<Vec<_>>();
            Some(lines.join("\n"))
        },
        _ => None,
    }
}

fn take_tail(
    text: &str,
    max_chars: usize,
) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    text.chars().skip(count - max_chars).collect()
}

#[cfg(test)]
#[path = "../../../unit/interactive/compact_test.rs"]
mod tests;
