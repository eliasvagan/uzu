use shoji::types::{
    basic::{ReasoningEffort, ToolCall, Value},
    session::chat::{ChatContentBlock, ChatMessage},
};

use super::*;
use crate::common::thinking::ReasoningEffortSet;

fn user(text: &str) -> ChatMessage {
    ChatMessage::user().with_text(text.to_string())
}

fn assistant(text: &str) -> ChatMessage {
    ChatMessage::assistant().with_text(text.to_string())
}

#[test]
fn token_counts_use_a_short_scale() {
    assert_eq!(format_token_count(0), "0");
    assert_eq!(format_token_count(900), "900");
    assert_eq!(format_token_count(1_000), "1k");
    assert_eq!(format_token_count(1_536), "1.5k");
    assert_eq!(format_token_count(18_432), "18k");
    assert_eq!(format_token_count(262_144), "262k");
}

#[test]
fn context_marks_an_estimate() {
    assert_eq!(format_context(18_432, Some(262_144), true), "context 18k/262k");
    assert_eq!(format_context(900, Some(262_144), false), "context ~900/262k");
    assert_eq!(format_context(12, None, true), "context 12");
}

#[test]
fn transcript_keeps_the_newest_messages() {
    let messages = vec![user("old"), assistant("older reply"), user("new"), assistant("newest reply")];
    let transcript =
        transcript_within(&messages, "User:\nnew".chars().count() + "Assistant:\nnewest reply".chars().count() + 2);
    assert!(transcript.starts_with("[earlier conversation omitted]"), "{transcript}");
    assert!(transcript.contains("User:\nnew"), "{transcript}");
    assert!(transcript.contains("Assistant:\nnewest reply"), "{transcript}");
    assert!(!transcript.contains("old"), "{transcript}");
}

#[test]
fn an_oversized_message_keeps_its_tail() {
    let messages = vec![user(&format!("START{}", "x".repeat(200)))];
    let transcript = transcript_within(&messages, 80);
    assert!(transcript.starts_with("[earlier conversation omitted]"), "{transcript}");
    assert!(transcript.ends_with('x'), "{transcript}");
    assert!(!transcript.contains("START"), "{transcript}");
}

#[test]
fn compacted_history_keeps_the_original_effort_and_the_summary() {
    let original =
        vec![ChatMessage::system().with_reasoning_effort(ReasoningEffort::XHigh), user("hello"), assistant("hi")];
    let kept = compacted_messages(&original, "They said hello.");
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].reasoning_effort(), Some(ReasoningEffort::XHigh));
    assert!(kept[1].text().unwrap().contains("They said hello."));
    assert!(!has_conversation(&kept[..1]));
    assert!(has_conversation(&original));
}

#[test]
fn tool_calls_are_named_in_the_transcript() {
    let messages = vec![ChatMessage::assistant().with_tool_call(ToolCall {
        identifier: Some("1".to_string()),
        name: "read_text_file".to_string(),
        arguments: Value::from(serde_json::json!({})),
    })];
    let transcript = transcript_within(&messages, 200);
    assert!(transcript.contains("Assistant called: read_text_file"), "{transcript}");
}

#[test]
fn compact_effort_prefers_a_short_summary() {
    let levels = ThinkingSupport::Levels {
        effort: ReasoningEffort::Default,
        supported: ReasoningEffortSet::ALL,
    };
    assert_eq!(compact_effort(levels), Some(ReasoningEffort::Low));
    assert_eq!(compact_effort(ThinkingSupport::Toggle(true)), Some(ReasoningEffort::Disabled));
    assert_eq!(compact_effort(ThinkingSupport::AlwaysOn), None);
    assert_eq!(compact_effort(ThinkingSupport::Unsupported), None);
}

#[test]
fn compact_request_forbids_tools() {
    let request = compact_request(&[user("remember the path /tmp/notes.txt")], Some(8_192));
    assert!(request.contains("Do not call tools"));
    assert!(request.contains("/tmp/notes.txt"));
}

#[test]
fn tool_results_stay_in_the_transcript() {
    let messages = vec![ChatMessage::tool().with_block(ChatContentBlock::ToolCallResult {
        identifier: Some("1".to_string()),
        name: Some("read_text_file".to_string()),
        value: Value::from(serde_json::json!("alpha")),
    })];
    let transcript = transcript_within(&messages, 200);
    assert!(transcript.contains("Tool read_text_file"), "{transcript}");
    assert!(transcript.contains("alpha"), "{transcript}");
}
