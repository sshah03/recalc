//! Core types: cells, marks, decisions, citations, answer types. Mostly plain
//! data; answer types check and canonicalize answers, and citations validate
//! themselves.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellKind {
    Literal,
    Expr,
    Command,
    Action,
    /// `@claude('prompt', {refs})`: a headless agent run. Side-effecting, so
    /// like `Action` it never runs by itself.
    AgentCall,
}

impl CellKind {
    /// The kind is inferred from the first non-whitespace character of the source.
    pub fn of_source(source: &str) -> CellKind {
        let s = source.trim_start();
        match s.chars().next() {
            Some('=') => CellKind::Expr,
            // `$1,200` is an amount, as anyone typing into a spreadsheet means
            // it; a command is `$ ` and its words.
            Some('$') if !is_amount(s) => CellKind::Command,
            Some('!') => CellKind::Action,
            Some('@') => CellKind::AgentCall,
            _ => CellKind::Literal,
        }
    }
}

/// `$` then a number (`$5`, `$1,200`, `$-3`, `$.50`): a dollar amount.
fn is_amount(s: &str) -> bool {
    let rest = &s[1..];
    let rest = rest.strip_prefix(['-', '+']).unwrap_or(rest);
    let rest = rest.strip_prefix('.').unwrap_or(rest);
    rest.starts_with(|c: char| c.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellState {
    Ok,
    Computing,
    /// An agent run waiting for a slot under `max_agents`.
    Queued,
    Stale,
    Error(String),
    Quarantined,
    Awaiting,
}

impl CellState {
    pub fn word(&self) -> String {
        match self {
            CellState::Ok => "ok".into(),
            CellState::Computing => "computing".into(),
            CellState::Queued => "queued".into(),
            CellState::Stale => "stale".into(),
            CellState::Error(msg) => format!("error: {msg}"),
            CellState::Quarantined => "quarantined".into(),
            CellState::Awaiting => "awaiting".into(),
        }
    }
}

/// Who performed a write. Serialized as a plain string ("human", "agent:a1", ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", from = "String")]
pub enum Actor {
    Human,
    Agent(String),
    Engine,
    Cli,
    System,
}

impl From<Actor> for String {
    fn from(a: Actor) -> String {
        match a {
            Actor::Human => "human".into(),
            Actor::Agent(id) => format!("agent:{id}"),
            Actor::Engine => "engine".into(),
            Actor::Cli => "cli".into(),
            Actor::System => "system".into(),
        }
    }
}

impl From<String> for Actor {
    fn from(s: String) -> Actor {
        match s.as_str() {
            "engine" => Actor::Engine,
            "cli" => Actor::Cli,
            "system" => Actor::System,
            _ => match s.strip_prefix("agent:") {
                Some(id) => Actor::Agent(id.to_string()),
                None => Actor::Human,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub by: Actor,
    /// Unix milliseconds.
    pub at: u64,
    /// Writer-supplied context (MCP set_cell's `note`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Provenance {
    pub fn now(by: Actor) -> Provenance {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before 1970")
            .as_millis() as u64;
        Provenance { by, at, note: None }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Cell {
    pub name: String,
    pub kind: CellKind,
    /// The raw definition string. Absent ⇒ pure literal.
    pub source: Option<String>,
    /// Current value; always a string.
    pub value: String,
    /// Names of cells this one references (parsed from `source`).
    pub deps: Vec<String>,
    pub state: CellState,
    /// Last run's metadata, for Command/Action cells.
    pub exit_code: Option<i32>,
    pub stderr: String,
    pub duration_ms: Option<u64>,
    /// A human confirmed this quarantined `$` cell, so it may auto-run.
    /// Reset when the source changes.
    pub confirmed: bool,
    /// Series-tracked: the grid shows this cell's history as a sparkline.
    pub series: bool,
    /// Who installed the current definition; suggestions route to this owner.
    /// None for literals.
    pub def_by: Option<Actor>,
    /// Present only in the Awaiting state.
    pub decision: Option<Decision>,
    /// Proposed values awaiting the owner; never applied on their own.
    pub pending_suggestions: Vec<Suggestion>,
    /// Human review state; survives value writes.
    pub mark: Option<Mark>,
    /// Display formatting; survives value writes, moves with the cell.
    pub format: Option<Format>,
    /// The passage behind the value. Cleared on every value write that
    /// doesn't bring its own.
    pub cite: Option<Cite>,
    /// The answer type a write here must fit. Survives value writes; an
    /// agent write that doesn't fit is refused (strict) or noted.
    #[serde(alias = "shape")]
    pub answer_type: Option<AnswerType>,
    /// What the last agent run cost, when the agent reported it.
    pub usage: Option<Usage>,
    /// The agent's conversation id, so it can be resumed in a terminal.
    pub session_id: Option<String>,
    /// What a streaming agent run is doing now. In memory only, never saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    pub provenance: Provenance,
}

/// A pending question on a cell: posed by an agent (`ask_cell`)
/// or by the system when a human edits a computed cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub question: String,
    pub options: Vec<DecisionOption>,
    #[serde(default)]
    pub allow_freetext: bool,
    /// The option the asker recommends: shown as the default, never picked
    /// on its behalf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<usize>,
    pub asker: Actor,
    pub posed_at: u64,
    /// System-posed edit decisions carry the typed-but-not-applied value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    /// A running agent waits on this answer over stdin (`stream::run`); the
    /// answer goes to the run, not into the cell's value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_run: bool,
    /// The agent named to answer (`ask_cell`'s `for`): `driver` or an agent
    /// cell's name. A person can always answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answerable_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionOption {
    pub label: String,
    pub value: String,
    /// A short preview of the option's effect, shown under the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub value: String,
    pub from: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub at: u64,
}

/// A review mark. `Verified` turns agent writes into suggestions and makes
/// bulk actions skip the cell; `Flagged` is a reminder. Recomputation
/// ignores marks, so a verified formula still tracks its inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub kind: MarkKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub by: Actor,
    pub at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Verified,
    Flagged,
}

/// The passage supporting a value; cleared when the value is recomputed or
/// overwritten. `file` is relative to the board, `quote` is verbatim, and
/// `where` is a free-form locator such as "§7.2".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cite {
    pub file: String,
    pub quote: String,
    #[serde(rename = "where", default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub by: Actor,
    pub at: u64,
    /// Whether the quote was found in the file when cited; None if the file
    /// couldn't be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<bool>,
}

impl Cite {
    /// The source is a path inside the board, `cell:<name>`, or an http(s)
    /// URL; the quote must say something.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(name) = self.file.strip_prefix("cell:") {
            if !valid_name(name) && A1Ref::parse(name).is_none() {
                return Err(format!("cite.file cell:{name} doesn't name a cell"));
            }
        } else if !self.is_url() {
            let path = std::path::Path::new(&self.file);
            if self.file.is_empty()
                || path.is_absolute()
                || path
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
            {
                return Err(
                    "cite.file must be a path inside the board directory, cell:<name>, or an http(s) URL"
                        .into(),
                );
            }
        }
        if self.quote.trim().is_empty() {
            return Err("cite.quote must be the supporting passage".into());
        }
        if self.quote.len() > 4000 {
            return Err("cite.quote is too long; quote the passage, not the section".into());
        }
        Ok(())
    }

    pub fn is_url(&self) -> bool {
        self.file.starts_with("https://") || self.file.starts_with("http://")
    }
}

/// Display formatting only; the stored value is unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Format {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num: Option<NumFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    /// Long text shows on several lines, and its row grows to three.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wrap: bool,
    /// Digits after the point for number, currency and percent (0–10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// The currency, as an ISO 4217 code (EUR, GBP, JPY); US dollars when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Fills that follow the value: the first rule that matches wins over `fill`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlight: Vec<Highlight>,
}

/// A fill that applies while the value matches: "fail" in red, over 90 in green.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Highlight {
    pub when: When,
    #[serde(default)]
    pub value: String,
    pub fill: Fill,
}

/// How a highlight compares: text is case-insensitive; above and below are numeric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum When {
    Is,
    Contains,
    Above,
    Below,
}

impl Highlight {
    pub fn matches(&self, value: &str) -> bool {
        let v = value.trim();
        let want = self.value.trim();
        let num = |s: &str| {
            s.chars()
                .filter(|c| !matches!(c, ',' | '$' | '%' | ' '))
                .collect::<String>()
                .parse::<f64>()
                .ok()
        };
        match self.when {
            When::Is => v.eq_ignore_ascii_case(want),
            When::Contains => !want.is_empty() && v.to_lowercase().contains(&want.to_lowercase()),
            When::Above => matches!((num(v), num(want)), (Some(a), Some(b)) if a > b),
            When::Below => matches!((num(v), num(want)), (Some(a), Some(b)) if a < b),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Center,
    Right,
}

/// How a number displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NumFormat {
    Text,
    Integer,
    Number,
    Currency,
    Percent,
    Date,
}

/// A fill from the small palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fill {
    Yellow,
    Green,
    Blue,
    Red,
    Purple,
    Gray,
}

impl Format {
    pub fn is_plain(&self) -> bool {
        *self == Format::default()
    }

    /// The fill a value shows: the first matching highlight, else the fixed fill.
    pub fn fill_for(&self, value: &str) -> Option<Fill> {
        self.highlight
            .iter()
            .find(|h| h.matches(value))
            .map(|h| h.fill)
            .or(self.fill)
    }

    /// Refuse what can't display: too many decimals, a currency code that isn't one.
    pub fn validate(&self) -> Result<(), String> {
        if self.decimals.is_some_and(|d| d > 10) {
            return Err("decimals is 0 to 10".into());
        }
        if let Some(c) = &self.currency {
            if c.len() != 3 || !c.chars().all(|ch| ch.is_ascii_uppercase()) {
                return Err(format!(
                    "currency is a three-letter ISO code like EUR or GBP, not {c:?}"
                ));
            }
        }
        if self.highlight.len() > 8 {
            return Err("at most 8 highlight rules on a cell".into());
        }
        for h in &self.highlight {
            if matches!(h.when, When::Above | When::Below)
                && h.value
                    .trim()
                    .replace([',', '$', '%'], "")
                    .parse::<f64>()
                    .is_err()
            {
                return Err(format!(
                    "highlight {:?} needs a number, not {:?}",
                    h.when, h.value
                ));
            }
        }
        Ok(())
    }
}

/// What one agent run cost, parsed from its machine-readable result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    /// The model that did the run: what the agent reported (Claude's
    /// `modelUsage`), else the one the call asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// The form an answer must take, checked when an agent writes the cell.
/// `strict` refuses a misfit; otherwise the write lands and the state notes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerType {
    pub kind: AnswerKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default = "default_true")]
    pub strict: bool,
    /// Whether "Not found" (and "n/a", "not stated") fits.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub allows_not_found: bool,
}

fn is_true(b: &bool) -> bool {
    *b
}

fn default_true() -> bool {
    true
}

/// The kinds of answer a cell can require. Serialized lowercase (`yesno`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnswerKind {
    Text,
    #[serde(rename = "yesno")]
    YesNo,
    Date,
    Number,
    Currency,
    Choice,
    Quote,
    /// Text matching the regex in `options[0]` exactly.
    Pattern,
    /// One JSON object or array; `options` are top-level keys it must have.
    Json,
}

impl AnswerKind {
    pub const ALL: [AnswerKind; 9] = [
        AnswerKind::Text,
        AnswerKind::YesNo,
        AnswerKind::Date,
        AnswerKind::Number,
        AnswerKind::Currency,
        AnswerKind::Choice,
        AnswerKind::Quote,
        AnswerKind::Pattern,
        AnswerKind::Json,
    ];

    pub fn name(self) -> &'static str {
        match self {
            AnswerKind::Text => "text",
            AnswerKind::YesNo => "yesno",
            AnswerKind::Date => "date",
            AnswerKind::Number => "number",
            AnswerKind::Currency => "currency",
            AnswerKind::Choice => "choice",
            AnswerKind::Quote => "quote",
            AnswerKind::Pattern => "pattern",
            AnswerKind::Json => "json",
        }
    }
}

impl std::str::FromStr for AnswerKind {
    type Err = String;

    fn from_str(s: &str) -> Result<AnswerKind, String> {
        AnswerKind::ALL
            .into_iter()
            .find(|k| k.name() == s)
            .ok_or_else(|| {
                let names: Vec<&str> = AnswerKind::ALL.iter().map(|k| k.name()).collect();
                format!("answer type must be one of {}", names.join(", "))
            })
    }
}

impl std::fmt::Display for AnswerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl AnswerType {
    pub fn new(kind: AnswerKind, options: &[String]) -> AnswerType {
        AnswerType {
            kind,
            options: options.to_vec(),
            strict: true,
            allows_not_found: true,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.kind == AnswerKind::Choice && self.options.len() < 2 {
            return Err("a choice answer type needs at least two options".into());
        }
        if self.kind == AnswerKind::Pattern {
            let [re] = self.options.as_slice() else {
                return Err("a pattern answer type needs one regex".into());
            };
            regex::Regex::new(re).map_err(|e| format!("bad pattern: {e}"))?;
        }
        Ok(())
    }

    /// What the cell takes, in a person's words, for a value typed that doesn't fit
    /// ("one of Approve, Reject"). Text and quotes take anything.
    pub fn wants(&self) -> String {
        match self.kind {
            AnswerKind::YesNo => "Yes or No".into(),
            AnswerKind::Date => "a date as YYYY-MM-DD".into(),
            AnswerKind::Number => "a number".into(),
            AnswerKind::Currency => "an amount".into(),
            AnswerKind::Choice => format!("one of {}", self.options.join(", ")),
            AnswerKind::Pattern => format!(
                "a value matching {}",
                self.options.first().map(String::as_str).unwrap_or("")
            ),
            AnswerKind::Json if self.options.is_empty() => "JSON".into(),
            AnswerKind::Json => format!("a JSON object with the keys {}", self.options.join(", ")),
            AnswerKind::Text | AnswerKind::Quote => "any text".into(),
        }
    }

    /// A value a person typed: its canonical form when it fits ("yes" → "Yes",
    /// "$1,200" → "1200"), or, when it doesn't, what the cell wants. A person's
    /// value is never refused, since they may be correcting the type.
    pub fn typed(&self, value: &str) -> Result<String, String> {
        self.check(value).map_err(|_| self.wants())
    }

    /// The instruction that tells an agent the form of the answer.
    pub fn sentence(&self) -> String {
        match self.kind {
            AnswerKind::YesNo => "Answer Yes or No.".into(),
            AnswerKind::Date => "Answer with the date only, as YYYY-MM-DD.".into(),
            AnswerKind::Number => "Answer with the number only, no units or symbols.".into(),
            AnswerKind::Currency => {
                "Answer with the amount as a plain number with no currency symbol; add the currency code only if it is not USD.".into()
            }
            AnswerKind::Choice => format!("Answer with exactly one of: {}.", self.options.join(", ")),
            AnswerKind::Quote => "Answer with the verbatim passage only.".into(),
            AnswerKind::Pattern => format!(
                "Answer with the value only, matching the pattern {}.",
                self.options.first().map(String::as_str).unwrap_or("")
            ),
            AnswerKind::Json if self.options.is_empty() => {
                "Answer with one JSON object or array only, nothing before or after it.".into()
            }
            AnswerKind::Json => format!(
                "Answer with one JSON object only, with the keys {}, and nothing before or after it.",
                self.options.join(", ")
            ),
            AnswerKind::Text => "Answer in one short sentence.".into(),
        }
    }

    /// The display format answers of this type take, if any.
    pub fn format(&self) -> Option<Format> {
        let mut f = Format::default();
        match self.kind {
            AnswerKind::Date => f.num = Some(NumFormat::Date),
            AnswerKind::Number => f.num = Some(NumFormat::Number),
            AnswerKind::Currency => f.num = Some(NumFormat::Currency),
            AnswerKind::YesNo | AnswerKind::Choice => f.align = Some(Align::Center),
            AnswerKind::Text | AnswerKind::Quote | AnswerKind::Pattern | AnswerKind::Json => {
                return None
            }
        }
        Some(f)
    }

    /// Checks an answer, returning its canonical form ("$1,200" → "1200") or
    /// an error an agent can act on. "Not found" and its variants always fit.
    pub fn check(&self, answer: &str) -> Result<String, String> {
        let raw = answer.trim();
        // What an agent wraps a short answer in: **done**, "Yes", `2026-01-02`.
        let bare = raw
            .trim_end_matches(['.', '!'])
            .trim()
            .trim_matches(|c| matches!(c, '"' | '\'' | '`' | '*' | '\u{201c}' | '\u{201d}'))
            .trim_end_matches(['.', '!'])
            .trim();
        if is_not_found(bare) && self.allows_not_found {
            return Ok("Not found".into());
        }
        let misfit = |expected: &str| {
            Err(format!(
                "expected {expected}, got {:?}. {}",
                first_line(raw),
                self.sentence()
            ))
        };
        match self.kind {
            AnswerKind::YesNo => match bare.to_ascii_lowercase().as_str() {
                "yes" | "y" | "true" => Ok("Yes".into()),
                "no" | "n" | "false" => Ok("No".into()),
                _ => misfit("Yes or No"),
            },
            AnswerKind::Date => {
                let ok = bare.len() == 10
                    && bare.bytes().enumerate().all(|(i, b)| match i {
                        4 | 7 => b == b'-',
                        _ => b.is_ascii_digit(),
                    });
                if ok {
                    Ok(bare.into())
                } else {
                    misfit("a date as YYYY-MM-DD")
                }
            }
            AnswerKind::Number | AnswerKind::Currency => {
                let cleaned: String = bare
                    .chars()
                    .filter(|c| !matches!(c, ',' | '$' | '€' | '£' | ' ' | '_'))
                    .collect();
                // A trailing currency code is allowed on amounts.
                let (num, code) = match cleaned.find(|c: char| c.is_ascii_alphabetic()) {
                    Some(i) if self.kind == AnswerKind::Currency => (&cleaned[..i], &cleaned[i..]),
                    _ => (cleaned.as_str(), ""),
                };
                let code_ok = code.is_empty()
                    || (code.len() == 3 && code.bytes().all(|b| b.is_ascii_alphabetic()));
                match num.parse::<f64>() {
                    Ok(v) if code_ok && v.is_finite() => Ok(if code.is_empty() {
                        num.to_string()
                    } else {
                        format!("{num} {}", code.to_ascii_uppercase())
                    }),
                    _ => misfit(if self.kind == AnswerKind::Currency {
                        "an amount as a plain number, with a currency code only if not USD"
                    } else {
                        "a number with no units or symbols"
                    }),
                }
            }
            AnswerKind::Choice => {
                let want = bare.to_ascii_lowercase();
                match self
                    .options
                    .iter()
                    .find(|o| o.trim().to_ascii_lowercase() == want)
                {
                    Some(o) => Ok(o.clone()),
                    None => misfit(&format!("one of {}", self.options.join(", "))),
                }
            }
            AnswerKind::Quote => {
                if raw.is_empty() {
                    misfit("the verbatim passage")
                } else {
                    Ok(raw
                        .trim_matches(['"', '\u{201c}', '\u{201d}'])
                        .trim()
                        .into())
                }
            }
            AnswerKind::Pattern => {
                let re = self.options.first().map(String::as_str).unwrap_or("");
                match regex::Regex::new(&format!("^(?:{re})$")) {
                    Ok(r) if r.is_match(bare) => Ok(bare.into()),
                    _ => misfit(&format!("a value matching {re}")),
                }
            }
            AnswerKind::Text => {
                if raw.is_empty() {
                    misfit("one short sentence")
                } else {
                    Ok(raw.into())
                }
            }
            AnswerKind::Json => {
                let Some((text, value)) = json_in(raw) else {
                    return misfit(&self.wants());
                };
                let missing: Vec<&str> = self
                    .options
                    .iter()
                    .map(String::as_str)
                    .filter(|k| value.get(k).is_none())
                    .collect();
                if missing.is_empty() {
                    Ok(text.into())
                } else {
                    Err(format!(
                        "the JSON has no {}. {}",
                        missing.join(", "),
                        self.sentence()
                    ))
                }
            }
        }
    }
}

/// The JSON an answer holds: the whole of it, or the one object or array it
/// wraps in a code fence or a sentence. Kept as the agent wrote it.
fn json_in(raw: &str) -> Option<(&str, serde_json::Value)> {
    let whole = |t: &str| {
        serde_json::from_str::<serde_json::Value>(t)
            .ok()
            .filter(|v| v.is_object() || v.is_array())
    };
    let t = raw.trim();
    if let Some(v) = whole(t) {
        return Some((t, v));
    }
    let start = t.find(['{', '['])?;
    let end = t.rfind(['}', ']'])?;
    let inner = t.get(start..=end)?;
    whole(inner).map(|v| (inner, v))
}

impl AnswerType {
    /// A run's printed output against the type. Agents often explain, then
    /// give the answer on the last line: when the whole doesn't fit, the last
    /// non-empty line may.
    pub fn check_printed(&self, printed: &str) -> Result<String, String> {
        self.check(printed).or_else(|e| {
            let last = printed
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            if last.trim() == printed.trim() {
                return Err(e);
            }
            self.check(last).map_err(|_| e)
        })
    }
}

/// An agent's way of saying the document doesn't answer the question.
pub fn is_not_found(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "not found" | "not stated" | "n/a" | "none found"
    )
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.chars().count() > 60 {
        let cut: String = line.chars().take(57).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

impl Cell {
    /// A blank literal; callers set the fields that differ.
    pub fn new(name: &str) -> Cell {
        Cell {
            name: name.to_string(),
            kind: CellKind::Literal,
            source: None,
            value: String::new(),
            deps: Vec::new(),
            state: CellState::Ok,
            exit_code: None,
            stderr: String::new(),
            duration_ms: None,
            confirmed: false,
            series: false,
            def_by: None,
            decision: None,
            pending_suggestions: Vec::new(),
            mark: None,
            format: None,
            cite: None,
            answer_type: None,
            usage: None,
            session_id: None,
            activity: None,
            provenance: Provenance::now(Actor::Human),
        }
    }

    pub fn verified(&self) -> bool {
        self.mark
            .as_ref()
            .is_some_and(|m| m.kind == MarkKind::Verified)
    }
}

/// Cell names: `[A-Za-z_][A-Za-z0-9_]*`, A1 names included, or an A1 name on
/// another sheet, `Notes!A2` (see `valid_sheet`). Also keeps a name safe as a
/// path under .recalc/.
pub fn valid_name(name: &str) -> bool {
    match split_sheet(name) {
        (Some(sheet), local) => valid_sheet(sheet) && a1_coord(local).is_some(),
        (None, local) => plain_name(local),
    }
}

/// `[A-Za-z_][A-Za-z0-9_]*`: a name on its own sheet.
fn plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A sheet name, by Excel's rules: 1–31 characters, none of `/ \ ? * : [ ]`,
/// no apostrophe or space at either end, not `History`. Also none of
/// `! { } " , ;`, which recalc's sources use around references, in lists and
/// between commands, and nothing a template line would misread.
pub fn valid_sheet(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 31
        && !name.contains(['/', '\\', '?', '*', ':', '[', ']', '!', '{', '}', '"', ','])
        && !name.chars().any(char::is_control)
        && !name.starts_with(['\'', ' '])
        && !name.ends_with(['\'', ' '])
        && !name.eq_ignore_ascii_case("history")
        // A template line starting `#` is a comment and `@` a directive; ` = `
        // and ` <<` separate a name from its content; `;` ends a command.
        && !name.starts_with(['#', '@'])
        && !name.contains(" = ")
        && !name.contains(" <<")
        && !name.contains(';')
}

/// A cell's sheet and its name on that sheet: `Notes!A2` → `(Some("Notes"),
/// "A2")`, `A2` → `(None, "A2")`. For names as recalc keeps them, unquoted.
pub fn split_sheet(name: &str) -> (Option<&str>, &str) {
    match name.rsplit_once('!') {
        Some((sheet, local)) => (Some(sheet), local),
        None => (None, name),
    }
}

/// The name of `local` on `sheet`; the first sheet's cells have no prefix.
pub fn on_sheet(sheet: Option<&str>, local: &str) -> String {
    match sheet {
        Some(sheet) => format!("{sheet}!{local}"),
        None => local.to_string(),
    }
}

/// A sheet as a reference writes it: quoted when it isn't a plain word,
/// `'My Notes'`, with an apostrophe inside doubled, as Excel does.
pub fn quote_sheet(sheet: &str) -> String {
    if plain_name(sheet) && a1_coord(sheet).is_none() {
        sheet.to_string()
    } else {
        format!("'{}'", sheet.replace('\'', "''"))
    }
}

/// A cell's name as a reference writes it: `'My Notes'!A2`.
pub fn ref_text(name: &str) -> String {
    match split_sheet(name) {
        (Some(sheet), local) => format!("{}!{local}", quote_sheet(sheet)),
        (None, local) => local.to_string(),
    }
}

/// Split a reference's sheet off, as written in a source: `Notes!A2`,
/// `'My Notes'!$A$2`. Returns the sheet unquoted and the rest as written.
/// None for a malformed quote.
pub fn ref_sheet(text: &str) -> Option<(Option<String>, &str)> {
    let text = text.trim();
    if let Some(quoted) = text.strip_prefix('\'') {
        // 'It''s'!A1: a doubled apostrophe is one.
        let mut sheet = String::new();
        let mut chars = quoted.char_indices();
        while let Some((i, c)) = chars.next() {
            if c != '\'' {
                sheet.push(c);
                continue;
            }
            if quoted[i + 1..].starts_with('\'') {
                sheet.push('\'');
                chars.next();
                continue;
            }
            let rest = quoted[i + 1..].strip_prefix('!')?;
            return Some((Some(sheet), rest));
        }
        return None;
    }
    Some(match text.split_once('!') {
        Some((sheet, rest)) => (Some(sheet.to_string()), rest),
        None => (None, text),
    })
}

/// The cell a reference names, with `$` anchors stripped (`$A$1` → `A1`,
/// `'My Notes'!$A$2` → `My Notes!A2`). Anchors only affect how fill shifts
/// the reference. An unqualified A1 ref means the first sheet here; a host
/// on another sheet resolves it with `ref_name_on`.
pub fn ref_name(text: &str) -> Option<String> {
    ref_name_on(text, None)
}

/// `ref_name` for a reference written in a cell on `host`: an unqualified A1
/// ref means the host's own sheet, as in Excel. Named cells are workbook-wide.
pub fn ref_name_on(text: &str, host: Option<&str>) -> Option<String> {
    let (sheet, rest) = ref_sheet(text)?;
    let local = if plain_name(rest) && a1_coord(rest).is_none() {
        // A named cell: workbook-wide, never on a sheet.
        if sheet.is_some() {
            return None;
        }
        return Some(rest.to_string());
    } else {
        let r = A1Ref::parse(rest)?;
        a1_name(r.col, r.row)
    };
    let sheet = sheet.as_deref().or(host);
    if sheet.is_some_and(|s| !valid_sheet(s)) {
        return None;
    }
    Some(on_sheet(sheet, &local))
}

/// The named cell a live session reports its state in: `B2_status`, and
/// for a cell on another sheet `Notes_B2_status` (a sheet's other characters
/// become `_`), since named cells are workbook-wide.
pub fn status_name(cell: &str) -> String {
    let flat: String = cell
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let flat = if flat.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{flat}")
    } else {
        flat
    };
    format!("{flat}_status")
}

/// A grid cell's sheet and coordinates: `Notes!B3` → `(Some("Notes"), 2, 3)`.
/// None for named cells.
pub fn cell_coord(name: &str) -> Option<(Option<&str>, u32, u32)> {
    let (sheet, local) = split_sheet(name);
    let (col, row) = a1_coord(local)?;
    Some((sheet, col, row))
}

/// An A1 reference with its anchors: `$B$2` is fixed in both directions,
/// `B$2` shifts by column only, `$B2` by row only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct A1Ref {
    pub col: u32,
    pub row: u32,
    pub col_fixed: bool,
    pub row_fixed: bool,
}

impl A1Ref {
    pub fn parse(text: &str) -> Option<A1Ref> {
        let (col_fixed, rest) = match text.strip_prefix('$') {
            Some(r) => (true, r),
            None => (false, text),
        };
        let letters = rest.chars().take_while(|c| c.is_ascii_uppercase()).count();
        if letters == 0 {
            return None;
        }
        let (col_s, row_s) = rest.split_at(letters);
        let (row_fixed, row_s) = match row_s.strip_prefix('$') {
            Some(r) => (true, r),
            None => (false, row_s),
        };
        let (col, row) = a1_coord(&format!("{col_s}{row_s}"))?;
        Some(A1Ref {
            col,
            row,
            col_fixed,
            row_fixed,
        })
    }

    /// Shift the unanchored axes; the text form keeps its anchors. None
    /// when the shift would leave the sheet (above row 1, left of A).
    pub fn shifted(self, dcol: i64, drow: i64) -> Option<String> {
        let axis = |n: u32, fixed: bool, d: i64| -> Option<u32> {
            if fixed {
                return Some(n);
            }
            u32::try_from(i64::from(n) + d).ok().filter(|v| *v > 0)
        };
        let col = axis(self.col, self.col_fixed, dcol)?;
        let row = axis(self.row, self.row_fixed, drow)?;
        let name = a1_name(col, row);
        let letters = name.trim_end_matches(|c: char| c.is_ascii_digit());
        Some(format!(
            "{}{letters}{}{row}",
            if self.col_fixed { "$" } else { "" },
            if self.row_fixed { "$" } else { "" },
        ))
    }
}

/// Parse an A1-notation name into 1-based (column, row): `"AA10"` → `(27, 10)`.
/// None for anything else (lowercase, no digits, row 0).
pub fn a1_coord(name: &str) -> Option<(u32, u32)> {
    let letters = name.chars().take_while(|c| c.is_ascii_uppercase()).count();
    if letters == 0 {
        return None;
    }
    let (col_s, row_s) = name.split_at(letters);
    if row_s.is_empty() || !row_s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let col = col_s
        .chars()
        .fold(0u32, |acc, c| acc * 26 + (c as u32 - 'A' as u32 + 1));
    let row: u32 = row_s.parse().ok()?;
    (row > 0).then_some((col, row))
}

/// Inverse of `a1_coord`: `(27, 10)` → `"AA10"`.
pub fn a1_name(col: u32, row: u32) -> String {
    let mut letters = Vec::new();
    let mut c = col;
    while c > 0 {
        letters.push(b'A' + ((c - 1) % 26) as u8);
        c = (c - 1) / 26;
    }
    letters.reverse();
    format!(
        "{}{row}",
        String::from_utf8(letters).expect("ASCII letters")
    )
}

/// Expand a rectangular A1 range into member names, corners in either order.
/// Corners may carry `$` anchors. The first corner may name a sheet
/// (`Notes!A2:C5`), which the second shares.
pub fn range_cells(from: &str, to: &str) -> Option<Vec<String>> {
    range_cells_on(from, to, None)
}

/// `range_cells` for a range written in a cell on `host` (see `ref_name_on`).
pub fn range_cells_on(from: &str, to: &str, host: Option<&str>) -> Option<Vec<String>> {
    let first = ref_name_on(from, host)?;
    let (sheet, c1, r1) = cell_coord(&first)?;
    let (to_sheet, to_rest) = ref_sheet(to)?;
    // `Notes!A2:Notes!C5` is allowed; `Notes!A2:Other!C5` isn't a range.
    if to_sheet.as_deref().is_some_and(|t| Some(t) != sheet) {
        return None;
    }
    let to = A1Ref::parse(to_rest)?;
    let (c2, r2) = (to.col, to.row);
    let (lo_c, hi_c) = (c1.min(c2), c1.max(c2));
    let (lo_r, hi_r) = (r1.min(r2), r1.max(r2));
    let mut cells = Vec::new();
    for row in lo_r..=hi_r {
        for col in lo_c..=hi_c {
            cells.push(on_sheet(sheet, &a1_name(col, row)));
        }
    }
    Some(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_on_other_sheets_follow_excel() {
        assert!(
            valid_name("Notes!A2")
                && valid_name("My Notes!B10")
                && valid_name("A2")
                && valid_name("status")
        );
        assert!(!valid_name("Notes!status"), "a sheet holds only A1 cells");
        assert!(!valid_name("Bad:Name!A1") && !valid_name("!A1") && !valid_name("History!A1"));
        assert!(valid_sheet("Q3 plan") && valid_sheet("It's") && valid_sheet(&"x".repeat(31)));
        for bad in [
            "",
            "a/b",
            "a\\b",
            "a?",
            "a*",
            "a:b",
            "[x]",
            "a!b",
            "{x}",
            "\"x",
            "'x",
            "x'",
            " x",
            "x ",
            "history",
            &"x".repeat(32),
        ] {
            assert!(!valid_sheet(bad), "{bad:?}");
        }
        assert_eq!(split_sheet("My Notes!A2"), (Some("My Notes"), "A2"));
        assert_eq!(cell_coord("Notes!B3"), Some((Some("Notes"), 2, 3)));
        assert_eq!(cell_coord("B3"), Some((None, 2, 3)));
        assert_eq!(cell_coord("status"), None);
        assert_eq!(ref_text("My Notes!A2"), "'My Notes'!A2");
        assert_eq!(ref_text("It's!A2"), "'It''s'!A2");
        assert_eq!(ref_text("Notes!A2"), "Notes!A2");
        assert_eq!(
            ref_text("B1!A2"),
            "'B1'!A2",
            "a sheet that looks like a cell is quoted"
        );
    }

    #[test]
    fn a_sessions_status_cell_is_a_valid_name() {
        assert_eq!(status_name("B2"), "B2_status");
        assert_eq!(status_name("review"), "review_status");
        assert_eq!(status_name("Notes!B2"), "Notes_B2_status");
        assert_eq!(status_name("My Notes!B2"), "My_Notes_B2_status");
        assert_eq!(status_name("2024!A1"), "_2024_A1_status");
        for c in ["B2", "My Notes!B2", "2024!A1", "v1.2!C3"] {
            assert!(valid_name(&status_name(c)), "{c}");
        }
    }

    #[test]
    fn references_resolve_on_their_sheet() {
        assert_eq!(ref_name("Notes!$A$2").as_deref(), Some("Notes!A2"));
        assert_eq!(ref_name("'My Notes'!B3").as_deref(), Some("My Notes!B3"));
        assert_eq!(ref_name("'It''s'!B3").as_deref(), Some("It's!B3"));
        assert_eq!(ref_name("'unclosed!B3"), None);
        assert_eq!(
            ref_name_on("A2", Some("Notes")).as_deref(),
            Some("Notes!A2"),
            "unqualified means the host's sheet"
        );
        assert_eq!(
            ref_name_on("status", Some("Notes")).as_deref(),
            Some("status"),
            "named cells are workbook-wide"
        );
        assert_eq!(
            ref_name_on("Other!A2", Some("Notes")).as_deref(),
            Some("Other!A2")
        );
        assert_eq!(ref_name("Notes!status"), None);
        assert_eq!(
            range_cells("Notes!A1", "B2").unwrap(),
            vec!["Notes!A1", "Notes!B1", "Notes!A2", "Notes!B2"]
        );
        assert_eq!(
            range_cells("'My Notes'!A1", "'My Notes'!A2").unwrap(),
            vec!["My Notes!A1", "My Notes!A2"]
        );
        assert_eq!(range_cells("Notes!A1", "Other!B2"), None);
        assert_eq!(
            range_cells_on("A1", "A2", Some("Notes")).unwrap(),
            vec!["Notes!A1", "Notes!A2"]
        );
    }

    #[test]
    fn kind_from_source() {
        assert_eq!(CellKind::of_source("42"), CellKind::Literal);
        assert_eq!(CellKind::of_source("= {a} + 1"), CellKind::Expr);
        assert_eq!(CellKind::of_source("  $ cargo test"), CellKind::Command);
        assert_eq!(CellKind::of_source("$echo hi"), CellKind::Command);
        // `$` then a number is an amount someone typed, not a command.
        for amount in ["$1,200", "$5", "$-3", "$.50", " $1200.00"] {
            assert_eq!(CellKind::of_source(amount), CellKind::Literal, "{amount}");
        }
        assert_eq!(CellKind::of_source("! git commit"), CellKind::Action);
        assert_eq!(CellKind::of_source("@claude('hi')"), CellKind::AgentCall);
        assert_eq!(CellKind::of_source(""), CellKind::Literal);
    }

    #[test]
    fn a1_coords_round_trip() {
        assert_eq!(a1_coord("A1"), Some((1, 1)));
        assert_eq!(a1_coord("Z9"), Some((26, 9)));
        assert_eq!(a1_coord("AA10"), Some((27, 10)));
        assert_eq!(a1_coord("test_count"), None);
        assert_eq!(a1_coord("A0"), None);
        assert_eq!(a1_coord("a1"), None);
        assert_eq!(a1_name(27, 10), "AA10");
        assert_eq!(a1_name(1, 1), "A1");
        assert_eq!(a1_name(52, 3), "AZ3");
    }

    #[test]
    fn range_expansion() {
        assert_eq!(range_cells("A1", "A3").unwrap(), ["A1", "A2", "A3"]);
        assert_eq!(range_cells("B2", "A1").unwrap(), ["A1", "B1", "A2", "B2"]);
        assert_eq!(range_cells("$A$1", "A$2").unwrap(), ["A1", "A2"]);
        assert!(range_cells("A1", "notA1").is_none());
    }

    #[test]
    fn anchored_refs_name_the_cell_and_shift_by_axis() {
        assert_eq!(ref_name("$B$2").as_deref(), Some("B2"));
        assert_eq!(ref_name("B$2").as_deref(), Some("B2"));
        assert_eq!(ref_name(" $B2 ").as_deref(), Some("B2"));
        assert_eq!(ref_name("total").as_deref(), Some("total"));
        assert_eq!(ref_name("$total"), None);
        assert_eq!(ref_name("B$"), None);

        let r = A1Ref::parse("$B$2").unwrap();
        assert_eq!(r.shifted(3, 5).unwrap(), "$B$2");
        assert_eq!(A1Ref::parse("B$2").unwrap().shifted(3, 5).unwrap(), "E$2");
        assert_eq!(A1Ref::parse("$B2").unwrap().shifted(3, 5).unwrap(), "$B7");
        assert_eq!(A1Ref::parse("B2").unwrap().shifted(3, 5).unwrap(), "E7");
        assert_eq!(A1Ref::parse("Z1").unwrap().shifted(1, 0).unwrap(), "AA1");
        assert_eq!(A1Ref::parse("B2").unwrap().shifted(-1, -1).unwrap(), "A1");
        assert_eq!(A1Ref::parse("B2").unwrap().shifted(-2, 0), None);
        assert_eq!(A1Ref::parse("$B2").unwrap().shifted(-5, 0).unwrap(), "$B2");
    }

    #[test]
    fn a_typed_value_takes_its_canonical_form_or_says_what_fits() {
        let yn = AnswerType::new(AnswerKind::YesNo, &[]);
        assert_eq!(yn.typed("y"), Ok("Yes".to_string()));
        assert_eq!(yn.typed("maybe"), Err("Yes or No".to_string()));
        let pick = AnswerType::new(AnswerKind::Choice, &["Approve".into(), "Reject".into()]);
        assert_eq!(pick.typed("approve"), Ok("Approve".to_string()));
        assert_eq!(
            pick.typed("later"),
            Err("one of Approve, Reject".to_string())
        );
        let amount = AnswerType::new(AnswerKind::Currency, &[]);
        assert_eq!(amount.typed("$1,200"), Ok("1200".to_string()));
        let date = AnswerType::new(AnswerKind::Date, &[]);
        assert_eq!(
            date.typed("10/3/2026"),
            Err("a date as YYYY-MM-DD".to_string())
        );
    }

    #[test]
    fn an_agents_answer_is_read_as_agents_write_it() {
        let status = AnswerType {
            kind: AnswerKind::Choice,
            options: vec!["done".into(), "partial".into(), "failed".into()],
            strict: true,
            allows_not_found: true,
        };
        for printed in [
            "done",
            "done.",
            "**done**",
            "`done`",
            "\"Done\"",
            "I wrote out/t5.csv with 4,420 rows.\n\ndone",
            "Checked every row.\n**done**\n",
        ] {
            assert_eq!(
                status.check_printed(printed),
                Ok("done".into()),
                "{printed:?}"
            );
        }
        // Prose with no answer of its own is still refused, saying what's wanted.
        let e = status
            .check_printed("I wrote out/t5.csv with 4,420 rows.")
            .unwrap_err();
        assert!(e.contains("one of done, partial, failed"), "{e}");
        // A last line counts only for an agent's printed output, not a typed value.
        assert!(status.check("Notes\ndone").is_err());
        let yes = AnswerType {
            kind: AnswerKind::YesNo,
            options: vec![],
            strict: true,
            allows_not_found: true,
        };
        assert_eq!(
            yes.check_printed("The clause requires consent.\nYes"),
            Ok("Yes".into())
        );
        let date = AnswerType {
            kind: AnswerKind::Date,
            options: vec![],
            strict: true,
            allows_not_found: true,
        };
        assert_eq!(date.check("`2026-03-01`"), Ok("2026-03-01".into()));
    }

    #[test]
    fn answer_types_check_and_canonicalize() {
        let yn = AnswerType::new(AnswerKind::YesNo, &[]);
        assert_eq!(yn.check("yes.").unwrap(), "Yes");
        assert_eq!(yn.check("NO").unwrap(), "No");
        assert_eq!(yn.check("not stated").unwrap(), "Not found");
        let err = yn.check("Yes, because clause 4 says so").unwrap_err();
        assert!(
            err.contains("expected Yes or No") && err.contains("Answer Yes or No."),
            "{err}"
        );

        let d = AnswerType::new(AnswerKind::Date, &[]);
        assert_eq!(d.check(" 2024-01-31 ").unwrap(), "2024-01-31");
        assert!(d.check("January 31, 2024").is_err());

        let n = AnswerType::new(AnswerKind::Number, &[]);
        assert_eq!(n.check("1,200").unwrap(), "1200");
        assert_eq!(n.check("-3.5").unwrap(), "-3.5");
        assert!(n.check("about 12").is_err());
        assert!(n.check("12 months").is_err());

        let c = AnswerType::new(AnswerKind::Currency, &[]);
        assert_eq!(c.check("$3,500.00").unwrap(), "3500.00");
        assert_eq!(c.check("1200 eur").unwrap(), "1200 EUR");
        assert!(c.check("twelve months' fees").is_err());

        let ch = AnswerType::new(
            AnswerKind::Choice,
            &["Consent required".into(), "Notice only".into()],
        );
        assert_eq!(ch.check("consent required.").unwrap(), "Consent required");
        assert!(ch
            .check("Consent")
            .unwrap_err()
            .contains("one of Consent required, Notice only"));

        let q = AnswerType::new(AnswerKind::Quote, &[]);
        assert_eq!(
            q.check("\u{201c}may not assign\u{201d}").unwrap(),
            "may not assign"
        );
        assert!(q.check("").is_err());

        let t = AnswerType::new(AnswerKind::Text, &[]);
        assert_eq!(t.check("Delaware").unwrap(), "Delaware");
        assert!("chart".parse::<AnswerKind>().is_err());
        // A pattern: the whole answer must match.
        let id = AnswerType::new(AnswerKind::Pattern, &["CVE-\\d{4}-\\d+".into()]);
        assert_eq!(id.check("CVE-2026-10412").unwrap(), "CVE-2026-10412");
        assert!(id.check("see CVE-2026-10412").is_err());
        assert!(AnswerType::new(AnswerKind::Pattern, &["(".into()])
            .validate()
            .is_err());
        // JSON: the object itself, from a code fence or a sentence around it.
        let j = AnswerType::new(AnswerKind::Json, &["case_id".into(), "prediction".into()]);
        let fenced = "Here it is:\n```json\n{\"case_id\": \"C19\", \"prediction\": \"crl\"}\n```";
        assert_eq!(
            j.check(fenced).unwrap(),
            "{\"case_id\": \"C19\", \"prediction\": \"crl\"}"
        );
        let short = j.check("{\"case_id\": \"C19\"}").unwrap_err();
        assert!(short.contains("no prediction"), "{short}");
        assert!(j.check("crl, probably").is_err());
        assert!(j.sentence().contains("with the keys case_id, prediction"));
        let any = AnswerType::new(AnswerKind::Json, &[]);
        assert_eq!(any.check("[1, 2]").unwrap(), "[1, 2]");
        assert!(
            any.check("42").is_err(),
            "a bare number isn't an object or array"
        );
        assert_eq!("json".parse::<AnswerKind>().unwrap(), AnswerKind::Json);
        // "Not found" fits unless the type says otherwise.
        let mut n = AnswerType::new(AnswerKind::Number, &[]);
        assert_eq!(n.check("n/a").unwrap(), "Not found");
        n.allows_not_found = false;
        assert!(n.check("n/a").is_err());
        assert!(AnswerType::new(AnswerKind::Choice, &["a".into()])
            .validate()
            .is_err());
    }

    #[test]
    fn name_validity() {
        assert!(valid_name("A1"));
        assert!(valid_name("AA10"));
        assert!(valid_name("test_count"));
        assert!(valid_name("_x"));
        assert!(!valid_name(""));
        assert!(!valid_name("1abc"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("a.b"));
        assert!(!valid_name("a b"));
    }
}
