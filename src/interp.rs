//! `{name}` interpolation in `$` and `!` command sources: dependency
//! extraction and substitution.
//!
//! A `{...}` is a ref only if its body is a cell name (with optional `$`
//! anchors and a `:raw`/`:file`/`.code`/`.err`/`.dur` suffix), an A1 range,
//! or a repo ref (`{file:path}`, `{glob:pattern}`). Anything else, like
//! `${HOME}` or `awk '{print $1}'`, is left verbatim.

use crate::model::{a1_coord, range_cells, ref_name};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Form {
    /// `{name}`: the value, shell-quoted.
    Quoted,
    /// `{name:raw}`: the value verbatim, unquoted.
    Raw,
    /// `{name:file}`: the path to the cell's backing file.
    File,
    /// `{name.code}` / `{name.err}` / `{name.dur}`: last run metadata.
    Code,
    Err,
    Dur,
    /// `{file:path}`: a board-relative repo file, resolved to its absolute
    /// path. The name is the path; on `$` cells the file is watched.
    RepoFile,
    /// `{glob:pattern}`: matching repo files' absolute paths, each quoted.
    RepoGlob,
    /// `{B2:D9}`: every cell of the block, row by row; the name is `from:to`.
    /// In a command each value is one quoted word.
    Range,
}

#[derive(Debug, PartialEq)]
pub enum Piece {
    Text(String),
    Ref { name: String, form: Form },
}

pub fn parse(body: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut rest = body;
    while let Some(open) = rest.find('{') {
        // `${...}` is shell variable syntax, never a cell ref.
        let shell_var = rest[..open].ends_with('$');
        let (before, after_open) = (&rest[..open], &rest[open + 1..]);
        match after_open.find('}') {
            Some(close) if !shell_var => match parse_ref(&after_open[..close]) {
                Some((name, form)) => {
                    text.push_str(before);
                    if !text.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut text)));
                    }
                    pieces.push(Piece::Ref { name, form });
                    rest = &after_open[close + 1..];
                }
                None => {
                    text.push_str(&rest[..open + 1]);
                    rest = after_open;
                }
            },
            _ => {
                text.push_str(&rest[..open + 1]);
                rest = after_open;
            }
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    pieces
}

fn parse_ref(body: &str) -> Option<(String, Form)> {
    // Repo prefixes win over suffix forms, so cells named `file` or `glob`
    // can't use `:raw`/`:file`.
    if let Some(path) = body.strip_prefix("file:") {
        let path = path.trim();
        return (!path.is_empty()).then(|| (path.to_string(), Form::RepoFile));
    }
    if let Some(pattern) = body.strip_prefix("glob:") {
        let pattern = pattern.trim();
        return (!pattern.is_empty()).then(|| (pattern.to_string(), Form::RepoGlob));
    }
    let (name, form) = if let Some((name, suffix)) = body.split_once(':') {
        match suffix {
            "raw" => (name, Form::Raw),
            "file" => (name, Form::File),
            _ => {
                range_cells(name, suffix)?;
                let (from, to) = (ref_name(name)?, ref_name(suffix)?);
                return Some((format!("{from}:{to}"), Form::Range));
            }
        }
    } else if let Some((name, suffix)) = body
        .rsplit_once('.')
        .filter(|(_, s)| matches!(*s, "code" | "err" | "dur"))
    {
        match suffix {
            "code" => (name, Form::Code),
            "err" => (name, Form::Err),
            "dur" => (name, Form::Dur),
            _ => return None,
        }
    } else {
        (body, Form::Quoted)
    };
    ref_name(name).map(|name| (name, form))
}

/// Substitute every ref with the unquoted value `lookup` returns, quoting it
/// here: text and path forms are shell-quoted, numeric forms and `:raw` not.
///
/// A ref inside quotes closes and reopens the surrounding quoting around the
/// value (`"a{x}b"` becomes `"a"'v'"b"`), since single quotes nested in
/// double quotes protect nothing.
pub fn resolve(body: &str, lookup: &dyn Fn(&str, Form) -> String) -> String {
    let mut lex = Lexer::default();
    let mut out = String::new();
    for piece in parse(body) {
        match piece {
            Piece::Text(t) => {
                lex.feed(&t);
                out.push_str(&t);
            }
            Piece::Ref { name, form } => {
                let v = match form {
                    Form::Range => range_rows(&name)
                        .into_iter()
                        .flatten()
                        .map(|m| shell_quote(&lookup(&m, Form::Quoted)))
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => lookup(&name, form),
                };
                let quoted = match form {
                    Form::Quoted | Form::Err | Form::RepoFile | Form::File => shell_quote(&v),
                    // Members are already quoted individually.
                    Form::RepoGlob | Form::Range => v,
                    Form::Raw | Form::Code | Form::Dur => {
                        out.push_str(&v);
                        continue;
                    }
                };
                match lex.quote() {
                    Quote::None => out.push_str(&quoted),
                    Quote::Single => out.push_str(&format!("'{quoted}'")),
                    Quote::Double => out.push_str(&format!("\"{quoted}\"")),
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Quote {
    None,
    Single,
    Double,
}

/// Enough sh lexing to know the quoting at a ref: quotes, backslashes, and
/// `$(...)`, which opens a fresh context. Backticks and here-docs aren't tracked.
struct Lexer {
    /// One frame per open `$(`: its quote state and bare-paren depth.
    frames: Vec<(Quote, u32)>,
    escaped: bool,
    dollar: bool,
}

impl Default for Lexer {
    fn default() -> Self {
        Lexer {
            frames: vec![(Quote::None, 0)],
            escaped: false,
            dollar: false,
        }
    }
}

impl Lexer {
    fn quote(&self) -> Quote {
        self.frames.last().map_or(Quote::None, |f| f.0)
    }

    fn feed(&mut self, text: &str) {
        for c in text.chars() {
            let dollar = std::mem::replace(&mut self.dollar, false);
            if std::mem::take(&mut self.escaped) {
                continue;
            }
            let depth = self.frames.len();
            let frame = self
                .frames
                .last_mut()
                .expect("the outer frame is never popped");
            match (frame.0, c) {
                (Quote::Single, '\'') => frame.0 = Quote::None,
                (Quote::Single, _) => {}
                (_, '\\') => self.escaped = true,
                (_, '$') => self.dollar = true,
                (_, '(') if dollar => self.frames.push((Quote::None, 0)),
                (Quote::Double, '"') => frame.0 = Quote::None,
                (Quote::Double, _) => {}
                (Quote::None, '\'') => frame.0 = Quote::Single,
                (Quote::None, '"') => frame.0 = Quote::Double,
                (Quote::None, '(') => frame.1 += 1,
                (Quote::None, ')') if frame.1 > 0 => frame.1 -= 1,
                (Quote::None, ')') if depth > 1 => {
                    self.frames.pop();
                }
                (Quote::None, _) => {}
            }
        }
    }
}

/// The cells of a `from:to` range ref, row by row.
pub fn range_rows(range: &str) -> Vec<Vec<String>> {
    let Some((from, to)) = range.split_once(':') else {
        return Vec::new();
    };
    let cells = range_cells(from, to).unwrap_or_default();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut last = None;
    for c in cells {
        let row = a1_coord(&c).map(|(_, r)| r);
        if row != last || rows.is_empty() {
            rows.push(Vec::new());
            last = row;
        }
        rows.last_mut().expect("pushed above").push(c);
    }
    rows
}

/// Single-quote `s` so it is inert under `sh -c`.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Cell names a source depends on, in order of first appearance, excluding repo refs.
pub fn deps(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in parse(body) {
        if let Piece::Ref { name, form } = piece {
            if matches!(form, Form::RepoFile | Form::RepoGlob) {
                continue;
            }
            let members = match form {
                Form::Range => range_rows(&name).into_iter().flatten().collect(),
                _ => vec![name],
            };
            for m in members {
                if !out.contains(&m) {
                    out.push(m);
                }
            }
        }
    }
    out
}

/// The repo paths and globs a command references: (pattern, is_glob).
pub fn repo_refs(body: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for piece in parse(body) {
        if let Piece::Ref { name, form } = piece {
            match form {
                Form::RepoFile => out.push((name, false)),
                Form::RepoGlob => out.push((name, true)),
                _ => {}
            }
        }
    }
    out
}

/// Why a source's refs are unusable, if they are: a deleted-cell ref, a repo
/// ref outside the board, or a glob that doesn't compile.
pub fn repo_ref_error(body: &str) -> Option<String> {
    // Left by a row or column delete (see `remap_refs`).
    if body.contains("{#REF!") {
        return Some("refers to a cell that was deleted ({#REF!}); undo the delete, or point it at another cell".into());
    }
    for (pattern, is_glob) in repo_refs(body) {
        let form = if is_glob { "glob" } else { "file" };
        let upward = std::path::Path::new(&pattern)
            .components()
            .any(|c| c == std::path::Component::ParentDir);
        if pattern.starts_with('/') || upward {
            return Some(format!(
                "{{{form}:{pattern}}} must stay inside the board directory"
            ));
        }
        if is_glob {
            if let Err(e) = globset::Glob::new(&pattern) {
                return Some(format!("glob: {e}"));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_refs() {
        assert_eq!(
            parse("grep -c ERROR {logfile}"),
            vec![
                Piece::Text("grep -c ERROR ".into()),
                Piece::Ref {
                    name: "logfile".into(),
                    form: Form::Quoted
                },
            ]
        );
    }

    #[test]
    fn suffix_forms() {
        assert_eq!(
            deps("cat {raw:file} {flags:raw} {run.code} {run.err} {run.dur}"),
            ["raw", "flags", "run"]
        );
        assert!(matches!(
            parse("{a:file}")[0],
            Piece::Ref {
                form: Form::File,
                ..
            }
        ));
        assert!(matches!(
            parse("{a:raw}")[0],
            Piece::Ref {
                form: Form::Raw,
                ..
            }
        ));
        assert!(matches!(
            parse("{a.code}")[0],
            Piece::Ref {
                form: Form::Code,
                ..
            }
        ));
    }

    #[test]
    fn shell_and_awk_braces_left_alone() {
        assert_eq!(
            parse("echo ${HOME}"),
            vec![Piece::Text("echo ${HOME}".into())]
        );
        assert_eq!(
            parse("awk '{print $1}' f"),
            vec![Piece::Text("awk '{print $1}' f".into())]
        );
        assert_eq!(
            parse("jq '{a: .b}'"),
            vec![Piece::Text("jq '{a: .b}'".into())]
        );
        assert_eq!(
            parse("echo {unclosed"),
            vec![Piece::Text("echo {unclosed".into())]
        );
        assert_eq!(parse("{a:nope}"), vec![Piece::Text("{a:nope}".into())]);
    }

    #[test]
    fn adjacent_and_repeated() {
        assert_eq!(deps("{a}{b} {a}"), ["a", "b"]);
    }

    #[test]
    fn anchored_refs_resolve_to_the_plain_cell() {
        assert_eq!(deps("cat {$A$1} {B$2:file} {$C3.code}"), ["A1", "B2", "C3"]);
        // awk's field syntax is not a ref.
        assert_eq!(parse("awk '{$1}'"), vec![Piece::Text("awk '{$1}'".into())]);
    }

    #[test]
    fn repo_ref_forms() {
        assert_eq!(
            parse("wc -l {file:src/main.rs}")[1],
            Piece::Ref {
                name: "src/main.rs".into(),
                form: Form::RepoFile
            }
        );
        assert_eq!(
            parse("cat {glob:src/**/*.rs}")[1],
            Piece::Ref {
                name: "src/**/*.rs".into(),
                form: Form::RepoGlob
            }
        );
        // Repo refs aren't cell deps.
        let body = "diff {file:a.txt} {passed} {glob:*.rs}";
        assert_eq!(deps(body), ["passed"]);
        assert_eq!(
            repo_refs(body),
            [("a.txt".to_string(), false), ("*.rs".to_string(), true)]
        );
        // Empty bodies are not refs; the file: prefix wins over suffix forms.
        assert_eq!(parse("{file:}"), vec![Piece::Text("{file:}".into())]);
        assert!(matches!(
            &parse("{file:raw}")[0],
            Piece::Ref { name, form: Form::RepoFile } if name == "raw"
        ));
    }

    #[test]
    fn repo_refs_resolve_quoted() {
        let lookup = |name: &str, form: Form| match form {
            Form::RepoFile => format!("/repo/{name}"),
            Form::RepoGlob => "'/repo/a.rs' '/repo/b 2.rs'".to_string(),
            _ => String::new(),
        };
        // A single path is quoted here; a glob's list arrives pre-quoted.
        assert_eq!(
            resolve("wc -l {file:my notes.txt}", &lookup),
            "wc -l '/repo/my notes.txt'"
        );
        assert_eq!(
            resolve("cat {glob:*.rs}", &lookup),
            "cat '/repo/a.rs' '/repo/b 2.rs'"
        );
    }

    #[test]
    fn resolve_quotes_by_default() {
        let lookup = |name: &str, form: Form| match (name, form) {
            ("msg", Form::Quoted) => "it's done".to_string(),
            ("flags", Form::Raw) => "-a -b".to_string(),
            ("raw", Form::File) => "/x/.recalc/cells/raw".to_string(),
            ("run", Form::Code) => "3".to_string(),
            _ => String::new(),
        };
        assert_eq!(
            resolve("git commit -am {msg}", &lookup),
            r"git commit -am 'it'\''s done'"
        );
        assert_eq!(resolve("ls {flags:raw}", &lookup), "ls -a -b");
        assert_eq!(
            resolve("wc -l {raw:file}", &lookup),
            "wc -l '/x/.recalc/cells/raw'"
        );
        assert_eq!(resolve("test {run.code} -eq 0", &lookup), "test 3 -eq 0");
    }

    #[test]
    fn quoting_makes_injection_inert() {
        let evil = "'; rm -rf x; '".to_string();
        let quoted = shell_quote(&evil);
        assert_eq!(quoted, r"''\''; rm -rf x; '\'''");
    }

    /// Run `body` with `x` = `value` under `sh -c` and return stdout.
    fn sh(body: &str, value: &str) -> String {
        let command = resolve(body, &|_, _| value.to_string());
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn a_ref_inside_quotes_stays_inert() {
        let evil = "$(echo PWNED) `echo PWNED` \" ' \\";
        for body in [
            "printf %s {x}",
            "printf %s \"{x}\"",
            "printf %s '{x}'",
            "printf %s \"a {x} b\"",
            "printf %s 'a {x} b'",
            "printf %s \"$(printf %s {x})\"",
            "printf %s \"$(printf %s \"{x}\")\"",
        ] {
            let got = sh(body, evil);
            assert!(
                !got.contains("PWNED\n") && got.contains("$(echo PWNED)"),
                "{body}: {got:?}"
            );
        }
        assert_eq!(sh("printf %s \"a {x} b\"", "v w"), "a v w b");
        assert_eq!(sh("printf %s 'a {x} b'", "v w"), "a v w b");
    }

    #[test]
    fn escapes_and_subshells_track_the_quote_state() {
        let mut lex = Lexer::default();
        lex.feed(r#"echo "it\"s "#);
        assert_eq!(lex.quote(), Quote::Double);
        lex.feed(r#"" $(a "b" (c) "#);
        assert_eq!(lex.quote(), Quote::None);
        lex.feed(") '");
        assert_eq!(lex.quote(), Quote::Single);
        assert_eq!(lex.frames.len(), 1);
    }
}
