use std::fs;

use tempfile::tempdir;

use super::*;

#[test]
fn a_typed_absolute_path_is_read_into_the_message() {
    let root = tempdir().unwrap();
    let file = root.path().join("notes.txt");
    fs::write(&file, "alpha\nbeta\n").unwrap();

    let prepared = prepare_message(&format!("summarize {}", file.display()));
    assert!(prepared.text.contains("alpha\nbeta"), "{}", prepared.text);
    assert!(prepared.text.contains("summarize"), "{}", prepared.text);
    let note = prepared.note.unwrap();
    assert!(note.contains("notes.txt"), "{note}");
    assert!(note.contains("text"), "{note}");
}

#[test]
fn an_iterm_drop_escapes_spaces_and_still_resolves() {
    let root = tempdir().unwrap();
    let file = root.path().join("my file.txt");
    fs::write(&file, "dropped body").unwrap();
    let dropped = file.display().to_string().replace(' ', r"\ ");

    let prepared = prepare_message(&dropped);
    assert!(prepared.text.contains("dropped body"), "{}", prepared.text);
    assert!(prepared.note.unwrap().contains("my file.txt"));
}

#[test]
fn a_quoted_path_and_a_url_are_distinguished() {
    let root = tempdir().unwrap();
    let file = root.path().join("chapter.txt");
    fs::write(&file, "quoted body").unwrap();
    let message = format!("read \"{}\" and also https://example.com/chapter.txt", file.display());

    let prepared = prepare_message(&message);
    assert!(prepared.text.contains("quoted body"), "{}", prepared.text);
    assert!(!prepared.text.contains("<file path=\"https://example.com/chapter.txt\">"), "{}", prepared.text);
    assert_eq!(prepared.note.unwrap().lines().count(), 1);
}

#[test]
fn a_bare_relative_name_is_attached_only_when_it_is_the_whole_message() {
    let root = tempdir().unwrap();
    let file = root.path().join("notes.txt");
    fs::write(&file, "whole").unwrap();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(root.path()).unwrap();

    let whole = prepare_message("notes.txt");
    let sentence = prepare_message("please open notes.txt");
    std::env::set_current_dir(previous).unwrap();

    assert!(whole.text.contains("whole"), "{}", whole.text);
    assert!(sentence.note.is_none(), "{}", sentence.text);
    assert!(!sentence.text.contains("<file"), "{}", sentence.text);
}

#[test]
fn an_unpacked_epub_directory_attaches_the_first_chapter_only() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("META-INF")).unwrap();
    fs::write(
        root.path().join("META-INF/container.xml"),
        r#"<container><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
    )
    .unwrap();
    fs::write(
        root.path().join("content.opf"),
        r#"<package><manifest><item id="body" href="book.html" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="body"/></spine></package>"#,
    )
    .unwrap();
    let first = "UNIQUE_CHAPTER_ONE ".repeat(30);
    let second = "UNIQUE_CHAPTER_TWO ".repeat(30);
    fs::write(
        root.path().join("book.html"),
        format!(
            "<html><body><p>{first}</p><p>When I set out to lead humanity along my Golden Path.</p><p>-Leto II</p><p>{second}</p></body></html>"
        ),
    )
    .unwrap();

    let prepared = prepare_message(&format!("summarize {}", root.path().display()));
    assert!(prepared.text.contains("UNIQUE_CHAPTER_ONE"), "{}", prepared.text);
    assert!(!prepared.text.contains("UNIQUE_CHAPTER_TWO"), "{}", prepared.text);
    assert!(prepared.note.unwrap().contains("EPUB chapter 1"));
}

#[test]
fn a_spaced_path_is_the_longest_existing_match() {
    let root = tempdir().unwrap();
    let file = root.path().join("chapter one.txt");
    fs::write(&file, "spaced body").unwrap();

    let prepared = prepare_message(&format!("summarize {} please", file.display()));
    assert!(prepared.text.contains("spaced body"), "{}", prepared.text);
    assert!(prepared.note.unwrap().contains("chapter one.txt"));
}

#[test]
fn a_missing_spaced_path_does_not_attach_its_parent() {
    let root = tempdir().unwrap();
    let present = root.path().join("present");
    fs::create_dir(&present).unwrap();
    let missing = present.join("missing name.txt");

    let prepared = prepare_message(&format!("look at {} later", missing.display()));
    assert!(prepared.note.is_none(), "{}", prepared.text);
    assert!(!prepared.text.contains("<file"), "{}", prepared.text);
}

#[test]
fn a_missing_path_is_left_unchanged() {
    let prepared = prepare_message("look at /this/path/does/not/exist.txt please");
    assert!(prepared.note.is_none());
    assert_eq!(prepared.text, "look at /this/path/does/not/exist.txt please");
}
