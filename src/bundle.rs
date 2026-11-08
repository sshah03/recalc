//! A board as one file to send: File → Export Board…, `recalc export`.
//!
//! A board is its folder, `.recalc/` and all. An export is an ordinary zip
//! of that folder as git would see it (its `.gitignore`s, including
//! `.recalc/`'s, which keeps out what belongs to one computer), plus copies of
//! the documents its cells name from outside the folder, so a document grid
//! arrives whole:
//!
//! ```text
//! Leases.recalc/                      the board's folder, under its own name
//! Leases.recalc/outside/1-lease.pdf   a document from elsewhere
//! Leases.recalc/.recalc/export.json   {"format": 1, "outside": {"/abs/lease.pdf": "outside/1-lease.pdf"}}
//! ```
//!
//! Anything that unzips opens it: Finder, `unzip`, File → Open…. The first
//! time the board opens, `apply_export` points the cells that named an
//! outside document at its copy, and the note is removed.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// Whether `file` is a zip (an exported board, by its first bytes).
pub fn is_zip(file: &Path) -> bool {
    let mut magic = [0u8; 4];
    fs::File::open(file)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == *b"PK\x03\x04"
}

/// What an export holds, for the person to read back.
#[derive(Debug, Default, PartialEq)]
pub struct Exported {
    pub files: usize,
    pub bytes: u64,
    /// Documents copied in from outside the folder.
    pub outside: usize,
    /// Paths cells name outside the folder that no longer exist.
    pub missing: Vec<String>,
}

/// The note an export leaves in the board for its first open.
const MANIFEST: &str = ".recalc/export.json";

/// "3 files, 1.2 MB, to Leases.recalc", with what didn't come along.
pub fn summary(done: &Exported, out: &Path) -> String {
    let size = match done.bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} bytes"),
    };
    let noun = if done.files == 1 { "file" } else { "files" };
    let name = out.file_name().map_or_else(
        || out.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut s = format!("{} {noun}, {size}, to {name}", done.files);
    if done.outside > 0 {
        s.push_str(&format!(
            ", with {} from outside the board's folder",
            done.outside
        ));
    }
    if !done.missing.is_empty() {
        s.push_str(&format!(
            ". Not found, so left out: {}",
            done.missing.join(", ")
        ));
    }
    s
}

/// Where an export of the board in `dir` goes by default: `Leases.recalc`
/// (or a repo's `pulse`) → `Leases.recalc.zip` (`pulse.recalc.zip`).
pub fn zip_name(dir: &Path) -> String {
    let name = dir
        .file_name()
        .map_or("board".into(), |n| n.to_string_lossy().into_owned());
    let stem = name.strip_suffix(".recalc").unwrap_or(&name);
    format!("{stem}.recalc.zip")
}

/// `out` with the export's extension, whatever the person typed.
pub fn as_zip(out: &Path) -> PathBuf {
    let name = out
        .file_name()
        .map_or("board".into(), |n| n.to_string_lossy().into_owned());
    if name.ends_with(".zip") {
        return out.to_path_buf();
    }
    let stem = name.strip_suffix(".recalc").unwrap_or(&name);
    out.with_file_name(format!("{stem}.recalc.zip"))
}

/// Write the board in `dir` to `out`, a zip, as one folder under the board's name.
pub fn export(dir: &Path, out: &Path) -> Result<Exported> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("reading {}", dir.display()))?;
    if !dir.join(".recalc").is_dir() {
        bail!("{} has no board in it", dir.display());
    }
    let top = dir
        .file_name()
        .map_or("board".into(), |n| n.to_string_lossy().into_owned());
    // Written beside the target and renamed, so a failed export leaves no half file.
    let tmp = out.with_file_name(format!(
        ".{}.part",
        out.file_name()
            .map_or("export".into(), |n| n.to_string_lossy())
    ));
    let mut zip = zip::ZipWriter::new(
        fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?,
    );
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut done = Exported::default();
    let out_abs = out.canonicalize().ok();
    for (path, rel) in board_files(&dir)? {
        if path == tmp || Some(&path) == out_abs.as_ref() {
            continue;
        }
        add_file(&mut zip, &format!("{top}/{rel}"), &path, opts, &mut done)?;
    }
    // Documents the cells name from elsewhere.
    let mut outside = BTreeMap::new();
    for (path, name) in outside_documents(&dir)? {
        if !path.is_file() {
            done.missing.push(path.display().to_string());
            continue;
        }
        let rel = format!("outside/{}-{}", outside.len() + 1, name);
        add_file(&mut zip, &format!("{top}/{rel}"), &path, opts, &mut done)?;
        outside.insert(path.display().to_string(), rel);
        done.outside += 1;
    }
    if !outside.is_empty() {
        zip.start_file(format!("{top}/{MANIFEST}"), opts)?;
        zip.write_all(
            serde_json::to_string_pretty(&serde_json::json!({ "format": 1, "outside": outside }))?
                .as_bytes(),
        )?;
    }
    zip.finish()?;
    fs::rename(&tmp, out).with_context(|| format!("writing {}", out.display()))?;
    Ok(done)
}

/// The board's files as git would see them, with their paths inside it:
/// `.gitignore`s apply (`.recalc/`'s keeps out what belongs to this
/// computer), `.git` stays out, and so does an export's unapplied note.
fn board_files(dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut files = Vec::new();
    let walk = ignore::WalkBuilder::new(dir)
        .hidden(false)
        .require_git(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in walk {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let rel = slashed(
            entry
                .path()
                .strip_prefix(dir)
                .expect("walked inside the board"),
        );
        if rel != MANIFEST {
            files.push((entry.path().to_path_buf(), rel));
        }
    }
    Ok(files)
}

/// File → Save a Copy…: copy the board in `dir` to `into`, a new folder, as
/// git would see it. Documents named from elsewhere stay where they are;
/// they're still on this computer. The copy's agent cells start new
/// conversations rather than continuing the original's.
pub fn copy_board(dir: &Path, into: &Path) -> Result<Exported> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("reading {}", dir.display()))?;
    if !dir.join(".recalc").is_dir() {
        bail!("{} has no board in it", dir.display());
    }
    if into.read_dir().is_ok_and(|mut d| d.next().is_some()) {
        bail!("{} already has files in it", into.display());
    }
    // Resolved through its nearest folder that exists: `into` may not yet.
    let real_into = into.ancestors().find_map(|a| {
        a.canonicalize()
            .ok()
            .map(|real| real.join(into.strip_prefix(a).unwrap_or(Path::new(""))))
    });
    if real_into.is_some_and(|i| i.starts_with(&dir)) {
        bail!("choose a place outside the board's own folder");
    }
    let mut done = Exported::default();
    for (path, rel) in board_files(&dir)? {
        let target = into.join(&rel);
        fs::create_dir_all(target.parent().expect("inside into"))?;
        done.bytes +=
            fs::copy(&path, &target).with_context(|| format!("copying {}", path.display()))?;
        done.files += 1;
    }
    // Each conversation belongs to the original's cell.
    if let Ok(metas) = fs::read_dir(into.join(".recalc/meta")) {
        for meta in metas.flatten() {
            let Ok(text) = fs::read_to_string(meta.path()) else {
                continue;
            };
            let Ok(mut table) = text.parse::<toml::Table>() else {
                continue;
            };
            if table.remove("session_id").is_some() {
                fs::write(meta.path(), toml::to_string(&table)?)?;
            }
        }
    }
    Ok(done)
}

fn add_file(
    zip: &mut zip::ZipWriter<fs::File>,
    name: &str,
    path: &Path,
    opts: zip::write::SimpleFileOptions,
    done: &mut Exported,
) -> Result<()> {
    zip.start_file(name, opts)?;
    let n = std::io::copy(
        &mut fs::File::open(path).with_context(|| format!("reading {}", path.display()))?,
        zip,
    )?;
    done.files += 1;
    done.bytes += n;
    Ok(())
}

/// Literal values that are absolute paths to somewhere outside `dir`, with
/// the name each copy goes by.
fn outside_documents(dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut found = Vec::new();
    let mut names: Vec<(PathBuf, PathBuf)> = Vec::new();
    for home in sheet_homes(dir) {
        if let Ok(entries) = fs::read_dir(home.join("cells")) {
            names.extend(entries.flatten().map(|e| (home.clone(), e.path())));
        }
    }
    names.sort();
    for (home, file) in names {
        // Formulas and commands have a def; only typed-in values name documents.
        let Some(cell) = file.file_name() else {
            continue;
        };
        if home.join("defs").join(cell).exists() {
            continue;
        }
        let Ok(value) = fs::read_to_string(&file) else {
            continue;
        };
        let value = value.strip_suffix('\n').unwrap_or(&value);
        let path = Path::new(value);
        if value.contains('\n') || !path.is_absolute() || path.starts_with(dir) {
            continue;
        }
        let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if real.starts_with(dir) || found.iter().any(|(p, _): &(PathBuf, String)| p == path) {
            continue;
        }
        let name = path
            .file_name()
            .map_or("document".into(), |n| n.to_string_lossy().into_owned());
        found.push((path.to_path_buf(), name));
    }
    Ok(found)
}

/// Where a board keeps each sheet's folders: `.recalc/` for the first, then
/// `.recalc/sheets/<Sheet>/`.
fn sheet_homes(dir: &Path) -> Vec<PathBuf> {
    let mut homes = vec![dir.join(".recalc")];
    if let Ok(sheets) = fs::read_dir(dir.join(".recalc/sheets")) {
        let mut sheets: Vec<PathBuf> = sheets.flatten().map(|e| e.path()).collect();
        sheets.sort();
        homes.extend(sheets);
    }
    homes
}

fn slashed(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Unzip an exported board beside the zip, as Finder would, and return the
/// board's folder: `Downloads/Leases.recalc.zip` → `Downloads/Leases.recalc`,
/// or `Leases 2.recalc` when that's taken. Refuses a zip that isn't one board.
pub fn unzip(file: &Path) -> Result<PathBuf> {
    let mut zip = zip::ZipArchive::new(
        fs::File::open(file).with_context(|| format!("reading {}", file.display()))?,
    )
    .with_context(|| format!("{} isn't a zip", file.display()))?;
    let not_a_board = || anyhow::anyhow!("{} isn't a board recalc exported", file.display());
    // One folder at the top, with a board in it.
    let names: Vec<String> = zip
        .file_names()
        .map(|n| n.map(String::from))
        .collect::<Result<_, _>>()
        .map_err(|_| not_a_board())?;
    let top = names
        .first()
        .and_then(|n| n.split('/').next())
        .unwrap_or_default()
        .to_string();
    let inside = format!("{top}/");
    if top.is_empty()
        || top == ".."
        || !names.iter().all(|n| n.starts_with(&inside))
        || !names
            .iter()
            .any(|n| n.starts_with(&format!("{top}/.recalc/")))
    {
        return Err(not_a_board());
    }
    let parent = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (stem, ext) = match top.strip_suffix(".recalc") {
        Some(stem) => (stem.to_string(), ".recalc"),
        None => (top.clone(), ""),
    };
    let into = (1..)
        .map(|i| {
            parent.join(if i == 1 {
                format!("{stem}{ext}")
            } else {
                format!("{stem} {i}{ext}")
            })
        })
        .find(|p| !p.exists())
        .expect("some name is free");
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().map_err(|_| not_a_board())?.into_owned();
        let Some(rel) = name.strip_prefix(&format!("{top}/")).map(Path::new) else {
            continue;
        };
        // Nothing outside `into`, whatever the archive says.
        if entry.is_dir() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            continue;
        }
        let target = into.join(rel);
        fs::create_dir_all(target.parent().expect("inside into"))?;
        let mut out =
            fs::File::create(&target).with_context(|| format!("writing {}", target.display()))?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(into)
}

/// On a board's first open after an export, point the cells that named a
/// document from elsewhere at its copy, then drop the note. Run by
/// `Store::open`, however the board was unzipped.
pub fn apply_export(dir: &Path) -> Result<()> {
    let note = dir.join(MANIFEST);
    let Ok(text) = fs::read_to_string(&note) else {
        return Ok(());
    };
    let manifest: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if manifest["format"].as_u64().unwrap_or(0) > 1 {
        bail!("this board was exported by a newer recalc; update recalc to open it");
    }
    let outside: BTreeMap<String, String> =
        serde_json::from_value(manifest["outside"].clone()).unwrap_or_default();
    for cells in sheet_homes(dir)
        .iter()
        .filter_map(|h| fs::read_dir(h.join("cells")).ok())
    {
        for cell in cells.flatten() {
            let Ok(value) = fs::read_to_string(cell.path()) else {
                continue;
            };
            let Some(copy) = outside.get(value.strip_suffix('\n').unwrap_or(&value)) else {
                continue;
            };
            if dir.join(copy).is_file() {
                fs::write(cell.path(), format!("{copy}\n"))?;
            }
        }
    }
    // Another process opening the board at once may have removed it already.
    let _ = fs::remove_file(&note);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let p = dir.path().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        dir
    }

    #[test]
    fn an_export_is_a_zip_of_the_board_that_opens_with_its_documents_and_without_this_computer() {
        let elsewhere = board_with(&[("lease.pdf", "the lease")]);
        let lease = elsewhere.path().canonicalize().unwrap().join("lease.pdf");
        let gone = elsewhere.path().join("gone.pdf");
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("Leases.recalc");
        for (path, text) in [
            (".recalc/cells/A2", format!("{}\n", lease.display())),
            (".recalc/cells/A3", "docs/inside.md\n".into()),
            (".recalc/cells/A4", format!("{}\n", gone.display())),
            (".recalc/cells/B2", "Delaware\n".into()),
            (".recalc/meta/B2.toml", "kind = \"literal\"\n".into()),
            (".recalc/.gitignore", "engine.pid\nmcp*.json\n".into()),
            (".recalc/engine.pid", "123".into()),
            (
                ".recalc/mcp.json",
                "{\"command\": \"/Users/me/recalc\"}".into(),
            ),
            ("docs/inside.md", "inside".into()),
            (".gitignore", "target/\n".into()),
            ("target/big.bin", "build output".into()),
            (".git/HEAD", "ref: refs/heads/main".into()),
        ] {
            let p = src.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        assert_eq!(zip_name(&src), "Leases.recalc.zip");
        let out = tempfile::tempdir().unwrap();
        let file = out.path().join(zip_name(&src));
        let done = export(&src, &file).unwrap();
        assert!(is_zip(&file));
        assert_eq!(done.outside, 1);
        assert_eq!(done.missing, vec![gone.display().to_string()]);

        // Unzipped by recalc, beside the file and under the board's name.
        let into = unzip(&file).unwrap();
        assert_eq!(into, out.path().join("Leases.recalc"));
        let read = |p: &str| fs::read_to_string(into.join(p)).ok();
        assert_eq!(
            read(".recalc/cells/A2").as_deref(),
            Some(format!("{}\n", lease.display()).as_str()),
            "untouched until the board opens"
        );
        apply_export(&into).unwrap();
        assert_eq!(read(".recalc/cells/B2").as_deref(), Some("Delaware\n"));
        assert_eq!(read("docs/inside.md").as_deref(), Some("inside"));
        assert_eq!(
            read(".recalc/cells/A3").as_deref(),
            Some("docs/inside.md\n")
        );
        let a2 = read(".recalc/cells/A2").unwrap();
        assert_eq!(a2, "outside/1-lease.pdf\n", "the cell names its copy");
        assert_eq!(read(a2.trim()).as_deref(), Some("the lease"));
        assert!(read(MANIFEST).is_none(), "the note goes once applied");
        for left in [
            ".recalc/engine.pid",
            ".recalc/mcp.json",
            "target/big.bin",
            ".git/HEAD",
        ] {
            assert!(read(left).is_none(), "{left} stays behind");
        }
        // A second unzip doesn't land on the first.
        assert_eq!(unzip(&file).unwrap(), out.path().join("Leases 2.recalc"));
    }

    #[test]
    fn a_copy_is_the_board_as_git_sees_it_with_its_own_conversations() {
        let src = board_with(&[
            (".recalc/cells/B2", "Delaware\n"),
            (".recalc/defs/C2", "@claude('which law?')\n"),
            (
                ".recalc/meta/C2.toml",
                "kind = \"agent_call\"\nsession_id = \"abc\"\n",
            ),
            (".recalc/.gitignore", "engine.pid\n"),
            (".recalc/engine.pid", "123"),
            ("docs/a.md", "a"),
            (".git/HEAD", "ref"),
        ]);
        let out = tempfile::tempdir().unwrap();
        let into = out.path().join("Copy.recalc");
        let done = copy_board(src.path(), &into).unwrap();
        assert_eq!(done.files, 5);
        let read = |p: &str| fs::read_to_string(into.join(p)).ok();
        assert_eq!(read(".recalc/cells/B2").as_deref(), Some("Delaware\n"));
        assert_eq!(read("docs/a.md").as_deref(), Some("a"));
        let meta = read(".recalc/meta/C2.toml").unwrap();
        assert!(
            meta.contains("agent_call") && !meta.contains("session_id"),
            "{meta}"
        );
        assert!(read(".recalc/engine.pid").is_none() && read(".git/HEAD").is_none());
        assert!(
            fs::read_to_string(src.path().join(".recalc/meta/C2.toml"))
                .unwrap()
                .contains("abc"),
            "the original keeps its own"
        );
        assert!(copy_board(src.path(), &into)
            .unwrap_err()
            .to_string()
            .contains("already has files"));
        let inside = src.path().join("sub/Copy");
        assert!(copy_board(src.path(), &inside)
            .unwrap_err()
            .to_string()
            .contains("outside the board's own folder"));
    }

    #[test]
    fn unzip_refuses_what_isnt_a_board_and_writes_nothing_outside_its_folder() {
        let out = tempfile::tempdir().unwrap();
        let text = out.path().join("cells.recalctemplate");
        fs::write(&text, "A1 = 1\n").unwrap();
        assert!(!is_zip(&text));
        assert!(unzip(&text).is_err());

        let opts = zip::write::SimpleFileOptions::default();
        let plain = out.path().join("photos.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&plain).unwrap());
        zip.start_file("photos/a.jpg", opts).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        assert!(unzip(&plain)
            .unwrap_err()
            .to_string()
            .contains("isn't a board"));

        let evil = out.path().join("evil.recalc.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&evil).unwrap());
        zip.start_file("evil.recalc/.recalc/cells/A1", opts)
            .unwrap();
        zip.write_all(b"1\n").unwrap();
        zip.start_file("evil.recalc/../../escaped", opts).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let into = unzip(&evil).unwrap();
        assert!(
            !out.path().join("escaped").exists()
                && !out.path().parent().unwrap().join("escaped").exists()
        );
        assert_eq!(
            fs::read_to_string(into.join(".recalc/cells/A1")).unwrap(),
            "1\n"
        );

        let future = board_with(&[(MANIFEST, "{\"format\": 2}")]);
        assert!(apply_export(future.path())
            .unwrap_err()
            .to_string()
            .contains("newer recalc"));
    }
}
