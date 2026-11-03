//! Reads a cited document as text for the source pane. PDF and Word
//! documents are reduced to their text; other files pass through.

use std::path::{Component, Path, PathBuf};

/// Larger files are refused.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, serde::Serialize)]
pub struct Source {
    /// The path as cited, relative to the board.
    pub file: String,
    /// "text" | "pdf" | "docx" | "cell" | "url": what the text came from.
    pub kind: &'static str,
    pub text: String,
}

/// Resolve a cited path against the board directory, refusing anything that
/// would leave it, and read it as text.
pub fn read(root: &Path, file: &str) -> Result<Source, String> {
    let rel = Path::new(file);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(format!("{file}: outside the board directory"));
    }
    // A symlink inside the board can point out of it; check the resolved path.
    let path: PathBuf = root
        .join(rel)
        .canonicalize()
        .map_err(|e| format!("{file}: {e}"))?;
    let inside = root.canonicalize().is_ok_and(|r| path.starts_with(r));
    if !inside {
        return Err(format!("{file}: outside the board directory"));
    }
    let meta = std::fs::metadata(&path).map_err(|e| format!("{file}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{file}: not a file"));
    }
    if meta.len() > MAX_BYTES {
        return Err(format!("{file}: too large to show here"));
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let (kind, text) = match ext.as_str() {
        "pdf" => ("pdf", pdf_text(&path)?),
        "docx" => ("docx", docx_text(&path)?),
        _ => ("text", plain_text(&path)?),
    };
    Ok(Source {
        file: file.to_string(),
        kind,
        text,
    })
}

fn plain_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.iter().take(4096).any(|&b| b == 0) {
        return Err(format!(
            "{}: binary file; only text, PDF, and Word documents can be shown",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn pdf_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| format!("could not read the PDF's text: {e}"))?;
    Ok(tidy(&text))
}

/// A .docx is a zip whose body is `word/document.xml`; keeps the text and
/// paragraph breaks and drops the markup.
fn docx_text(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a Word document: {e}"))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| "not a Word document: no word/document.xml".to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    let mut out = String::with_capacity(xml.len() / 4);
    let mut rest = xml.as_str();
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            break;
        };
        let tag = &rest[lt + 1..lt + gt];
        if tag.starts_with("/w:p") || tag.starts_with("w:br") || tag.starts_with("w:cr") {
            out.push('\n');
        } else if tag.starts_with("w:tab") {
            out.push('\t');
        }
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    Ok(tidy(&out))
}

/// Whether `quote` occurs in `text`, ignoring case, quote and dash style, and
/// whitespace. An elision (`…` or `...`) matches any text between the parts.
pub fn find_quote(text: &str, quote: &str) -> bool {
    let text = normalize(text);
    let quote = quote.replace("...", "…");
    let parts: Vec<String> = quote
        .split('…')
        .map(normalize)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return false;
    }
    let mut from = 0;
    for part in parts {
        match text[from..].find(&part) {
            Some(i) => from += i + part.len(),
            None => return false,
        }
    }
    true
}

fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        let c = match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '`' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201F}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            c if c.is_whitespace() => ' ',
            c => c,
        };
        if c == ' ' {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.extend(c.to_lowercase());
    }
    // Quotes are often trimmed mid-sentence, so edge punctuation is ignored.
    out.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '"' | '\''))
        .to_string()
}

/// Collapse the runs of blank lines extraction leaves behind.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_and_refuses_to_leave_the_board() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        // Plain text isn't tidied, so quotes match what the agent read.
        std::fs::write(dir.path().join("docs/a.md"), "# A\n\n\n\nbody\n").unwrap();
        let s = read(dir.path(), "docs/a.md").unwrap();
        assert_eq!(s.kind, "text");
        assert_eq!(s.text, "# A\n\n\n\nbody\n");
        assert_eq!(tidy("a\n\n\n\nb  \n"), "a\n\nb\n");
        assert!(read(dir.path(), "../a.md").unwrap_err().contains("outside"));
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("docs/link"))
            .unwrap();
        assert!(read(dir.path(), "docs/link")
            .unwrap_err()
            .contains("outside"));
        assert!(read(dir.path(), "/etc/hosts")
            .unwrap_err()
            .contains("outside"));
        assert!(read(dir.path(), "docs").unwrap_err().contains("not a file"));
        std::fs::write(dir.path().join("docs/bin"), [0u8, 1, 2, 3]).unwrap();
        assert!(read(dir.path(), "docs/bin").unwrap_err().contains("binary"));
    }

    #[test]
    fn docx_body_becomes_paragraphs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.docx");
        let file = std::fs::File::create(&path).unwrap();
        let mut z = zip::ZipWriter::new(file);
        z.start_file::<_, ()>("word/document.xml", Default::default())
            .unwrap();
        std::io::Write::write_all(
            &mut z,
            br#"<w:document><w:body><w:p><w:r><w:t>Section 7.</w:t></w:r><w:r><w:t xml:space="preserve"> Neither party may assign &amp; delegate.</w:t></w:r></w:p><w:p><w:r><w:t>Next.</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .unwrap();
        z.finish().unwrap();
        let s = read(dir.path(), "c.docx").unwrap();
        assert_eq!(s.kind, "docx");
        assert_eq!(
            s.text,
            "Section 7. Neither party may assign & delegate.\nNext.\n"
        );
    }

    #[test]
    fn a_quote_is_found_the_way_a_reader_would_find_it() {
        let text = "Section 7. Assignment.\nNeither party may assign this\nAgreement without the prior written consent of the other party — except to an Affiliate.";
        assert!(find_quote(
            text,
            "Neither party may assign this Agreement without the prior written consent"
        ));
        assert!(
            find_quote(text, "neither party may ASSIGN this agreement"),
            "case"
        );
        assert!(
            find_quote(
                text,
                "“prior written consent of the other party - except”"
                    .trim_matches(|c| c == '“' || c == '”')
            ),
            "dashes"
        );
        assert!(
            find_quote(text, "Neither party may assign … except to an Affiliate."),
            "elision"
        );
        assert!(
            find_quote(text, "Neither party may assign ... except to an Affiliate"),
            "three dots"
        );
        assert!(
            !find_quote(text, "Either party may assign this Agreement freely"),
            "made up"
        );
        assert!(
            !find_quote(text, "except to an Affiliate … Neither party"),
            "parts out of order"
        );
        assert!(!find_quote(text, "  … "), "nothing to find");
    }
}
