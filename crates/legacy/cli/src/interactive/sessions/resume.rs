use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use shoji::types::session::chat::{ChatMessage, ChatRole};

use super::compact::has_conversation;
use crate::interactive::components::HistoryCellType;

const PREVIEW_CHARS: usize = 72;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedChat {
    pub model_id: String,
    pub model_name: String,
    pub messages: Vec<ChatMessage>,
    pub history: Vec<HistoryCellType>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResumeOffer {
    pub summary: String,
    pub preview: Option<String>,
}

impl SavedChat {
    pub fn capture(
        model_id: impl Into<String>,
        model_name: impl Into<String>,
        messages: Vec<ChatMessage>,
        history: Vec<HistoryCellType>,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            model_name: model_name.into(),
            messages,
            history,
        }
    }

    pub fn can_resume(&self) -> bool {
        !self.model_id.trim().is_empty() && has_conversation(&self.messages)
    }

    pub fn offer(&self) -> ResumeOffer {
        let name = if self.model_name.trim().is_empty() {
            self.model_id.clone()
        } else {
            self.model_name.clone()
        };
        let turns = self.turn_count();
        let label = if turns == 1 {
            "turn"
        } else {
            "turns"
        };
        ResumeOffer {
            summary: format!("{name} · {turns} {label}"),
            preview: self.opening_line(),
        }
    }

    fn turn_count(&self) -> usize {
        let requests = self.history.iter().filter(|cell| matches!(cell, HistoryCellType::Request { .. })).count();
        if requests > 0 {
            return requests;
        }
        self.messages.iter().filter(|message| message.role == ChatRole::User {}).count()
    }

    fn opening_line(&self) -> Option<String> {
        let text = self
            .history
            .iter()
            .find_map(|cell| match cell {
                HistoryCellType::Request {
                    text,
                } => Some(text.clone()),
                _ => None,
            })
            .or_else(|| {
                self.messages.iter().find_map(|message| match &message.role {
                    ChatRole::User {} => message.text(),
                    _ => None,
                })
            })?;
        let line = one_line(&text, PREVIEW_CHARS);
        if line.is_empty() {
            None
        } else {
            Some(line)
        }
    }
}

pub fn load_last_chat() -> Option<SavedChat> {
    let path = saved_chat_path().ok()?;
    let chat = load_at(&path)?;
    if chat.can_resume() {
        Some(chat)
    } else {
        None
    }
}

pub fn store_last_chat(chat: &SavedChat) {
    let Ok(path) = saved_chat_path() else {
        return;
    };
    if let Err(error) = store_at(chat, &path) {
        tracing::warn!("Unable to save the chat: {error}");
    }
}

pub fn clear_last_chat() {
    if let Ok(path) = saved_chat_path() {
        let _ = remove_at(&path);
    }
}

pub(crate) fn load_at(path: &Path) -> Option<SavedChat> {
    let bytes = fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(chat) => Some(chat),
        Err(error) => {
            tracing::warn!("Unable to read saved chat {}: {error}", path.display());
            None
        },
    }
}

pub(crate) fn store_at(
    chat: &SavedChat,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if !chat.can_resume() {
        return Ok(());
    }
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary_name = path.file_name().map(|name| {
        let mut temporary_name = name.to_os_string();
        temporary_name.push(".tmp");
        temporary_name
    });
    let Some(temporary_name) = temporary_name else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "session path has no file name").into());
    };
    let temporary = path.with_file_name(temporary_name);
    fs::write(&temporary, serde_json::to_vec_pretty(chat)?)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

pub(crate) fn remove_at(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn saved_chat_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let config = confy::get_configuration_file_path(crate::interactive::APP_IDENTIFIER, "config")?;
    Ok(config.with_file_name("session.json"))
}

fn one_line(
    text: &str,
    max_chars: usize,
) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let mut short = flat.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    short.push('…');
    short
}

#[cfg(test)]
#[path = "../../../unit/interactive/resume_test.rs"]
mod tests;
