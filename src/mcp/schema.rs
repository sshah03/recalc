//! The tool list and argument checking.

use super::*;

pub(super) fn cite_schema() -> Value {
    json!({"type": "object", "description": "the passage behind the value", "properties": {
        "file": {"type": "string", "description": "document path, relative to the board directory"},
        "quote": {"type": "string", "description": "the supporting passage, verbatim"},
        "where": {"type": "string", "description": "optional locator: section, clause, page"}},
        "required": ["file", "quote"]})
}

/// An argument the tool doesn't take, or a required one left out.
pub(super) fn argument_problem(tool: &str, args: &Value) -> Option<String> {
    static SCHEMAS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let tools = SCHEMAS.get_or_init(tool_list);
    let schema = &tools.as_array()?.iter().find(|t| t["name"] == tool)?["inputSchema"];
    let props = schema["properties"].as_object()?;
    let takes = || props.keys().cloned().collect::<Vec<_>>().join(", ");
    let given = args.as_object();
    if let Some(extra) = given
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
        .find(|k| !props.contains_key(*k))
    {
        return Some(format!(
            "{tool} has no argument {extra:?}; it takes {}",
            takes()
        ));
    }
    let missing: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|r| given.is_none_or(|g| !g.contains_key(*r)))
        .collect();
    if !missing.is_empty() {
        return Some(format!(
            "{tool} needs {}; it takes {}",
            missing.join(" and "),
            takes()
        ));
    }
    None
}

pub(super) fn tool_list() -> Value {
    let string = |desc: &str| json!({"type": "string", "description": desc});
    let tools = [
        ("list_cells", "Snapshot of every cell, on every sheet: name, kind, value, state, provenance. A cell on another sheet is named Notes!A2.",
         json!({"type": "object", "properties": {}})),
        ("get_cell", "Full detail for one cell: value, source, state, deps, dependents, review mark, any pending decision and unadopted human suggestions. Cells you read are tracked: if one changes later, your next tool result carries a second block {\"changed_inputs\": [{name, was, now}]}.",
         json!({"type": "object", "properties": {"name": string("cell name")}, "required": ["name"]})),
        ("set_cell", "Set a cell. Content starting with = $ ! @ installs a formula/command/agent source; anything else is a literal value. Writing over a cell a human has verified records your value as a suggestion instead of overwriting it. A cell with an answer type (answer_type on get_cell) takes only an answer that fits (Yes/No, a date, a number, one of its options) and refuses anything else, saying what it wanted; answer again in that form, never with extra words. When the value comes from a document, pass cite so a reviewer can open the passage from the cell. Pass if_unchanged when the value was derived from cells you read: the write is refused, with the diff, if they moved since. The result says what landed: `written` when the answer type normalized your answer, `unchanged` when the cell already held it, `outside_answer_type` when a lenient answer type let a misfit through.",
         json!({"type": "object", "properties": {
             "name": string("cell name"),
             "content": string("value or =/$/! source"),
             "note": string("optional context recorded in provenance"),
             "cite": cite_schema(),
             "if_unchanged": {"description": "true: refuse if any cell you read has changed since; or a list of cell names to check", "anyOf": [{"type": "boolean"}, {"type": "array", "items": {"type": "string"}}]}},
             "required": ["name", "content"]})),
        ("set_answer_type", "Give answer cells the type their answers must take: the check behind a question column. Type the answers (range B2:B7), not the header; an answer type aimed at a document-grid header lands on the agent cells under it, and the result says where. kind is one of text, yesno, date, number, currency, choice (with options), quote, pattern (options: [one regex] the whole answer must match), json (one JSON object or array; options: top-level keys it must have); strict (default true) refuses a write that doesn't fit, otherwise the misfit is only noted; not_found (default true) lets \"Not found\" fit, false when every row must have a value. kind \"none\" clears it.",
         json!({"type": "object", "properties": {
             "name": string("a cell, or a grid header (its answers get the answer type)"),
             "range": string("the answer cells, like B2:B7, instead of name"),
             "kind": {"type": "string", "enum": ["text", "yesno", "date", "number", "currency", "choice", "quote", "pattern", "json", "none"]},
             "options": {"type": "array", "items": {"type": "string"}, "description": "the allowed answers, for kind choice; the regex, for kind pattern; the keys the object must have, for kind json"},
             "strict": {"type": "boolean"},
             "not_found": {"type": "boolean", "description": "whether \"Not found\" fits; default true"}},
             "required": ["kind"]})),
        ("cite_cell", "Attach the passage that supports a cell's value: the source (a path inside the board directory, cell:<name> for another cell's value, or an http(s) URL, which isn't checked) and the verbatim quote, plus an optional locator like \"§7.2\" or \"page 3\". Use it for your own cell before you answer, or for any cell you wrote. Called with only a name, it clears the citation.",
         json!({"type": "object", "properties": {
             "name": string("cell name"),
             "file": string("path to the document, relative to the board directory"),
             "quote": string("the supporting passage, verbatim"),
             "where": string("optional locator for people: a section, clause, or page")},
             "required": ["name"]})),
        ("get_range", "Read many cells in one call: a 2-D block between two A1 corners (from, to; returns [[value]]), or any cells by name (names: [...]; returns {name: value}), named cells like plan_1 included. Every cell read is registered for changed_inputs.",
         json!({"type": "object", "properties": {"from": string("one corner, e.g. A1"), "to": string("the other corner, e.g. C10"), "names": {"type": "array", "items": {"type": "string"}, "description": "cells by name, named or A1, instead of from/to"}}})),
        ("set_range", "Write many cells in one call and one recompute pass, each routed like set_cell: a 2-D block from an A1 anchor (from, values: [[content]]), or any cells by name (cells: {name: content}), named cells like plan_1 and plan_1_status included. Prefer it over many set_cell calls when filling a column or laying out a plan. cites mirrors the shape you used: [[{file, quote, where} or null]] for a block, {name: cite} for cells. Cells with an answer type refuse answers that don't fit; the error names them. if_unchanged works as on set_cell.",
         json!({"type": "object", "properties": {
             "from": string("anchor, e.g. A2 (the block form)"),
             "values": {"type": "array", "items": {"type": "array", "items": {"type": "string"}}},
             "cells": {"type": "object", "additionalProperties": {"type": "string"}, "description": "cells by name, named or A1, and their contents (instead of from/values)"},
             "cites": {"description": "[[cite or null]] mirroring values, or {name: cite} mirroring cells", "anyOf": [{"type": "array", "items": {"type": "array", "items": cite_schema()}}, {"type": "object", "additionalProperties": cite_schema()}]},
             "if_unchanged": {"description": "true, or a list of cell names: refuse the write if they changed since you read them", "anyOf": [{"type": "boolean"}, {"type": "array", "items": {"type": "string"}}]}}})),
        ("trigger_cell", "Explicitly run a ! action or @ agent cell (the only way they run) or force-refresh a $ cell. Returns the run's output, or {running: true} if it outlasts a 45s block; then await_cell on it. An ! action an agent wrote runs first by a person's hand.",
         json!({"type": "object", "properties": {"name": string("cell name")}, "required": ["name"]})),
        ("run_range", "Run the @ agent, ! action and $ cells in a rectangle that are out of date (never run, stale, or errored), as the person's Run Selection does, and return at once with what was queued; all: true re-runs current ones too. For filling a table: they run max_agents at a time, each after the cells it reads. Skips verified cells and ones already running; an ! action an agent wrote still waits for a person's first run.",
         json!({"type": "object", "properties": {"from": string("top-left corner, e.g. B2"), "to": string("bottom-right corner, e.g. L6"), "all": {"type": "boolean", "description": "re-run cells whose answers are current too"}}, "required": ["from", "to"]})),
        ("fill_range", "Repeat the first cell of each row or column across a block, shifting A1 references as a person's fill does ($ anchors stay put): write a prompt or formula once in B2, then fill B2:L6. direction: down (default) or right; a single-row block fills right.",
         json!({"type": "object", "properties": {"from": string("top-left corner, e.g. B2"), "to": string("bottom-right corner, e.g. L6"), "direction": string("down or right")}, "required": ["from", "to"]})),
        ("edit_structure", "Insert or delete rows or columns on one sheet, as a person does: cells below or right move, and references to them follow from any sheet (a reference to a deleted cell becomes #REF!). Refused while a cell runs or an agent cell's terminal session is live.",
         json!({"type": "object", "properties": {"action": string("insert or delete"), "axis": string("rows or columns"), "at": {"type": "integer", "description": "1-based row or column number"}, "count": {"type": "integer", "description": "how many, default 1"}, "sheet": string("which sheet, e.g. Notes; default the first")}, "required": ["action", "axis", "at"]})),
        ("sort_rows", "Sort rows first..last of one sheet by a column (numbers before text, blanks last). Cells move whole and references into them follow. Refused while a cell runs or an agent cell's terminal session is live.",
         json!({"type": "object", "properties": {"column": string("column letter, e.g. C"), "first": {"type": "integer"}, "last": {"type": "integer"}, "descending": {"type": "boolean"}, "sheet": string("which sheet, e.g. Notes; default the first")}, "required": ["column", "first", "last"]})),
        ("rename_sheet", "Rename a sheet, the first one (Sheet1) included. Its cells stay where they are and every reference to the sheet, from any sheet, follows the new name. Refused while one of its cells runs, or when another sheet has the name.",
         json!({"type": "object", "properties": {"from": string("the sheet's current name, e.g. Sheet1"), "to": string("its new name, e.g. Runs")}, "required": ["from", "to"]})),
        ("duplicate_sheet", "Copy a sheet to a new one after it, as a fresh copy: every cell as written (formulas, commands and agent prompts; values as they are), with formats and answer types, but no agents' answers, marks or citations. Unqualified references in the copy mean its own sheet, so a workpaper sheet becomes the next client's by changing its inputs. Named cells stay the board's.",
         json!({"type": "object", "properties": {"from": string("the sheet to copy, e.g. Harbor"), "to": string("the new sheet's name, e.g. Pinecrest")}, "required": ["from", "to"]})),
        ("stop_cell", "Stop a cell's run, or take it out of the queue. It keeps its last value and shows as out of date.",
         json!({"type": "object", "properties": {"name": string("cell name")}, "required": ["name"]})),
        ("answer_cell", "Answer an @ agent cell yourself, instead of running it as an agent of its own: when the person asks you to answer cells (\"answer B2:B7 yourself\"). Read its prompt and inputs with get_cell first. The value lands as the cell's answer and its prompt stays, so Run can still ask its own agent later; set_cell on an agent cell would replace the prompt. It must fit the cell's answer type; a verified cell takes it as a suggestion. Pass cite when the answer rests on a document.",
         json!({"type": "object", "properties": {
             "name": string("the @ agent cell"),
             "value": string("your answer"),
             "note": string("optional: one line on how you got it"),
             "cite": {"type": "object", "properties": {"file": {"type": "string"}, "quote": {"type": "string"}, "where": {"type": "string"}}, "required": ["file", "quote"]}},
             "required": ["name", "value"]})),
        ("set_format", "Set how cells display: bold, italic, align (left, center, right), num (text, integer, number, currency, percent, date), decimals (0-10), currency (ISO code, e.g. EUR), fill (yellow, green, blue, red, purple, gray), wrap (long text on several lines), highlight (fills that follow the value, first match wins: [{when: is|contains|above|below, value, fill}], e.g. a status column with \"fail\" in red). Give name or an A1 range; omit every field to clear. It replaces the cell's whole format.",
         json!({"type": "object", "properties": {"name": string("cell name"), "range": string("A1 range, e.g. C2:C9"), "bold": {"type": "boolean"}, "italic": {"type": "boolean"}, "align": string("left, center or right"), "num": string("text, integer, number, currency, percent or date"), "decimals": {"type": "integer", "minimum": 0, "maximum": 10}, "currency": string("ISO 4217 code: USD, EUR, GBP, JPY…"), "fill": string("yellow, green, blue, red, purple or gray"), "wrap": {"type": "boolean"}, "highlight": {"type": "array", "items": {"type": "object", "properties": {"when": string("is, contains, above or below"), "value": string("text to match, or a number for above/below"), "fill": string("yellow, green, blue, red, purple or gray")}, "required": ["when", "fill"]}}}})),
        ("append_cell", "Write a value into a series cell; its history records the point (for data you emit over time).",
         json!({"type": "object", "properties": {"name": string("cell name"), "value": string("value")}, "required": ["name", "value"]})),
        ("history", "A cell's value history, oldest first.",
         json!({"type": "object", "properties": {"name": string("cell name"), "limit": {"type": "integer"}}, "required": ["name"]})),
        ("await_cell", "Block until the cell satisfies a predicate (e.g. \"!= ''\", \"== true\", \"> 0.9\") or, with no predicate, until it changes. Returns within timeout_ms (default 45s, at most 50s) with satisfied=false if not yet met; call again until satisfied.",
         json!({"type": "object", "properties": {
             "name": string("cell name"),
             "predicate": string("comparison applied to the cell's value"),
             "timeout_ms": {"type": "integer"}},
             "required": ["name"]})),
        ("ask_cell", "Pose a question on a cell and block until a human answers. Ask only when the options have materially different tradeoffs a human should weigh. Exhaust the cells, the files, and the board's conventions first, and take the conservative option yourself when several would do. Give 2–5 short options; put the tradeoff in an option's description or preview, not its label; mark the one you'd pick as recommended. Returns {resolved:false} on timeout without picking anything; re-call with the same question to keep waiting (it re-attaches, never duplicates).",
         json!({"type": "object", "properties": {
             "name": string("cell name"),
             "question": string("what you need decided"),
             "options": {"type": "array", "items": {"anyOf": [{"type": "string"}, {"type": "object", "properties": {"label": {"type": "string"}, "value": {"type": "string"}, "preview": {"type": "string", "description": "a few lines showing what this choice means: a diff, a sentence, the first lines of a file"}}, "required": ["label"]}]}, "description": "the choices, in order"},
             "allow_freetext": {"type": "boolean"},
             "recommended": {"type": "integer", "description": "zero-based index of the option you propose; shown as the default, never chosen for the human"},
             "timeout_ms": {"type": "integer"},
             "for": string("optional: the id of an agent that may answer instead of the person (\"driver\", or the cell another agent computes). The person can always answer.")},
             "required": ["name", "question"]})),
        ("ask_agent", "Put another agent to work on a question and wait for its answer. Writes @agent('prompt', {inputs}) into the cell you name and runs it, so the handoff sits on the board where the person can read and check it. Returns what trigger_cell does: the answer, or {running: true} past 45s (then await_cell). Agents started by agents stop three deep.",
         json!({"type": "object", "properties": {
             "name": string("an empty cell for the answer (a named cell like review_B2, or an A1 cell)"),
             "agent": string("claude, codex, or a name under [agents] (default claude)"),
             "prompt": string("what to ask; {refs} to other cells are filled in and become inputs"),
             "inputs": {"type": "array", "items": {"type": "string"}, "description": "extra cells or ranges the agent is given as context, e.g. [\"B2\", \"A1:A5\"]"}},
             "required": ["name", "prompt"]})),
        ("resolve_decision", "Answer a pending decision programmatically: an option number (1-based), an option value, or freetext.",
         json!({"type": "object", "properties": {"name": string("cell name"), "choice": {"description": "option number, option value, or freetext"}}, "required": ["name", "choice"]})),
        ("mark_cell", "Flag a cell for human review (mark \"flagged\", with an optional note saying why), or clear a mark (\"none\"). Verifying is reserved for humans.",
         json!({"type": "object", "properties": {"name": string("cell name"), "mark": {"type": "string", "enum": ["flagged", "none"]}, "note": string("why it needs a look")}, "required": ["name", "mark"]})),
        ("open_session", "Put an @ agent cell's live terminal in front of the human: the recalc window starts the cell's interactive session (or reattaches to it) and shows it. For a long or interactive job you want them to watch or type into; a headless run (trigger_cell) needs no session. Does nothing until a window is open on this board.",
         json!({"type": "object", "properties": {"name": string("an @ agent cell")}, "required": ["name"]})),
        ("show_cell", "Select a cell in the human's window so they look at it: the result you want reviewed, the question you left, the row that went red. Does nothing until a window is open on this board.",
         json!({"type": "object", "properties": {"name": string("cell name")}, "required": ["name"]})),
    ];
    json!(tools
        .iter()
        .map(|(name, desc, schema)| json!({
            "name": name,
            "description": desc,
            "inputSchema": schema,
        }))
        .collect::<Vec<_>>())
}
