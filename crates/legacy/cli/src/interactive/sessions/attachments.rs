use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::interactive::sessions::tools::{list_directory, read_epub_chapter, read_text_file};

const MAX_ATTACHMENTS: usize = 5;

pub struct PreparedMessage {
    pub text: String,
    pub note: Option<String>,
}

pub fn prepare_message(message: &str) -> PreparedMessage {
    let mut paths = find_paths(message);
    let hidden = paths.len().saturating_sub(MAX_ATTACHMENTS);
    paths.truncate(MAX_ATTACHMENTS);
    if paths.is_empty() {
        return PreparedMessage {
            text: message.to_string(),
            note: None,
        };
    }

    let mut text = message.to_string();
    text.push_str("\n\nThe user attached these local files. Answer from their contents.\n");
    let mut notes = Vec::new();
    for path in &paths {
        let (kind, body) = read_attachment(path);
        notes.push(format!("Attached {} ({kind})", path.display()));
        text.push_str(&format!("\n<file path=\"{}\">\n{body}\n</file>\n", path.display()));
    }
    if hidden > 0 {
        notes.push(format!("{hidden} more paths were not attached"));
    }
    PreparedMessage {
        text,
        note: Some(notes.join("\n")),
    }
}

fn read_attachment(path: &Path) -> (String, String) {
    let display = path.display().to_string();
    if is_epub(path) {
        return match read_epub_chapter::call(display, 1) {
            Ok(body) => ("EPUB chapter 1".to_string(), body),
            Err(error) => ("unreadable".to_string(), error),
        };
    }
    if path.is_dir() {
        return match list_directory::call(display) {
            Ok(body) => ("directory".to_string(), body),
            Err(error) => ("unreadable".to_string(), error),
        };
    }
    match read_text_file::call(display, None, None) {
        Ok(body) => ("text".to_string(), body),
        Err(error) => ("unreadable".to_string(), error),
    }
}

fn is_epub(path: &Path) -> bool {
    if path.is_dir() {
        return path.join("META-INF/container.xml").is_file() || path.join("content.opf").is_file();
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
}

fn find_paths(message: &str) -> Vec<PathBuf> {
    let chars: Vec<char> = message.chars().collect();
    let mut found = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let at_boundary = index == 0 || chars[index - 1].is_whitespace();
        if at_boundary && matches!(chars[index], '"' | '\'') {
            let quote = chars[index];
            if let Some(end) = chars[index + 1..].iter().position(|character| *character == quote) {
                let inner: String = chars[index + 1..index + 1 + end].iter().collect();
                remember(&mut found, &inner);
                index += end + 2;
                continue;
            }
        }
        if at_boundary && starts_path(&chars[index..]) {
            if let Some((path, consumed)) = longest_existing_path(&chars[index..]) {
                let canonical = fs::canonicalize(&path).unwrap_or(path);
                if !found.iter().any(|existing| existing == &canonical) {
                    found.push(canonical);
                }
                index += consumed.max(1);
                continue;
            }
        }
        index += 1;
    }

    remember(&mut found, message.trim());
    found
}

/// The longest existing path on this line, starting at `chars`.
///
/// A dropped or pasted path may contain spaces. Shell-escaped spaces stay
/// inside the first token. A raw space extends the match only when a longer
/// path exists, so `/tmp/present/missing name` does not attach `/tmp/present`.
fn longest_existing_path(chars: &[char]) -> Option<(PathBuf, usize)> {
    let line_len = chars.iter().position(|character| *character == '\n').unwrap_or(chars.len());
    let line = &chars[..line_len];
    let first_len = path_token_len(line);
    if first_len == 0 {
        return None;
    }

    let mut end = line.len();
    while end >= first_len {
        let mut candidate_end = end;
        while candidate_end > 0 && line[candidate_end - 1].is_whitespace() {
            candidate_end -= 1;
        }
        if candidate_end < first_len {
            break;
        }
        let raw: String = line[..candidate_end].iter().collect();
        if let Some(path) = existing_path(&raw) {
            return Some((path, candidate_end));
        }
        if candidate_end == first_len {
            break;
        }
        let Some(space_at) = line[..candidate_end].iter().rposition(|character| character.is_whitespace()) else {
            break;
        };
        if space_at < first_len {
            break;
        }
        end = space_at;
    }
    None
}

fn path_token_len(chars: &[char]) -> usize {
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' && index + 1 < chars.len() {
            index += 2;
            continue;
        }
        if chars[index].is_whitespace() {
            break;
        }
        index += 1;
    }
    index
}

fn remember(
    found: &mut Vec<PathBuf>,
    token: &str,
) {
    let Some(path) = existing_path(token) else {
        return;
    };
    let canonical = fs::canonicalize(&path).unwrap_or(path);
    if !found.iter().any(|existing| existing == &canonical) {
        found.push(canonical);
    }
}

fn existing_path(token: &str) -> Option<PathBuf> {
    let unescaped = unescape(token);
    let candidates = [unescaped.clone(), trim_punctuation(&unescaped)];
    candidates.into_iter().find_map(|candidate| {
        if candidate.is_empty() || candidate == "/" {
            return None;
        }
        let path = expand_home(&candidate);
        fs::metadata(&path).is_ok().then_some(path)
    })
}

fn trim_punctuation(token: &str) -> String {
    token
        .trim_end_matches(|character: char| matches!(character, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'))
        .to_string()
}

fn starts_path(chars: &[char]) -> bool {
    matches!(chars.first(), Some('/' | '~')) || chars.starts_with(&['.', '/']) || chars.starts_with(&['.', '.', '/'])
}

fn unescape(token: &str) -> String {
    let mut text = String::new();
    let mut characters = token.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(escaped) = characters.next() {
                text.push(escaped);
            }
        } else {
            text.push(character);
        }
    }
    text
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
#[path = "../../../unit/interactive/attachments_test.rs"]
mod tests;
