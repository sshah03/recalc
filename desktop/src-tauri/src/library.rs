//! The question-column library for document grids. Builtins ship with the
//! app; saved columns live in the config directory and shadow a builtin of
//! the same name.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use recalc::model::{AnswerKind, AnswerType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub question: String,
    pub kind: AnswerKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Shipped with the app, not saved by the user.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub builtin: bool,
}

impl Column {
    /// The answer type, which supplies the check, prompt sentence and format.
    pub fn answer_type(&self) -> AnswerType {
        AnswerType::new(self.kind, &self.options)
    }

    /// The instruction that tells an agent the form of the answer.
    pub fn answer_sentence(&self) -> String {
        self.answer_type().sentence()
    }

    /// The display format the answers take, if any.
    pub fn format(&self) -> Option<recalc::model::Format> {
        self.answer_type().format()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("a column needs a name".into());
        }
        if self.question.trim().is_empty() {
            return Err("a column needs a question".into());
        }
        self.answer_type()
            .validate()
            .map_err(|_| "a choice column needs at least two options".to_string())
    }
}

/// Columns that ship with the app, general ones first. Names are what the picker shows.
pub fn builtins() -> Vec<Column> {
    let col = |name: &str, kind: &str, question: &str, options: &[&str]| Column {
        name: name.into(),
        question: question.into(),
        kind: kind.parse().expect("built-in columns use known kinds"),
        options: options.iter().map(|s| s.to_string()).collect(),
        builtin: true,
    };
    vec![
        col("Summary", "text", "Summarize the document in one sentence.", &[]),
        col("Document type", "choice", "What kind of document is this?", &["Contract", "Amendment", "Invoice", "Report", "Form", "Letter", "Paper", "Other"]),
        col("Date", "date", "What is the document's date? Use the effective or issue date if there are several.", &[]),
        col("Parties", "text", "Who are the parties or authors? Give the names as written.", &[]),
        col("Total amount", "currency", "What is the total amount stated? Give the largest total if there are several.", &[]),
        col("Key quote", "quote", "Quote the single passage most important to understanding this document.", &[]),
        col("Risk level", "choice", "How much risk does this document carry for us?", &["Low", "Medium", "High"]),
        col("Action needed", "yesno", "Does this document ask anything of us, or need a response?", &[]),
        col("Deadline", "date", "What is the nearest deadline or due date the document sets?", &[]),
        col("Governing law", "text", "Which jurisdiction's law governs? Name the jurisdiction only.", &[]),
        col("Term", "text", "What is the initial term, and does it renew automatically?", &[]),
        col("Change of control", "choice", "If our side undergoes a change of control or assigns the agreement, what does it require?", &["Consent required", "Notice only", "Unrestricted", "Not stated"]),
        col("Liability cap", "text", "How is liability capped? Give the cap as written, or say uncapped or not stated.", &[]),
        col("Termination", "text", "How can our side end the agreement, and on how much notice?", &[]),
        col("Payment terms", "text", "What are the payment terms? Net days and any late fees.", &[]),
        col("Tax year", "number", "Which tax year does the document cover? Answer with the year.", &[]),
        col("Method", "text", "What method or approach does the document use? One sentence.", &[]),
        col("Main finding", "text", "What is the main finding or conclusion? One sentence.", &[]),
        col("Sample size", "number", "How many subjects, records, or cases were studied? Answer with the number.", &[]),
    ]
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("columns.json"))
}

fn saved(app: &AppHandle) -> Vec<Column> {
    path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write(app: &AppHandle, columns: &[Column]) -> Result<(), String> {
    let Some(p) = path(app) else {
        return Err("no config directory".into());
    };
    std::fs::create_dir_all(p.parent().expect("has parent")).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(columns).map_err(|e| e.to_string())?;
    std::fs::write(p, json).map_err(|e| e.to_string())
}

/// Saved columns first, then the builtins they don't shadow.
pub fn all(app: &AppHandle) -> Vec<Column> {
    merge(saved(app), builtins())
}

pub fn merge(saved: Vec<Column>, builtins: Vec<Column>) -> Vec<Column> {
    let mut out = saved;
    for b in builtins {
        if !out.iter().any(|c| c.name.eq_ignore_ascii_case(&b.name)) {
            out.push(b);
        }
    }
    out
}

/// Save a column, replacing one of the same name.
pub fn save(app: &AppHandle, mut column: Column) -> Result<(), String> {
    column.validate()?;
    column.builtin = false;
    let mut columns = saved(app);
    columns.retain(|c| !c.name.eq_ignore_ascii_case(&column.name));
    columns.insert(0, column);
    write(app, &columns)
}

/// Drop a saved column; builtins can't be removed, only shadowed.
pub fn forget(app: &AppHandle, name: &str) -> Result<(), String> {
    let mut columns = saved(app);
    let before = columns.len();
    columns.retain(|c| !c.name.eq_ignore_ascii_case(name));
    if columns.len() == before {
        return Err(format!("{name} is a builtin column; it can't be removed"));
    }
    write(app, &columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid_and_shadowed_by_saved_columns() {
        for c in builtins() {
            c.validate().unwrap_or_else(|e| panic!("{}: {e}", c.name));
        }
        let mine = Column {
            name: "governing law".into(),
            question: "Governing law, as a two-letter state code.".into(),
            kind: AnswerKind::Text,
            options: vec![],
            builtin: false,
        };
        let all = merge(vec![mine.clone()], builtins());
        assert_eq!(all[0], mine);
        assert_eq!(
            all.iter()
                .filter(|c| c.name.eq_ignore_ascii_case("governing law"))
                .count(),
            1
        );
    }

    #[test]
    fn answer_types_and_formats_follow_the_kind() {
        use recalc::model::{Align, NumFormat};
        let mut c = builtins().remove(11);
        assert_eq!(c.kind, AnswerKind::Choice);
        assert!(c
            .answer_sentence()
            .contains("Consent required, Notice only"));
        assert_eq!(c.format().unwrap().align, Some(Align::Center));
        c.kind = AnswerKind::Currency;
        assert_eq!(c.format().unwrap().num, Some(NumFormat::Currency));
        c.kind = AnswerKind::Text;
        assert!(c.format().is_none());
        // An unknown kind doesn't load.
        assert!(
            serde_json::from_str::<Column>(r#"{"name":"x","question":"q","kind":"chart"}"#)
                .is_err()
        );
    }
}
