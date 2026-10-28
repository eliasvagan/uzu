use std::any::Any;

use chrono::Local;
use iocraft::prelude::*;
use nagare::{
    chat::{ChatSession, ChatSessionError, ChatSessionStreamChunk},
    tool::uzu_tool_function,
};
use shoji::types::{
    basic::{CancelToken, SamplingSeed, ToolCall},
    model::Model,
    session::chat::{ChatConfig, ChatMessage, ChatReplyConfig, ChatReplyStats, ChatRole},
};
use uzu::engine::Engine;

use crate::interactive::{
    components::{ApplicationState, HistoryCellType, TranscriptItem},
    helpers::HINT_SESSION_INTERRUPT,
    sessions::{
        SessionState,
        tools::{list_directory, list_epub_chapters, read_epub_chapter, read_text_file, search_text},
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChatSessionStatus {
    Idle,
    Loading,
    Generating,
    Compacting,
}

#[derive(Clone)]
pub struct ChatSessionState {
    session: Option<ChatSession>,
    pending_items: Vec<TranscriptItem>,
    pending_stats: Option<ChatReplyStats>,
    cancel_token: Option<CancelToken>,
    status: ChatSessionStatus,
    context_used: u32,
    context_limit: Option<u32>,
    context_exact: bool,
}

impl ChatSessionState {
    pub fn loading() -> Self {
        Self {
            session: None,
            pending_items: Vec::new(),
            pending_stats: None,
            cancel_token: None,
            status: ChatSessionStatus::Loading,
            context_used: 0,
            context_limit: None,
            context_exact: true,
        }
    }

    pub fn idle(
        session: ChatSession,
        context_limit: Option<u32>,
    ) -> Self {
        Self {
            session: Some(session),
            pending_items: Vec::new(),
            pending_stats: None,
            cancel_token: None,
            status: ChatSessionStatus::Idle,
            context_used: 0,
            context_limit,
            context_exact: true,
        }
    }
}

impl SessionState for ChatSessionState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn is_busy(&self) -> bool {
        matches!(
            self.status,
            ChatSessionStatus::Loading | ChatSessionStatus::Generating | ChatSessionStatus::Compacting
        )
    }

    fn interrupt(&self) -> bool {
        let Some(cancel_token) = self.cancel_token.as_ref() else {
            return false;
        };
        cancel_token.cancel();
        true
    }

    fn status_text(&self) -> Option<String> {
        let activity = match self.status {
            ChatSessionStatus::Idle => "loaded".to_string(),
            ChatSessionStatus::Loading => "loading".to_string(),
            ChatSessionStatus::Generating => format!("generating ({HINT_SESSION_INTERRUPT})"),
            ChatSessionStatus::Compacting => format!("compacting ({HINT_SESSION_INTERRUPT})"),
        };
        let context = (self.context_limit.is_some() || self.context_used > 0)
            .then(|| super::compact::format_context(self.context_used, self.context_limit, self.context_exact));
        Some(match context {
            Some(context) => format!("{context} · {activity}"),
            None => activity,
        })
    }

    fn pending_history_cell(&self) -> Option<HistoryCellType> {
        if self.pending_items.is_empty() && self.pending_stats.is_none() {
            return None;
        }
        Some(HistoryCellType::ChatTranscript {
            items: self.pending_items.clone(),
            stats: self.pending_stats.clone(),
        })
    }
}

pub async fn ensure_session(
    state: State<ApplicationState>,
    model: &Model,
) -> Option<ChatSession> {
    let mut state = state;
    {
        let state = state.read();
        if let Some(session) = chat_state(&state).and_then(|chat_state| chat_state.session.clone()) {
            return Some(session);
        }
    }

    {
        let mut state = state.write();
        if let Some(model_state) = state.model_state.as_mut() {
            model_state.session_state = Some(Box::new(ChatSessionState::loading()));
        }
    }

    let (engine, seed, no_tools) = {
        let state = state.read();
        (state.engine.clone(), state.seed, state.no_tools)
    };
    let session = match create_session(&engine, model, seed, no_tools).await {
        Ok(session) => session,
        Err(error) => {
            let mut state = state.write();
            if let Some(model_state) = state.model_state.as_mut() {
                model_state.session_state = None;
            }
            state.history.push(HistoryCellType::CommandResult {
                result: format!("Failed to load session: {}", error),
            });
            return None;
        },
    };

    let sampling_defaults = session.sampling_defaults().await.unwrap_or_default();
    let context_limit = session.context_limit().await;
    {
        let mut state = state.write();
        if let Some(model_state) = state.model_state.as_mut() {
            model_state.sampling_defaults = sampling_defaults;
            model_state.session_state = Some(Box::new(ChatSessionState::idle(session.clone(), context_limit)));
        }
    }
    Some(session)
}

pub async fn create_session(
    engine: &Engine,
    model: &Model,
    seed: Option<i64>,
    no_tools: bool,
) -> anyhow::Result<ChatSession> {
    let mut config = ChatConfig::default();
    if let Some(seed) = seed {
        config = config.with_sampling_seed(SamplingSeed::Custom {
            seed,
        });
    }
    let mut session = engine.chat(model.clone(), config).await?;
    if !no_tools && session.supports_tool_calls().await {
        session.add_tool(get_current_date_time).await?;
        session.add_tool(sleep).await?;
        session.add_tool(list_directory).await?;
        session.add_tool(read_text_file).await?;
        session.add_tool(search_text).await?;
        session.add_tool(list_epub_chapters).await?;
        session.add_tool(read_epub_chapter).await?;
    }
    Ok(session)
}

pub async fn run_session(
    state: State<ApplicationState>,
    session: ChatSession,
    text: String,
) {
    let mut state = state;
    let (thinking_support, thinking_override, thinking) = {
        let state = state.read();
        (
            state.model_state.as_ref().map(|model_state| model_state.thinking).unwrap_or_default(),
            state.reasoning_effort_override(),
            state.thinking(),
        )
    };

    let reasoning_effort = match thinking_override {
        Some(effort) => match thinking_support.fulfill_requested_effort(effort) {
            Ok(effort) => effort,
            Err(error) => {
                state.write().history.push(HistoryCellType::CommandResult {
                    result: format!("Invalid --reasoning-effort: {error}"),
                });
                return;
            },
        },
        None => thinking_support.with_preference(&thinking).reasoning_effort(),
    };

    let mut messages = Vec::<ChatMessage>::new();
    if let Some(reasoning_effort) = reasoning_effort
        && session.messages().await.is_empty()
    {
        let system_message = ChatMessage::system().with_reasoning_effort(reasoning_effort);
        messages.push(system_message);
    };
    let prepared = super::attachments::prepare_message(&text);
    if let Some(note) = prepared.note {
        state.write().history.push(HistoryCellType::CommandResult {
            result: note,
        });
    }
    messages.push(ChatMessage::user().with_text(prepared.text));

    let reply_config = ChatReplyConfig::default().with_sampling_policy(state.read().preferences().sampling.policy());
    {
        let mut state = state.write();
        if let Some(model_state) = state.model_state.as_mut() {
            model_state.thinking_locked = true;
        }
        if let Some(chat_state) = chat_state_mut(&mut state) {
            chat_state.pending_items = Vec::new();
            chat_state.pending_stats = None;
            chat_state.cancel_token = None;
            chat_state.status = ChatSessionStatus::Generating;
        }
    }

    let history_offset = session.messages().await.len();
    let stream = session.reply_with_stream(messages, reply_config).await;
    let mut latest_stats: Option<ChatReplyStats> = None;
    {
        let mut state = state.write();
        if let Some(chat_state) = chat_state_mut(&mut state) {
            chat_state.cancel_token = Some(stream.cancel_token());
        }
    }

    while let Some(chunk) = stream.next().await {
        match chunk {
            ChatSessionStreamChunk::Replies {
                replies,
            } => {
                if let Some(reply) = replies.last() {
                    latest_stats = Some(reply.stats.clone());
                }
            },
            ChatSessionStreamChunk::ToolResults {
                ..
            } => {},
            ChatSessionStreamChunk::Error {
                error,
            } => {
                state.write().history.push(HistoryCellType::CommandResult {
                    result: format!("Chat error: {}", error),
                });
                break;
            },
        }
        let items = build_transcript(&session.messages().await, history_offset);
        let mut state = state.write();
        if let Some(chat_state) = chat_state_mut(&mut state) {
            chat_state.pending_items = items;
            chat_state.pending_stats = latest_stats.clone();
        }
    }

    let final_items = build_transcript(&session.messages().await, history_offset);
    let encoded_tokens = session.encoded_token_count().await;
    let mut state = state.write();
    if let Some(chat_state) = chat_state_mut(&mut state) {
        chat_state.pending_items = Vec::new();
        chat_state.pending_stats = None;
        chat_state.cancel_token = None;
        chat_state.status = ChatSessionStatus::Idle;
        if let Some(encoded_tokens) = encoded_tokens {
            chat_state.context_used = encoded_tokens;
            chat_state.context_exact = true;
        }
    }
    if !final_items.is_empty() || latest_stats.is_some() {
        state.history.push(HistoryCellType::ChatTranscript {
            items: final_items,
            stats: latest_stats,
        });
    }
}

const COMPACT_TOKEN_LIMIT: u32 = 4_096;

pub async fn compact(state: State<ApplicationState>) {
    let mut state = state;
    let (session, rejected) = {
        let snapshot = state.read();
        let rejected = match snapshot.model_state.as_ref() {
            None => Some("Load a chat model first."),
            Some(model_state) if !model_state.model.is_chat_capable() => Some("Compact is for chat models."),
            Some(_) => None,
        };
        let session = chat_state(&snapshot).and_then(|chat_state| chat_state.session.clone());
        (session, rejected)
    };
    if let Some(rejected) = rejected {
        push_result(&mut state, rejected);
        return;
    }
    let Some(session) = session else {
        push_result(&mut state, "Start a chat before compacting.");
        return;
    };

    let original = session.messages().await;
    if !super::compact::has_conversation(&original) {
        push_result(&mut state, "Nothing to compact yet.");
        return;
    }

    let (thinking, sampling, recorded_tokens) = {
        let snapshot = state.read();
        (
            snapshot.model_state.as_ref().map(|model_state| model_state.thinking).unwrap_or_default(),
            snapshot.preferences().sampling.policy(),
            chat_state(&snapshot).map(|chat_state| chat_state.context_used).unwrap_or(0),
        )
    };
    let from_tokens = if recorded_tokens > 0 {
        recorded_tokens
    } else {
        session.encoded_token_count().await.unwrap_or(0)
    };

    {
        let mut snapshot = state.write();
        if let Some(chat_state) = chat_state_mut(&mut snapshot) {
            chat_state.pending_items.clear();
            chat_state.pending_stats = None;
            chat_state.cancel_token = None;
            chat_state.status = ChatSessionStatus::Compacting;
        }
    }

    let mut seed = Vec::new();
    if let Some(effort) = super::compact::compact_effort(thinking) {
        seed.push(ChatMessage::system().with_reasoning_effort(effort));
    }
    let request = super::compact::compact_request(&original, session.context_limit().await);
    if let Err(error) = session.replace_messages(seed).await {
        let _ = session.replace_messages(original).await;
        settle_compact(&mut state, None);
        push_result(&mut state, &compact_failure(&error));
        return;
    }

    let reply_config = ChatReplyConfig::default()
        .with_sampling_policy(sampling)
        .with_tool_turn_limit(Some(0))
        .with_token_limit(Some(COMPACT_TOKEN_LIMIT));
    let stream = session.reply_with_stream(vec![ChatMessage::user().with_text(request)], reply_config).await;
    let cancel_token = stream.cancel_token();
    {
        let mut snapshot = state.write();
        if let Some(chat_state) = chat_state_mut(&mut snapshot) {
            chat_state.cancel_token = Some(cancel_token.clone());
        }
    }

    let mut summary = None;
    let mut failure = None;
    while let Some(chunk) = stream.next().await {
        match chunk {
            ChatSessionStreamChunk::Replies {
                replies,
            } => {
                if let Some(reply) = replies.last() {
                    summary = reply.message.text().filter(|text| !text.trim().is_empty());
                    if let Some(text) = reply.message.text().filter(|text| !text.is_empty()) {
                        let mut snapshot = state.write();
                        if let Some(chat_state) = chat_state_mut(&mut snapshot) {
                            chat_state.pending_items = vec![TranscriptItem::Text(text)];
                        }
                    }
                }
            },
            ChatSessionStreamChunk::ToolResults {
                ..
            } => {},
            ChatSessionStreamChunk::Error {
                error,
            } => {
                failure = Some(compact_failure(&error));
                break;
            },
        }
    }

    if cancel_token.is_cancelled() {
        let _ = session.replace_messages(original).await;
        settle_compact(&mut state, None);
        push_result(&mut state, "Compact cancelled. The conversation was kept.");
        return;
    }
    if let Some(failure) = failure {
        let _ = session.replace_messages(original).await;
        settle_compact(&mut state, None);
        push_result(&mut state, &failure);
        return;
    }
    let Some(summary) = summary.map(|summary| summary.trim().to_string()).filter(|summary| !summary.is_empty()) else {
        let _ = session.replace_messages(original).await;
        settle_compact(&mut state, None);
        push_result(&mut state, "The summary was empty. The conversation was kept.");
        return;
    };

    let kept = super::compact::compacted_messages(&original, &summary);
    let counted_text = kept.iter().filter_map(|message| message.text()).collect::<Vec<_>>().join("\n");
    if let Err(error) = session.replace_messages(kept).await {
        let _ = session.replace_messages(original).await;
        settle_compact(&mut state, None);
        push_result(&mut state, &compact_failure(&error));
        return;
    }

    let summary_tokens = session
        .count_text_tokens(&counted_text)
        .await
        .unwrap_or_else(|| u32::try_from(counted_text.chars().count() / 4).unwrap_or(u32::MAX));
    settle_compact(&mut state, Some(summary_tokens));
    let into = super::compact::format_token_count(summary_tokens);
    let note = if from_tokens > 0 {
        format!(
            "Compacted {} tokens into ~{into}. The model now sees only this summary.",
            super::compact::format_token_count(from_tokens)
        )
    } else {
        format!("Compacted the conversation into ~{into}. The model now sees only this summary.")
    };
    let mut snapshot = state.write();
    snapshot.history.push(HistoryCellType::CommandResult {
        result: note,
    });
    snapshot.history.push(HistoryCellType::ChatTranscript {
        items: vec![TranscriptItem::Text(summary)],
        stats: None,
    });
}

fn settle_compact(
    state: &mut State<ApplicationState>,
    summary_tokens: Option<u32>,
) {
    let mut snapshot = state.write();
    if let Some(chat_state) = chat_state_mut(&mut snapshot) {
        chat_state.pending_items.clear();
        chat_state.pending_stats = None;
        chat_state.cancel_token = None;
        chat_state.status = ChatSessionStatus::Idle;
        if let Some(summary_tokens) = summary_tokens {
            chat_state.context_used = summary_tokens;
            chat_state.context_exact = false;
        }
    }
}

fn compact_failure(error: &ChatSessionError) -> String {
    if matches!(error, ChatSessionError::ToolTurnLimitExceeded { .. }) {
        "The model tried to call a tool instead of summarizing. The conversation was kept.".to_string()
    } else {
        format!("Could not compact: {error}. The conversation was kept.")
    }
}

fn push_result(
    state: &mut State<ApplicationState>,
    result: &str,
) {
    state.write().history.push(HistoryCellType::CommandResult {
        result: result.to_string(),
    });
}

/// A tool call is `calling` until its result message shows up in the history,
/// then it becomes `called`.
pub fn build_transcript(
    messages: &[ChatMessage],
    history_offset: usize,
) -> Vec<TranscriptItem> {
    let messages = &messages[history_offset.min(messages.len())..];
    let mut items = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if message.role != (ChatRole::Assistant {}) {
            continue;
        }
        if let Some(reasoning) =
            message.reasoning().map(|content| content.trim().to_string()).filter(|content| !content.is_empty())
        {
            items.push(TranscriptItem::Thinking(reasoning));
        }
        if let Some(text) = message.text().filter(|content| !content.is_empty()) {
            items.push(TranscriptItem::Text(text));
        }
        for tool_call in message.tool_calls() {
            items.push(TranscriptItem::ToolCall {
                name: tool_call.name.clone(),
                called: tool_call_called(messages, index, &tool_call),
            });
        }
    }
    items
}

fn tool_call_called(
    messages: &[ChatMessage],
    index: usize,
    tool_call: &ToolCall,
) -> bool {
    messages[index + 1..].iter().any(|message| {
        message.role == (ChatRole::Tool {})
            && message.tool_call_results().iter().any(|(identifier, _, _)| match (&tool_call.identifier, identifier) {
                (Some(call_identifier), Some(result_identifier)) => call_identifier == result_identifier,
                _ => true,
            })
    })
}

fn chat_state(state: &ApplicationState) -> Option<&ChatSessionState> {
    state
        .model_state
        .as_ref()
        .and_then(|model_state| model_state.session_state.as_deref())
        .and_then(|session_state| session_state.as_any().downcast_ref::<ChatSessionState>())
}

fn chat_state_mut(state: &mut ApplicationState) -> Option<&mut ChatSessionState> {
    state
        .model_state
        .as_mut()
        .and_then(|model_state| model_state.session_state.as_deref_mut())
        .and_then(|session_state| session_state.as_any_mut().downcast_mut::<ChatSessionState>())
}

/// Returns the current date and time in UTC and the device's local time, with explicit UTC offsets.
#[uzu_tool_function]
fn get_current_date_time() -> String {
    let now = Local::now();
    format!(
        "Current UTC date and time: {}.\nDevice local date and time: {}.",
        now.with_timezone(&chrono::Utc).format("%A, %-d %B %Y at %H:%M:%S (UTC%:z)"),
        now.format("%A, %-d %B %Y at %H:%M:%S (UTC%:z)"),
    )
}

/// Sleeps for the given number of seconds before responding, at most 60 seconds.
#[uzu_tool_function]
async fn sleep(
    /// Number of seconds to sleep, at most 60.
    seconds: f64
) -> Result<String, String> {
    if !(0.0..=60.0).contains(&seconds) {
        return Err(format!("Refusing to sleep for {} seconds, must be between 0 and 60", seconds));
    }
    tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await;
    Ok(format!("Slept for {} seconds", seconds))
}
