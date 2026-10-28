use shoji::types::session::chat::{ChatMessage, ChatReplyStats};

use super::{SavedChat, load_at, remove_at, store_at};
use crate::interactive::components::{HistoryCellType, TranscriptItem};

#[test]
fn a_saved_chat_round_trips_and_describes_itself() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.json");
    let chat = SavedChat::capture(
        "alibaba:qwen3.8:27b",
        "Qwen3.8",
        vec![
            ChatMessage::system().with_text("stay brief".to_string()),
            ChatMessage::user().with_text("Write about the\ngarden café".to_string()),
            ChatMessage::assistant().with_text("The soil remembers.".to_string()),
        ],
        vec![
            HistoryCellType::Request {
                text: "Write about the\ngarden café".to_string(),
            },
            HistoryCellType::ChatTranscript {
                items: vec![TranscriptItem::Text("The soil remembers.".to_string())],
                stats: Some(ChatReplyStats::default()),
            },
        ],
    );

    store_at(&chat, &path).unwrap();
    let loaded = load_at(&path).unwrap();
    assert_eq!(loaded, chat);
    assert!(loaded.can_resume());
    let offer = loaded.offer();
    assert_eq!(offer.summary, "Qwen3.8 · 1 turn");
    assert_eq!(offer.preview.as_deref(), Some("Write about the garden café"));

    remove_at(&path).unwrap();
    assert!(load_at(&path).is_none());
    remove_at(&path).unwrap();
}

#[test]
fn a_chat_with_no_conversation_is_not_saved() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.json");
    let chat = SavedChat::capture(
        "model",
        "Model",
        vec![ChatMessage::system().with_text("only a setting".to_string())],
        Vec::new(),
    );

    assert!(!chat.can_resume());
    store_at(&chat, &path).unwrap();
    assert!(!path.exists());
}

#[test]
fn a_long_opening_line_is_shortened_to_one_line() {
    let chat = SavedChat::capture(
        "model",
        "",
        vec![ChatMessage::user().with_text("hello".to_string())],
        vec![HistoryCellType::Request {
            text: format!("  start   {}\nmore", "word ".repeat(40)),
        }],
    );

    let offer = chat.offer();
    assert!(offer.summary.starts_with("model · 1 turn"), "{}", offer.summary);
    let preview = offer.preview.unwrap();
    assert!(!preview.contains('\n'), "{preview}");
    assert!(preview.ends_with('…'), "{preview}");
    assert!(preview.chars().count() <= 72, "{preview}");
}

#[test]
fn unreadable_json_is_ignored() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.json");
    std::fs::write(&path, "{not json").unwrap();
    assert!(load_at(&path).is_none());
}
