use std::{fs, io::Write, path::Path};

use tempfile::tempdir;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::*;

#[test]
fn directory_listing_names_entries_and_rejects_a_file() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("notes")).unwrap();
    fs::write(root.path().join("readme.txt"), "hello").unwrap();

    let listing = list_directory::call(root.path().display().to_string()).unwrap();
    let notes = listing.find("dir   notes/").unwrap();
    let readme = listing.find("readme.txt").unwrap();
    assert!(notes < readme, "{listing}");
    assert!(listing.contains("5 B"));

    let error = list_directory::call(root.path().join("readme.txt").display().to_string()).unwrap_err();
    assert!(error.contains("is a file"), "{error}");
    let missing = list_directory::call(root.path().join("missing").display().to_string()).unwrap_err();
    assert!(missing.contains("Cannot read"), "{missing}");
}

#[test]
fn text_file_reads_a_line_window_and_refuses_binary() {
    let root = tempdir().unwrap();
    let file = root.path().join("notes.txt");
    fs::write(&file, "one\ntwo\nthree\nfour\nfive\n").unwrap();
    let path = file.display().to_string();

    let window = read_text_file::call(path.clone(), Some(2), Some(2)).unwrap();
    assert!(window.contains("two\nthree\n"), "{window}");
    assert!(window.contains("showing lines 2-3 of 5"), "{window}");
    assert!(!window.contains("one\n"));
    assert!(!window.contains("four"));

    let error = read_text_file::call(path.clone(), Some(0), None).unwrap_err();
    assert!(error.contains("counts from 1"), "{error}");
    let past_end = read_text_file::call(path, Some(9), None).unwrap_err();
    assert!(past_end.contains("past the end"), "{past_end}");

    let binary = root.path().join("data.bin");
    fs::write(&binary, b"abc\0def").unwrap();
    let error = read_text_file::call(binary.display().to_string(), None, None).unwrap_err();
    assert!(error.contains("binary"), "{error}");
    let error = read_text_file::call(root.path().display().to_string(), None, None).unwrap_err();
    assert!(error.contains("directory"), "{error}");
}

#[test]
fn search_finds_a_literal_line_and_skips_git() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/main.rs"), "fn main() {\n    let marker = 1;\n}\n").unwrap();
    fs::create_dir_all(root.path().join(".git")).unwrap();
    fs::write(root.path().join(".git/config"), "marker = hidden\n").unwrap();

    let found = search_text::call(root.path().display().to_string(), "marker".to_string(), Some(10)).unwrap();
    assert!(found.contains("src/main.rs:2:"), "{found}");
    assert!(!found.contains(".git"), "{found}");

    let none = search_text::call(root.path().display().to_string(), "absent".to_string(), None).unwrap();
    assert!(none.contains("No matches"), "{none}");
    let error = search_text::call(root.path().display().to_string(), String::new(), None).unwrap_err();
    assert!(error.contains("must not be empty"), "{error}");
}

#[test]
fn unpacked_epub_splits_chapters_on_epigraph_attributions() {
    let root = tempdir().unwrap();
    write_unpacked_epub(root.path(), "book.html", &epigraph_html());

    let chapters = list_epub_chapters::call(root.path().display().to_string()).unwrap();
    assert!(chapters.starts_with("2 chapters"), "{chapters}");
    assert!(chapters.contains("Chapterhouse: Dune"), "{chapters}");
    assert!(chapters.contains("When I set out"), "{chapters}");

    let path = root.path().display().to_string();
    let first = read_epub_chapter::call(path.clone(), 1).unwrap();
    let second = read_epub_chapter::call(path, 2).unwrap();
    assert!(first.contains("UNIQUE_CHAPTER_ONE"), "{first}");
    assert!(!first.contains("UNIQUE_CHAPTER_TWO"), "{first}");
    assert!(second.contains("UNIQUE_CHAPTER_TWO"), "{second}");
    assert!(second.contains("Leto II"), "{second}");
    assert!(!second.contains("UNIQUE_CHAPTER_ONE"), "{second}");

    let error = read_epub_chapter::call(root.path().display().to_string(), 0).unwrap_err();
    assert!(error.contains("counts from 1"), "{error}");
    let error = read_epub_chapter::call(root.path().display().to_string(), 3).unwrap_err();
    assert!(error.contains("out of range"), "{error}");
}

#[test]
fn zipped_epub_uses_chapter_headings_and_spine_order() {
    let root = tempdir().unwrap();
    let epub = root.path().join("book.epub");
    write_zip_epub(&epub);

    let chapters = list_epub_chapters::call(epub.display().to_string()).unwrap();
    assert!(chapters.starts_with("2 chapters"), "{chapters}");
    let path = epub.display().to_string();
    let first = read_epub_chapter::call(path.clone(), 1).unwrap();
    let second = read_epub_chapter::call(path, 2).unwrap();
    assert!(first.contains("UNIQUE_ZIP_ONE"), "{first}");
    assert!(!first.contains("UNIQUE_ZIP_TWO"), "{first}");
    assert!(second.contains("UNIQUE_ZIP_TWO"), "{second}");
    assert!(second.starts_with("Chapter 2 of 2"), "{second}");
}

fn write_unpacked_epub(
    root: &Path,
    href: &str,
    html: &str,
) {
    fs::create_dir_all(root.join("META-INF")).unwrap();
    fs::write(
        root.join("META-INF/container.xml"),
        r#"<container><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
    )
    .unwrap();
    fs::write(
        root.join("content.opf"),
        format!(
            r#"<package><manifest><item id="body" href="{href}" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="body"/></spine></package>"#
        ),
    )
    .unwrap();
    fs::write(root.join(href), html).unwrap();
}

fn epigraph_html() -> String {
    let first = long_paragraph("UNIQUE_CHAPTER_ONE When the ghola-baby was delivered");
    let second_quote =
        "When I set out to lead humanity along my Golden Path I promised a lesson their bones would remember.";
    let second = long_paragraph("UNIQUE_CHAPTER_TWO So she calls me Spider Queen");
    format!(
        "<html><body>\
         <h2>Chapterhouse: Dune</h2>\
         <p>April 1985</p>\
         <p>Those who would repeat the past must control the teaching of history.</p>\
         <p>-Bene Gesserit Coda</p>\
         <p>{first}</p>\
         <p>{second_quote}</p>\
         <p>-Leto II, the God Emperor</p>\
         <p>{second}</p>\
         </body></html>"
    )
}

fn long_paragraph(label: &str) -> String {
    format!("{label}. ").repeat(40)
}

fn write_zip_epub(path: &Path) {
    let file = fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut add = |name: &str, body: &str| {
        zip.start_file(name, options).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    };
    add(
        "META-INF/container.xml",
        r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
    );
    add(
        "OEBPS/content.opf",
        r#"<package><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="c2.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
    );
    add(
        "OEBPS/c1.xhtml",
        &format!("<html><body><h1>Chapter 1</h1><p>{}</p></body></html>", long_paragraph("UNIQUE_ZIP_ONE")),
    );
    add(
        "OEBPS/c2.xhtml",
        &format!("<html><body><h1>Chapter 2</h1><p>{}</p></body></html>", long_paragraph("UNIQUE_ZIP_TWO")),
    );
    zip.finish().unwrap();
}
