use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

use nagare::tool::uzu_tool_function;
use zip::{ZipArchive, result::ZipError};

const MAX_DIRECTORY_ENTRIES: usize = 300;
const DEFAULT_TEXT_LINES: u32 = 200;
const MAX_TEXT_LINES: u32 = 400;
const MAX_TEXT_CHARS: usize = 48_000;
const MAX_TEXT_FILE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_SEARCH_MATCHES: u32 = 15;
const MAX_SEARCH_MATCHES: u32 = 30;
const MAX_SEARCH_FILES: usize = 2_000;
const MAX_SEARCH_DEPTH: usize = 4;
const MAX_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_QUERY_CHARS: usize = 200;
const MAX_EPUB_ENTRY_BYTES: usize = 8 * 1024 * 1024;
const MAX_EPUB_TOTAL_BYTES: usize = 20 * 1024 * 1024;
const MAX_CHAPTER_CHARS: usize = 48_000;
const MAX_CHAPTERS_LISTED: usize = 200;
const SHORT_BLOCK_CHARS: usize = 400;
const SUBSTANTIAL_SPINE_CHARS: usize = 800;

/// List entries in a directory. Does not enter subdirectories. Returns at most 300 names, with file sizes.
#[uzu_tool_function]
pub(crate) fn list_directory(
    /// Absolute or relative path of the directory.
    path: String
) -> Result<String, String> {
    let directory = PathBuf::from(&path);
    let metadata = fs::metadata(&directory).map_err(|error| format!("Cannot read {path}: {error}"))?;
    if !metadata.is_dir() {
        return Err(format!("{path} is a file ({}); use read_text_file to read it", format_bytes(metadata.len())));
    }
    let mut entries = fs::read_dir(&directory)
        .map_err(|error| format!("Cannot read {path}: {error}"))?
        .map(|entry| {
            let entry = entry.map_err(|error| format!("Cannot read an entry in {path}: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let metadata = entry.metadata().map_err(|error| format!("Cannot read {name}: {error}"))?;
            let kind = if metadata.is_dir() {
                EntryKind::Directory
            } else if metadata.is_symlink() {
                EntryKind::Symlink
            } else {
                EntryKind::File {
                    bytes: metadata.len(),
                }
            };
            Ok(DirectoryEntry {
                name,
                kind,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    entries.sort_by(|left, right| left.kind.rank().cmp(&right.kind.rank()).then_with(|| left.name.cmp(&right.name)));
    let total = entries.len();
    let mut lines = Vec::new();
    lines.push(directory.display().to_string());
    for entry in entries.iter().take(MAX_DIRECTORY_ENTRIES) {
        lines.push(match entry.kind {
            EntryKind::Directory => format!("dir   {}/", entry.name),
            EntryKind::Symlink => format!("link  {}", entry.name),
            EntryKind::File {
                bytes,
            } => format!("file  {:>10}  {}", format_bytes(bytes), entry.name),
        });
    }
    if total > MAX_DIRECTORY_ENTRIES {
        lines.push(format!("{} more entries not shown", total - MAX_DIRECTORY_ENTRIES));
    } else if total == 0 {
        lines.push("(empty directory)".to_string());
    }
    Ok(lines.join("\n"))
}

/// Read lines from a UTF-8 text file. Refuses binary files and files larger than 8 MiB. Returns at most 400 lines and 48000 characters.
#[uzu_tool_function]
pub(crate) fn read_text_file(
    /// Absolute or relative path of the file.
    path: String,
    /// 1-based line to start at. Omit to start at line 1.
    start_line: Option<u32>,
    /// Number of lines to return, from 1 to 400. Omit for 200.
    line_count: Option<u32>,
) -> Result<String, String> {
    let start_line = start_line.unwrap_or(1);
    let line_count = line_count.unwrap_or(DEFAULT_TEXT_LINES);
    if start_line == 0 {
        return Err("start_line counts from 1".to_string());
    }
    if !(1..=MAX_TEXT_LINES).contains(&line_count) {
        return Err(format!("line_count must be from 1 to {MAX_TEXT_LINES}"));
    }
    let file = PathBuf::from(&path);
    let metadata = fs::metadata(&file).map_err(|error| format!("Cannot read {path}: {error}"))?;
    if metadata.is_dir() {
        return Err(format!("{path} is a directory; use list_directory"));
    }
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Err(format!("{path} is {} and larger than 8 MiB", format_bytes(metadata.len())));
    }
    let bytes = fs::read(&file).map_err(|error| format!("Cannot read {path}: {error}"))?;
    let text = decode_text(&path, &bytes)?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = (start_line as usize).saturating_sub(1);
    if start >= lines.len() {
        return Err(format!("{path} has {} lines; start_line {start_line} is past the end", lines.len()));
    }
    let end = (start + line_count as usize).min(lines.len());
    let mut body = lines[start..end].join("");
    let mut notes = Vec::new();
    if body.chars().count() > MAX_TEXT_CHARS {
        body = truncate_chars(&body, MAX_TEXT_CHARS);
        notes.push(format!("truncated after {MAX_TEXT_CHARS} characters"));
    }
    if end < lines.len() {
        notes.push(format!("showing lines {}-{} of {}", start + 1, end, lines.len()));
    }
    let mut output = format!("{path}\n{body}");
    if !notes.is_empty() {
        output.push_str("\n[");
        output.push_str(&notes.join("; "));
        output.push(']');
    }
    Ok(output)
}

/// Search a text file, or text files under a directory, for a literal string. Skips binary files, .git, target, and node_modules. Returns at most 30 matching lines.
#[uzu_tool_function]
pub(crate) fn search_text(
    /// File or directory to search.
    path: String,
    /// Literal text to find. The match is case-sensitive.
    query: String,
    /// Maximum matching lines to return, from 1 to 30. Omit for 15.
    max_matches: Option<u32>,
) -> Result<String, String> {
    let max_matches = max_matches.unwrap_or(DEFAULT_SEARCH_MATCHES);
    if query.is_empty() {
        return Err("query must not be empty".to_string());
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("query must be at most {MAX_QUERY_CHARS} characters"));
    }
    if !(1..=MAX_SEARCH_MATCHES).contains(&max_matches) {
        return Err(format!("max_matches must be from 1 to {MAX_SEARCH_MATCHES}"));
    }
    let root = PathBuf::from(&path);
    let metadata = fs::metadata(&root).map_err(|error| format!("Cannot read {path}: {error}"))?;
    let mut files = Vec::new();
    let mut files_truncated = false;
    if metadata.is_dir() {
        collect_search_files(&root, 0, &mut files, &mut files_truncated);
    } else {
        files.push(root.clone());
    }
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    for file in &files {
        if matches.len() >= max_matches as usize {
            break;
        }
        let Ok(metadata) = fs::metadata(file) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(file) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        scanned += 1;
        for (index, line) in text.split_inclusive('\n').enumerate() {
            if !line.contains(&query) {
                continue;
            }
            let excerpt = line.trim_end_matches(['\n', '\r']);
            let excerpt = truncate_chars(excerpt, 160);
            matches.push(format!("{}:{}: {excerpt}", file.display(), index + 1));
            if matches.len() >= max_matches as usize {
                break;
            }
        }
    }
    if matches.is_empty() {
        return Ok(format!("No matches for {query:?} in {path} ({scanned} text files scanned)"));
    }
    let mut output = matches.join("\n");
    if matches.len() == max_matches as usize || files_truncated {
        output.push_str("\n[more matches may exist");
        if files_truncated {
            output.push_str(&format!("; stopped after scanning {MAX_SEARCH_FILES} files"));
        }
        output.push(']');
    }
    Ok(output)
}

/// List chapters in an EPUB file or an unpacked EPUB directory. Chapters are split on headings such as "Chapter 1" and on epigraph attributions. When a book has no such markers, each spine document is a chapter.
#[uzu_tool_function]
pub(crate) fn list_epub_chapters(
    /// Path to a .epub file or an unpacked EPUB directory.
    path: String
) -> Result<String, String> {
    let book = load_epub(Path::new(&path))?;
    let shown = book.chapters.len().min(MAX_CHAPTERS_LISTED);
    let mut lines = Vec::new();
    lines.push(format!("{} chapters in {path}", book.chapters.len()));
    for (index, chapter) in book.chapters.iter().take(shown).enumerate() {
        lines.push(format!("{}. {} words. {}", index + 1, chapter.words, chapter.opening));
    }
    if book.chapters.len() > shown {
        lines.push(format!("{} more chapters not shown", book.chapters.len() - shown));
    }
    Ok(lines.join("\n"))
}

/// Read one chapter from an EPUB file or an unpacked EPUB directory. Chapter numbers start at 1 and match list_epub_chapters. Returns at most 48000 characters.
#[uzu_tool_function]
pub(crate) fn read_epub_chapter(
    /// Path to a .epub file or an unpacked EPUB directory.
    path: String,
    /// Chapter number starting at 1.
    chapter: u32,
) -> Result<String, String> {
    if chapter == 0 {
        return Err("chapter counts from 1".to_string());
    }
    let book = load_epub(Path::new(&path))?;
    let Some(text) = book.chapters.get(chapter as usize - 1) else {
        return Err(format!("{path} has {} chapters; chapter {chapter} is out of range", book.chapters.len()));
    };
    let mut body = text.body.clone();
    let truncated = body.chars().count() > MAX_CHAPTER_CHARS;
    if truncated {
        body = truncate_chars(&body, MAX_CHAPTER_CHARS);
    }
    let mut output = format!("Chapter {chapter} of {} from {path}\n\n{body}", book.chapters.len());
    if truncated {
        output.push_str(&format!("\n\n[truncated after {MAX_CHAPTER_CHARS} characters"));
        if book.chapters.len() == 1 {
            output.push_str("; no further chapter markers were found, so this is the start of the book");
        }
        output.push(']');
    }
    Ok(output)
}

#[derive(Clone, Copy)]
enum EntryKind {
    Directory,
    Symlink,
    File {
        bytes: u64,
    },
}

impl EntryKind {
    fn rank(self) -> u8 {
        match self {
            Self::Directory => 0,
            Self::Symlink => 1,
            Self::File {
                ..
            } => 2,
        }
    }
}

struct DirectoryEntry {
    name: String,
    kind: EntryKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Paragraph,
    Heading,
}

struct Block {
    kind: BlockKind,
    text: String,
    spine_index: usize,
}

struct Chapter {
    opening: String,
    words: usize,
    body: String,
}

struct Book {
    chapters: Vec<Chapter>,
}

enum EpubStore {
    Directory {
        opf_dir: PathBuf,
    },
    Zip {
        archive: ZipArchive<File>,
        opf_dir: String,
    },
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn truncate_chars(
    text: &str,
    max_chars: usize,
) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut end = 0;
    for (index, (byte, _)) in text.char_indices().enumerate() {
        if index == max_chars {
            end = byte;
            break;
        }
    }
    text[..end].to_string()
}

fn decode_text(
    path: &str,
    bytes: &[u8],
) -> Result<String, String> {
    if bytes.contains(&0) {
        return Err(format!("{path} looks like a binary file"));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{path} is not valid UTF-8"))
}

fn collect_search_files(
    directory: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
    truncated: &mut bool,
) {
    if *truncated || depth > MAX_SEARCH_DEPTH || files.len() >= MAX_SEARCH_FILES {
        *truncated = files.len() >= MAX_SEARCH_FILES;
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_SEARCH_FILES {
            *truncated = true;
            return;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(name.as_ref(), ".git" | "target" | "node_modules") {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_search_files(&path, depth + 1, files, truncated);
        } else if metadata.is_file() {
            files.push(path);
        }
    }
}

fn load_epub(path: &Path) -> Result<Book, String> {
    let display = path.display().to_string();
    let metadata = fs::metadata(path).map_err(|error| format!("Cannot read {display}: {error}"))?;
    let (mut store, opf_xml) = if metadata.is_dir() {
        let opf = find_opf_in_directory(path)?;
        let opf_xml = fs::read_to_string(&opf).map_err(|error| format!("Cannot read {}: {error}", opf.display()))?;
        let opf_dir = opf.parent().unwrap_or(path).to_path_buf();
        (
            EpubStore::Directory {
                opf_dir,
            },
            opf_xml,
        )
    } else {
        let file = File::open(path).map_err(|error| format!("Cannot read {display}: {error}"))?;
        let mut archive = ZipArchive::new(file).map_err(|error| format!("{display} is not an EPUB zip: {error}"))?;
        let container = match read_zip_entry(&mut archive, "META-INF/container.xml", MAX_EPUB_ENTRY_BYTES) {
            Ok(container) => Some(container),
            Err(error) if error.contains("not in the EPUB") => None,
            Err(error) => return Err(error),
        };
        let opf_path =
            container.as_deref().and_then(container_rootfile).unwrap_or("content.opf").trim_start_matches('/');
        let opf_xml = read_zip_entry(&mut archive, opf_path, MAX_EPUB_ENTRY_BYTES)
            .map_err(|error| format!("Cannot read the package document in {display}: {error}"))?;
        let opf_dir = Path::new(opf_path).parent().unwrap_or(Path::new("")).to_string_lossy().replace('\\', "/");
        (
            EpubStore::Zip {
                archive,
                opf_dir,
            },
            opf_xml,
        )
    };
    let spine = spine_hrefs(&opf_xml)?;
    if spine.is_empty() {
        return Err(format!("{display} has no readable spine documents"));
    }
    let mut blocks = Vec::new();
    let mut total = 0usize;
    for (spine_index, href) in spine.iter().enumerate() {
        let html = store.read(href, &mut total)?;
        blocks.extend(html_blocks(&html, spine_index));
    }
    let chapters = chapter_texts(&blocks);
    if chapters.is_empty() {
        return Err(format!("{display} has no readable text"));
    }
    Ok(Book {
        chapters,
    })
}

fn find_opf_in_directory(root: &Path) -> Result<PathBuf, String> {
    let container = root.join("META-INF/container.xml");
    if let Ok(xml) = fs::read_to_string(&container) {
        if let Some(full_path) = container_rootfile(&xml) {
            let opf = safe_join(root, full_path)?;
            if opf.is_file() {
                return Ok(opf);
            }
            return Err(format!("container.xml points at {} but that file is missing", opf.display()));
        }
    }
    let direct = root.join("content.opf");
    if direct.is_file() {
        return Ok(direct);
    }
    Err(format!("{} is not an unpacked EPUB (no META-INF/container.xml rootfile or content.opf)", root.display()))
}

impl EpubStore {
    fn read(
        &mut self,
        href: &str,
        total: &mut usize,
    ) -> Result<String, String> {
        let href = href.split(['#', '?']).next().unwrap_or(href);
        let href = percent_decode(href);
        match self {
            Self::Directory {
                opf_dir,
            } => {
                let file = safe_join(opf_dir, &href)?;
                let metadata =
                    fs::metadata(&file).map_err(|error| format!("Cannot read {}: {error}", file.display()))?;
                if metadata.len() as usize > MAX_EPUB_ENTRY_BYTES
                    || *total + metadata.len() as usize > MAX_EPUB_TOTAL_BYTES
                {
                    return Err("EPUB text is larger than 20 MiB; refusing to read it all".to_string());
                }
                let bytes = fs::read(&file).map_err(|error| format!("Cannot read {}: {error}", file.display()))?;
                *total += bytes.len();
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            },
            Self::Zip {
                archive,
                opf_dir,
            } => {
                if href.split(['/', '\\']).any(|part| part == "..") {
                    return Err(format!("EPUB link {href} escapes the book"));
                }
                let name = zip_join(opf_dir, &href);
                let bytes = read_zip_bytes(archive, &name, MAX_EPUB_ENTRY_BYTES)?;
                if *total + bytes.len() > MAX_EPUB_TOTAL_BYTES {
                    return Err("EPUB text is larger than 20 MiB; refusing to read it all".to_string());
                }
                *total += bytes.len();
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            },
        }
    }
}

fn read_zip_entry(
    archive: &mut ZipArchive<File>,
    name: &str,
    limit: usize,
) -> Result<String, String> {
    let bytes = read_zip_bytes(archive, name, limit)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_zip_bytes(
    archive: &mut ZipArchive<File>,
    name: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut entry = archive.by_name(name).map_err(|error| match error {
        ZipError::FileNotFound => format!("{name} is not in the EPUB"),
        other => format!("Cannot read {name} from the EPUB: {other}"),
    })?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = entry.read(&mut chunk).map_err(|error| format!("Cannot read {name} from the EPUB: {error}"))?;
        if read == 0 {
            break;
        }
        if bytes.len() + read > limit {
            return Err(format!("{name} is larger than {} bytes", limit));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(bytes)
}

fn container_rootfile(xml: &str) -> Option<&str> {
    xml_elements(xml, "rootfile").first().and_then(|element| xml_attr(element, "full-path"))
}

fn spine_hrefs(opf: &str) -> Result<Vec<String>, String> {
    let mut manifest = Vec::new();
    for element in xml_elements(opf, "item") {
        let Some(id) = xml_attr(element, "id") else {
            continue;
        };
        let Some(href) = xml_attr(element, "href") else {
            continue;
        };
        let media_type = xml_attr(element, "media-type").unwrap_or("");
        manifest.push((id, href, media_type));
    }
    let mut hrefs = Vec::new();
    for element in xml_elements(opf, "itemref") {
        let Some(idref) = xml_attr(element, "idref") else {
            continue;
        };
        let Some((_, href, media_type)) = manifest.iter().find(|(id, _, _)| *id == idref) else {
            continue;
        };
        if is_html_document(href, media_type) {
            hrefs.push((*href).to_string());
        }
    }
    Ok(hrefs)
}

fn is_html_document(
    href: &str,
    media_type: &str,
) -> bool {
    let href = href.split(['#', '?']).next().unwrap_or(href).to_ascii_lowercase();
    media_type.contains("html") || href.ends_with(".html") || href.ends_with(".xhtml") || href.ends_with(".htm")
}

fn html_blocks(
    html: &str,
    spine_index: usize,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut buffer = String::new();
    let mut kind: Option<BlockKind> = None;
    let mut skip: Option<String> = None;
    let bytes = html.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'<' {
            let tag = parse_tag(&html[index..]);
            index += tag.consumed;
            if let Some(name) = skip.as_deref() {
                if tag.closing && tag.name.eq_ignore_ascii_case(name) {
                    skip = None;
                }
                continue;
            }
            if tag.closing && kind.is_some() && is_block_tag(&tag.name) {
                push_block(&mut blocks, kind.take(), &mut buffer, spine_index);
                continue;
            }
            if tag.closing || tag.name.is_empty() {
                continue;
            }
            if tag.name.eq_ignore_ascii_case("script") || tag.name.eq_ignore_ascii_case("style") {
                skip = Some(tag.name);
                continue;
            }
            if is_block_tag(&tag.name) {
                push_block(&mut blocks, kind.take(), &mut buffer, spine_index);
                kind = Some(if is_heading_tag(&tag.name) {
                    BlockKind::Heading
                } else {
                    BlockKind::Paragraph
                });
            } else if tag.name.eq_ignore_ascii_case("br") && kind.is_some() {
                buffer.push(' ');
            }
            continue;
        }
        if skip.is_some() || kind.is_none() {
            index += html[index..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        if bytes[index] == b'&' {
            let (character, consumed) = decode_entity(&html[index..]);
            buffer.push(character);
            index += consumed;
            continue;
        }
        let character = html[index..].chars().next().unwrap_or('\u{fffd}');
        buffer.push(character);
        index += character.len_utf8();
    }
    push_block(&mut blocks, kind.take(), &mut buffer, spine_index);
    if blocks.is_empty() {
        let text = collapse_whitespace(&strip_tags(html));
        if !text.is_empty() {
            blocks.push(Block {
                kind: BlockKind::Paragraph,
                text,
                spine_index,
            });
        }
    }
    blocks
}

struct Tag {
    name: String,
    closing: bool,
    consumed: usize,
}

fn parse_tag(html: &str) -> Tag {
    let mut chars = html.char_indices().peekable();
    chars.next();
    let closing = chars.peek().is_some_and(|(_, character)| *character == '/');
    if closing {
        chars.next();
    }
    let name_start = chars.peek().map(|(index, _)| *index).unwrap_or(html.len());
    while chars.peek().is_some_and(|(_, character)| character.is_ascii_alphanumeric()) {
        chars.next();
    }
    let name_end = chars.peek().map(|(index, _)| *index).unwrap_or(html.len());
    let mut quoted: Option<char> = None;
    let mut consumed = html.len();
    for (index, character) in chars {
        if let Some(quote) = quoted {
            if character == quote {
                quoted = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quoted = Some(character),
            '>' => {
                consumed = index + 1;
                break;
            },
            _ => {},
        }
    }
    Tag {
        name: html[name_start..name_end].to_string(),
        closing,
        consumed,
    }
}

fn is_block_tag(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "blockquote")
}

fn is_heading_tag(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "h1" | "h2" | "h3")
}

fn push_block(
    blocks: &mut Vec<Block>,
    kind: Option<BlockKind>,
    buffer: &mut String,
    spine_index: usize,
) {
    let Some(kind) = kind else {
        buffer.clear();
        return;
    };
    let text = collapse_whitespace(buffer);
    buffer.clear();
    if text.is_empty() {
        return;
    }
    blocks.push(Block {
        kind,
        text,
        spine_index,
    });
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_tags(html: &str) -> String {
    let mut text = String::new();
    let mut skipping = false;
    for character in html.chars() {
        match character {
            '<' => skipping = true,
            '>' => skipping = false,
            _ if !skipping => text.push(character),
            _ => {},
        }
    }
    text
}

fn decode_entity(html: &str) -> (char, usize) {
    let Some(end) = html.find(';') else {
        return ('&', 1);
    };
    if end > 16 {
        return ('&', 1);
    }
    let body = &html[1..end];
    let character = if let Some(digits) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        u32::from_str_radix(digits, 16).ok().and_then(char::from_u32)
    } else if let Some(digits) = body.strip_prefix('#') {
        digits.parse::<u32>().ok().and_then(char::from_u32)
    } else {
        Some(match body {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "nbsp" => '\u{00a0}',
            _ => return ('&', 1),
        })
    };
    (character.unwrap_or('\u{fffd}'), end + 1)
}

fn chapter_texts(blocks: &[Block]) -> Vec<Chapter> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut boundaries = vec![0usize];
    for (index, block) in blocks.iter().enumerate().skip(1) {
        let starts = match block.kind {
            BlockKind::Heading if is_chapter_heading(&block.text) && has_body_before(blocks, index) => Some(index),
            BlockKind::Paragraph if is_attribution(&block.text) => {
                let start = epigraph_start(blocks, index);
                has_body_before(blocks, start).then_some(start)
            },
            _ => None,
        };
        if let Some(start) = starts {
            if boundaries.last().copied() != Some(start) {
                boundaries.push(start);
            }
        }
    }
    if boundaries.len() < 2 {
        let spine_starts = substantial_spine_starts(blocks);
        if spine_starts.len() >= 2 {
            boundaries = spine_starts;
        }
    }
    boundaries.push(blocks.len());
    boundaries
        .windows(2)
        .map(|window| {
            let body =
                blocks[window[0]..window[1]].iter().map(|block| block.text.as_str()).collect::<Vec<_>>().join("\n\n");
            Chapter {
                opening: opening_line(&blocks[window[0]].text),
                words: body.split_whitespace().count(),
                body,
            }
        })
        .collect()
}

fn is_chapter_heading(text: &str) -> bool {
    ["chapter", "part", "book", "prologue", "epilogue", "preface", "foreword", "afterword"]
        .into_iter()
        .any(|word| starts_with_word(text, word))
}

fn starts_with_word(
    text: &str,
    word: &str,
) -> bool {
    let Some(rest) = text.get(word.len()..) else {
        return false;
    };
    text[..word.len()].eq_ignore_ascii_case(word)
        && rest.chars().next().is_none_or(|character| !character.is_ascii_alphanumeric())
}

fn is_attribution(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, '-' | '–' | '—') {
        return false;
    }
    let rest = chars.as_str().trim_start();
    text.chars().count() <= 80 && rest.chars().filter(|character| character.is_alphabetic()).count() >= 3
}

fn epigraph_start(
    blocks: &[Block],
    attribution: usize,
) -> usize {
    let mut start = attribution;
    let mut stepped = 0;
    while start > 0 && stepped < 8 {
        let previous = &blocks[start - 1];
        if previous.kind != BlockKind::Paragraph
            || previous.text.chars().count() > SHORT_BLOCK_CHARS
            || is_attribution(&previous.text)
        {
            break;
        }
        start -= 1;
        stepped += 1;
    }
    start
}

fn has_body_before(
    blocks: &[Block],
    index: usize,
) -> bool {
    blocks[..index].iter().any(|block| block.text.chars().count() > SHORT_BLOCK_CHARS)
}

fn substantial_spine_starts(blocks: &[Block]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index < blocks.len() {
        let spine = blocks[index].spine_index;
        let end = blocks[index..]
            .iter()
            .position(|block| block.spine_index != spine)
            .map_or(blocks.len(), |offset| index + offset);
        let chars: usize = blocks[index..end].iter().map(|block| block.text.chars().count()).sum();
        if chars >= SUBSTANTIAL_SPINE_CHARS {
            starts.push(index);
        }
        index = end;
    }
    starts
}

fn opening_line(text: &str) -> String {
    let trimmed = truncate_chars(text, 90);
    if text.chars().count() > 90 {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

fn xml_elements<'a>(
    xml: &'a str,
    tag: &str,
) -> Vec<&'a str> {
    let needle = format!("<{tag}");
    let mut elements = Vec::new();
    let mut rest = xml;
    while let Some(index) = rest.find(&needle) {
        let after = &rest[index..];
        let boundary = after.as_bytes().get(needle.len()).copied().unwrap_or(b'>');
        if boundary.is_ascii_alphanumeric() || boundary == b':' || boundary == b'-' {
            rest = &after[needle.len()..];
            continue;
        }
        let Some(end) = tag_end(after) else {
            break;
        };
        elements.push(&after[..end]);
        rest = &after[end..];
    }
    elements
}

fn tag_end(tag: &str) -> Option<usize> {
    let mut quoted: Option<char> = None;
    for (index, character) in tag.char_indices().skip(1) {
        if let Some(quote) = quoted {
            if character == quote {
                quoted = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quoted = Some(character),
            '>' => return Some(index + 1),
            _ => {},
        }
    }
    None
}

fn xml_attr<'a>(
    element: &'a str,
    name: &str,
) -> Option<&'a str> {
    let bytes = element.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(name.as_bytes()) {
            let before_ok = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
            let after = index + name.len();
            if before_ok && bytes.get(after) == Some(&b'=') {
                let quote = *bytes.get(after + 1)?;
                if quote == b'"' || quote == b'\'' {
                    let value_start = after + 2;
                    let rest = element.get(value_start..)?;
                    let end = rest.find(quote as char)?;
                    return Some(&rest[..end]);
                }
            }
        }
        index += 1;
    }
    None
}

fn safe_join(
    root: &Path,
    href: &str,
) -> Result<PathBuf, String> {
    let relative = Path::new(href);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err(format!("EPUB link {href} escapes the book"));
    }
    Ok(root.join(relative))
}

fn zip_join(
    directory: &str,
    href: &str,
) -> String {
    let href = href.replace('\\', "/");
    if directory.is_empty() {
        href
    } else {
        format!("{}/{}", directory.trim_end_matches('/'), href.trim_start_matches('/'))
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
#[path = "../../../unit/interactive/tools_test.rs"]
mod tests;
