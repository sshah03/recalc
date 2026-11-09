use super::*;

fn engine() -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path()).unwrap();
    (dir, engine)
}

fn value(e: &Engine, name: &str) -> String {
    e.cells.get(name).unwrap().value.clone()
}

/// A board whose config maps agent names to stub shell templates.
fn engine_with_agents(toml: &str) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".recalc")).unwrap();
    std::fs::write(dir.path().join(".recalc/config.toml"), toml).unwrap();
    let engine = Engine::open(dir.path()).unwrap();
    (dir, engine)
}

#[tokio::test]
async fn a_typed_cell_tells_its_agent_the_form_at_run_time() {
    let (_d, mut e) = engine();
    e.set(
        "F2",
        "@claude('Is it reachable? One of Reachable, Not reachable')",
        Actor::Cli,
    )
    .unwrap();
    e.set("G2", "@claude('Which line?')", Actor::Cli).unwrap();
    e.set(
        "H2",
        "@claude('Is it reachable? Answer with exactly one of: Reachable, Not reachable.')",
        Actor::Cli,
    )
    .unwrap();
    for n in ["F2", "H2"] {
        e.set_answer_type(
            n,
            Some(crate::model::AnswerType {
                kind: crate::model::AnswerKind::Choice,
                options: vec!["Reachable".into(), "Not reachable".into()],
                strict: true,
                allows_not_found: true,
            }),
        )
        .unwrap();
    }
    let task = |e: &Engine, n: &str| {
        let src = e.cells[n].source.clone().unwrap();
        e.agent_task(n, &agent::parse(def_body(&src)).unwrap(), false)
    };
    let typed = task(&e, "F2");
    assert!(
        typed.contains("Answer with exactly one of: Reachable, Not reachable. Print only that"),
        "{typed}"
    );
    assert!(
        !task(&e, "G2").contains("Answer with"),
        "an untyped cell isn't told a form"
    );
    let baked = task(&e, "H2");
    assert_eq!(
        baked.matches("Answer with exactly one of").count(),
        1,
        "not said twice: {baked}"
    );
}

#[test]
fn every_cell_gets_the_same_footer_and_the_table_its_instructions() {
    let (dir, mut e) = engine();
    std::fs::create_dir_all(dir.path().join("contracts")).unwrap();
    std::fs::write(dir.path().join("contracts/a.md"), "text").unwrap();
    e.set("A2", "contracts/a.md", Actor::Cli).unwrap();
    e.set(
        "instructions",
        "Answer from the document; if it is silent, answer Not found.",
        Actor::Cli,
    )
    .unwrap();
    let task =
        |e: &Engine, src: &str| e.agent_task("X", &agent::parse(def_body(src)).unwrap(), false);
    // Document and coding tasks share the footer; table conventions reach
    // cells through the instructions.
    let doc = task(
        &e,
        "@claude('{instructions} Document: {A2}. Who are the parties?')",
    );
    let coding = task(&e, "@claude('fix the flaky test in src/auth.rs')");
    assert_eq!(
        doc.split("\n\n").last(),
        coding.split("\n\n").last(),
        "{doc}\n---\n{coding}"
    );
    assert!(doc.contains("cell X") && doc.contains("cite_cell"), "{doc}");
    assert!(
        doc.contains("answer Not found"),
        "from the instructions: {doc}"
    );
    assert!(!coding.contains("Not found"), "{coding}");
}

#[tokio::test]
async fn agent_call_interpolates_and_runs_only_on_trigger() {
    let (dir, mut e) = engine_with_agents("[agents]\nfake = \"echo ran: {task} | head -4\"\n");
    e.set("failed", "3", Actor::Cli).unwrap();
    e.set("A1", "x", Actor::Cli).unwrap();
    e.set(
        "review",
        "@fake('explain {failed} failures', {A1})",
        Actor::Cli,
    )
    .unwrap();
    // Side-effecting: installing never runs it.
    assert_eq!(e.cell("review").unwrap().state, CellState::Stale);
    assert_eq!(
        e.cell("review").unwrap().deps,
        vec!["failed".to_string(), "A1".to_string()]
    );

    e.trigger("review", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    let v = value(&e, "review");
    assert!(v.contains("ran: explain 3 failures"), "got {v:?}");
    assert!(v.contains("A1 = x"), "context block missing: {v:?}");
    assert!(dir.path().join(".recalc/mcp.json").exists());

    // A dep change marks it stale but must not re-run it.
    e.set("failed", "4", Actor::Cli).unwrap();
    assert_eq!(e.cell("review").unwrap().state, CellState::Stale);
    assert!(value(&e, "review").contains("explain 3 failures"));
}

#[tokio::test]
async fn agent_structured_output_lands_in_named_cells() {
    // The agent writes cells; the call cell holds the summary.
    let (dir, mut e) = engine_with_agents(
        "[agents]\nwriter = \"echo ok > .recalc/cells/verdict && echo summary\"\n",
    );
    e.set("check", "@writer('go')", Actor::Cli).unwrap();
    e.trigger("check", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "check"), "summary");
    assert_eq!(e.cell("check").unwrap().exit_code, Some(0));
    let store = crate::store::Store::open(dir.path()).unwrap();
    assert_eq!(store.get("verdict").unwrap().unwrap().value, "ok");
}

#[tokio::test]
async fn a_citation_can_point_at_another_cell_or_a_web_page() {
    let (_d, mut e) = engine();
    e.set("raw", "auc: 0.791\nn: 1600", Actor::Cli).unwrap();
    e.set("auc", "0.791", Actor::Cli).unwrap();
    let cite = |file: &str, quote: &str| Cite {
        file: file.into(),
        quote: quote.into(),
        location: None,
        by: Actor::Agent("a1".into()),
        at: 1,
        found: None,
    };
    e.set_cite("auc", Some(cite("cell:raw", "auc: 0.791")))
        .unwrap();
    assert_eq!(e.cells["auc"].cite.as_ref().unwrap().found, Some(true));
    e.set_cite("auc", Some(cite("cell:raw", "auc: 0.83")))
        .unwrap();
    assert_eq!(e.cells["auc"].cite.as_ref().unwrap().found, Some(false));
    e.set_cite(
        "auc",
        Some(cite("https://example.com/paper", "AUC of 0.791")),
    )
    .unwrap();
    assert_eq!(
        e.cells["auc"].cite.as_ref().unwrap().found,
        None,
        "a web page isn't checked"
    );
    assert!(e.set_cite("auc", Some(cite("cell:../x", "y"))).is_err());
    assert!(e.set_cite("auc", Some(cite("/etc/hosts", "y"))).is_err());
}

#[tokio::test]
async fn answer_type_specs_round_trip_patterns_and_not_found() {
    let t = parse_answer_type_spec("pattern:^(INV|CRN)-\\d+$!~").unwrap();
    assert_eq!(t.kind, AnswerKind::Pattern);
    assert_eq!(
        t.options,
        ["^(INV|CRN)-\\d+$"],
        "the | stays inside the regex"
    );
    assert!(!t.allows_not_found && !t.strict);
    assert_eq!(parse_answer_type_spec(&answer_type_spec(&t)).unwrap(), t);
    let plain = parse_answer_type_spec("number").unwrap();
    assert!(plain.allows_not_found && plain.strict);
}

#[tokio::test]
async fn a_board_can_give_the_driver_its_own_role() {
    let (dir, e) = engine_with_agents("driver = \"claude\"\n");
    std::fs::create_dir_all(dir.path().join(".recalc/prompts")).unwrap();
    std::fs::write(
        dir.path().join(".recalc/prompts/driver.local.md"),
        "You drive this migration board. Never touch vendor/.",
    )
    .unwrap();
    e.driver_info("sid").unwrap();
    let role = std::fs::read_to_string(dir.path().join(".recalc/prompts/driver.md")).unwrap();
    assert!(role.contains("Never touch vendor/"), "{role}");
}

#[tokio::test]
async fn the_driver_gets_its_own_mcp_config_and_role_file() {
    let (dir, e) = engine_with_agents("driver = \"claude\"\n");
    let (cmd, cwd) = e.driver_info("sid-1").unwrap();
    assert_eq!(cwd, dir.path());
    assert!(
        cmd.starts_with("export RECALC_AGENT='driver'; claude --session-id 'sid-1' --mcp-config "),
        "{cmd}"
    );
    assert!(cmd.contains("mcp-driver.json"), "{cmd}");
    assert!(cmd.contains("--append-system-prompt-file"), "{cmd}");
    let mcp = std::fs::read_to_string(dir.path().join(".recalc/mcp-driver.json")).unwrap();
    assert!(mcp.contains("\"--id\",\"driver\""), "{mcp}");
    let role = std::fs::read_to_string(dir.path().join(".recalc/prompts/driver.md")).unwrap();
    assert!(role.contains("You are the driver"));
    // A custom seat is used as-is; an unknown one says where to fix it.
    let (_d, e) = engine_with_agents("driver = \"mine\"\n[agents]\nmine = \"mine {task}\"\n");
    assert!(e
        .driver_info("s")
        .unwrap()
        .0
        .starts_with("export RECALC_AGENT='driver'; mine 'You are the driver"));
    let (_d, e) = engine_with_agents("driver = \"nope\"\n");
    assert!(e.driver_info("s").unwrap_err().contains("driver ="));
    // And a cell's session id is a property the engine keeps.
    let (_d, mut e) = engine();
    e.set("B2", "@claude('x')", Actor::Cli).unwrap();
    e.set_session_id("B2", Some("abc".into())).unwrap();
    assert_eq!(e.cell("B2").unwrap().session_id.as_deref(), Some("abc"));
    let (cmd, _) = e.resume_info("B2").unwrap();
    // As the cell: its writes carry B2's name, and the Stop hook finds its row.
    assert!(
        cmd.starts_with("export RECALC_AGENT='B2'; claude --resume 'abc'"),
        "{cmd}"
    );
    assert!(e.set_session_id("nope", None).is_err());
}

#[tokio::test]
async fn agent_results_carry_usage_and_are_held_to_the_shape() {
    // Mimics Claude Code's `--output-format json`.
    let json = r#"{"type":"result","is_error":false,"num_turns":2,"result":"yes, per clause 4","session_id":"9f9f-0001","total_cost_usd":0.02,"usage":{"input_tokens":100,"cache_read_input_tokens":900,"output_tokens":7}}"#;
    let (dir, mut e) = engine_with_agents("[agents]\nfake = \"cat result.json\"\n");
    std::fs::write(dir.path().join("result.json"), json).unwrap();
    e.set("B2", "@fake('is it assignable?')", Actor::Cli)
        .unwrap();
    e.trigger("B2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    // No shape: the answer is the result text, and the cost is on the cell.
    assert_eq!(value(&e, "B2"), "yes, per clause 4");
    let usage = e.cell("B2").unwrap().usage.clone().unwrap();
    assert_eq!((usage.input_tokens, usage.output_tokens), (1000, 7));
    assert_eq!(usage.cost_usd, Some(0.02));
    assert_eq!(usage.turns, Some(2));
    // The conversation id lands too, so the run can be picked up again.
    assert_eq!(
        e.cell("B2").unwrap().session_id.as_deref(),
        Some("9f9f-0001")
    );
    // A custom template that runs something recalc doesn't know can't be
    // resumed; the error says how to teach it.
    assert!(e.resume_info("B2").unwrap_err().contains("[resume]"));

    // A strict yes/no misfit errors, keeps the last value, and shows the
    // answer in the console.
    e.set_answer_type(
        "B2",
        Some(crate::model::AnswerType::new(
            crate::model::AnswerKind::YesNo,
            &[],
        )),
    )
    .unwrap();
    e.trigger("B2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    let cell = e.cell("B2").unwrap();
    assert!(
        matches!(&cell.state, CellState::Error(m) if m.contains("doesn't fit its answer type") && m.contains("Yes or No")),
        "{:?}",
        cell.state
    );
    assert_eq!(cell.value, "yes, per clause 4");
    assert!(cell.stderr.contains("answer as printed: yes, per clause 4"));

    // A permissive shape lets it land and only notes the misfit.
    let mut loose = crate::model::AnswerType::new(crate::model::AnswerKind::YesNo, &[]);
    loose.strict = false;
    e.set_answer_type("B2", Some(loose)).unwrap();
    e.trigger("B2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cell("B2").unwrap().state, CellState::Ok);
    assert!(e
        .activity()
        .any(|l| l.text.contains("outside its answer type")));

    // An answer that fits is canonicalized: "Yes." becomes "Yes".
    let (_d2, mut e2) = engine_with_agents("[agents]\nyes = \"echo 'Yes.'\"\n");
    e2.set("C2", "@yes('go')", Actor::Cli).unwrap();
    e2.set_answer_type(
        "C2",
        Some(crate::model::AnswerType::new(
            crate::model::AnswerKind::YesNo,
            &[],
        )),
    )
    .unwrap();
    e2.trigger("C2", Actor::Cli).unwrap();
    e2.drain().await.unwrap();
    assert_eq!(value(&e2, "C2"), "Yes");
    assert!(e2.cell("C2").unwrap().usage.is_none());
    // The shape survives a re-install of the prompt and a human write.
    e2.set("C2", "@yes('go again')", Actor::Cli).unwrap();
    assert!(e2.cell("C2").unwrap().answer_type.is_some());
    e2.set("C2", "maybe", Actor::Human).unwrap();
    assert!(e2.cell("C2").unwrap().answer_type.is_some());
}

#[tokio::test]
async fn second_trigger_refused_while_agent_runs() {
    let (_d, mut e) = engine_with_agents("[agents]\nslow = \"sleep 0.3 && echo done\"\n");
    e.set("x", "@slow('go')", Actor::Cli).unwrap();
    e.trigger("x", Actor::Human).unwrap();
    let err = e.trigger("x", Actor::Human).unwrap_err().to_string();
    assert!(err.contains("already running"), "{err}");
    // The first run was not superseded; its result lands.
    e.drain().await.unwrap();
    assert_eq!(value(&e, "x"), "done");
    // Finished: triggering again is fine.
    e.trigger("x", Actor::Human).unwrap();
    e.drain().await.unwrap();
}

#[tokio::test]
async fn silent_agent_failure_goes_red() {
    let (_d, mut e) = engine_with_agents("[agents]\nbroken = \"echo oops >&2; exit 3\"\n");
    e.set("x", "@broken('go')", Actor::Cli).unwrap();
    e.trigger("x", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    let cell = e.cells.get("x").unwrap();
    assert!(
        matches!(&cell.state, CellState::Error(m) if m.contains("agent exited 3")),
        "got {:?}",
        cell.state
    );
    assert!(cell.stderr.contains("oops"));
    // A command with the same outcome stays Ok; only an agent with no output
    // errors.
    e.set("c", "$ echo oops >&2; exit 3", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cells["c"].state, CellState::Ok);
}

#[tokio::test]
async fn missing_agent_binary_names_the_fix() {
    let (_d, mut e) =
        engine_with_agents("[agents]\nghost = \"no-such-binary-for-recalc {task}\"\n");
    e.set("x", "@ghost('go')", Actor::Cli).unwrap();
    e.trigger("x", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert!(
        matches!(&e.cells["x"].state, CellState::Error(m) if m.contains("command not found") && m.contains("[agents]")),
        "got {:?}",
        e.cells["x"].state
    );
}

#[tokio::test]
async fn agent_errors_are_readable() {
    let (_d, mut e) = engine_with_agents("");
    // Unknown agent: caught at trigger, names the config escape hatch.
    e.set("x", "@nope('hi')", Actor::Cli).unwrap();
    let err = e.trigger("x", Actor::Cli).unwrap_err().to_string();
    assert!(err.contains("unknown agent"), "got {err}");
    // Parse error: surfaced as the cell's error state at install.
    e.set("y", "@claude(no quotes)", Actor::Cli).unwrap();
    assert!(
        matches!(&e.cell("y").unwrap().state, CellState::Error(m) if m.contains("quoted")),
        "got {:?}",
        e.cell("y").unwrap().state
    );
    // Repo refs in a prompt follow the same containment rule as $ cells.
    e.set("z", "@claude('read {file:../secrets}')", Actor::Cli)
        .unwrap();
    assert!(
        matches!(&e.cell("z").unwrap().state, CellState::Error(m) if m.contains("board directory")),
        "got {:?}",
        e.cell("z").unwrap().state
    );
    assert!(e.trigger("z", Actor::Cli).is_err());
}

#[tokio::test]
async fn shipped_verify_template_is_live() {
    // `raw` must carry its watch annotation, and `verified` must catch a claim
    // the tests don't back.
    let (_d, mut e) = engine();
    e.apply_template(
        include_str!("../../examples/verify.recalctemplate"),
        Actor::Cli,
    )
    .unwrap();
    let info = e.console_info("raw").unwrap();
    assert_eq!(info.watch.as_deref(), Some("{src,tests}/**"));
    for check in ["ignored", "skips_added", "baseline", "tests_kept"] {
        assert!(e.cells.contains_key(check), "{check} is on the board");
    }
    e.set("claim_done", "true", Actor::Agent("a1".into()))
        .unwrap();
    // cargo test fails fast in a bare tempdir: status is false while the claim
    // says true.
    e.drain().await.unwrap();
    assert_eq!(value(&e, "verified"), "false");
    assert_eq!(
        value(&e, "status"),
        "false",
        "a formula reads the exit code"
    );
    assert!(e.cells["failed"].series);
}

#[tokio::test]
async fn an_agent_run_carries_its_identity_and_a_command_does_not() {
    // Agent runs carry RECALC_AGENT (see `cli::me`); `$` cells don't.
    let (_d, mut e) = engine_with_agents("[agents]\nclaude = \"echo \\\"[$RECALC_AGENT]\\\"\"\n");
    e.set("B2", "@claude('who')", Actor::Human).unwrap();
    e.set("cmd", "$ echo \"[$RECALC_AGENT]\"", Actor::Human)
        .unwrap();
    e.trigger("B2", Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "B2"), "[B2]");
    assert_eq!(value(&e, "cmd"), "[]");
    let (cmd, _) = e.session_info("B2", "sid").unwrap();
    assert!(cmd.starts_with("export RECALC_AGENT='B2'; "), "{cmd}");
}

#[tokio::test]
async fn shipped_mission_template_reads_claim_vs_truth() {
    // A stub claude that always claims done, against a repo whose
    // tests can't pass (a bare tempdir).
    let (_d, mut e) = engine_with_agents("[agents]\nclaude = \"echo done\"\n");
    e.apply_template(
        include_str!("../../examples/mission.recalctemplate"),
        Actor::Cli,
    )
    .unwrap();
    e.drain().await.unwrap();
    // No claim yet: verified is pending; no session: status is empty;
    // agent cells never ran by themselves.
    assert_eq!(value(&e, "E2"), "pending");
    assert_eq!(value(&e, "C2"), "");
    assert_eq!(e.cells["B2"].state, CellState::Stale);

    // The agent runs and replies done, and the check disagrees.
    e.trigger("B2", Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "B2"), "done");
    assert_eq!(value(&e, "E2"), "false");
    // Its check watches the repo, so the row stays live.
    assert_eq!(
        e.console_info("D2").unwrap().watch.as_deref(),
        Some("src/**")
    );
}

#[test]
fn a_new_conversation_from_another_computer_is_told_what_the_board_kept() {
    let (_d, mut e) = engine();
    e.set("B2", "New York", Actor::Agent("B2".into())).unwrap();
    e.set("B2", "Delaware", Actor::Agent("B2".into())).unwrap();
    e.set_mark(
        "B2",
        Some(crate::model::Mark {
            kind: crate::model::MarkKind::Flagged,
            note: Some("check §12 again".into()),
            by: Actor::Human,
            at: 1,
        }),
    )
    .unwrap();
    let recap = e.recap("B2").unwrap();
    assert!(
        recap.starts_with("You worked on this cell before, on another computer."),
        "{recap}"
    );
    assert!(
        recap.contains("Its answer now, written by you: \"Delaware\""),
        "{recap}"
    );
    assert!(
        recap.contains("A person flagged it: \"check §12 again\""),
        "{recap}"
    );
    assert!(
        recap.contains("earlier answers, newest first: \"New York\""),
        "{recap}"
    );
    assert!(recap.len() < 2000, "a recap stays small: {}", recap.len());
    e.set("empty", "", Actor::Cli).unwrap();
    assert!(e.recap("empty").unwrap().contains("It has no answer yet."));
}

#[test]
fn cells_on_other_sheets_are_ordinary_cells() {
    let (dir, mut e) = engine();
    e.set("Notes!A2", "never edit tests", Actor::Cli).unwrap();
    e.set("notes!B2", "two", Actor::Cli).unwrap();
    assert_eq!(value(&e, "Notes!A2"), "never edit tests");
    assert_eq!(
        value(&e, "Notes!B2"),
        "two",
        "a sheet is found whatever its capitals"
    );
    assert!(
        e.cell("notes!B2").is_none(),
        "and recalc keeps the board's spelling"
    );
    assert_eq!(e.sheets().others, ["Notes"]);
    assert!(dir.path().join(".recalc/sheets/Notes/cells/B2").is_file());
    assert!(
        e.set("Notes!status", "x", Actor::Cli).is_err(),
        "a sheet holds only A1 cells"
    );

    e.rename_sheet("Notes", "Rules").unwrap();
    assert!(e.cell("Notes!A2").is_none());
    assert_eq!(value(&e, "Rules!A2"), "never edit tests");
    e.rename_sheet("Sheet1", "Board").unwrap();
    assert_eq!(e.sheets().first, "Board");
    e.delete_sheet("Rules").unwrap();
    assert!(e.cell("Rules!A2").is_none() && e.cell("Rules!B2").is_none());
    assert!(e.sheets().others.is_empty());
}

#[tokio::test]
async fn references_cross_sheets_and_unqualified_ones_mean_their_own_sheet() {
    let (_d, mut e) = engine();
    e.set("A1", "10", Actor::Cli).unwrap();
    e.set("Notes!A1", "1", Actor::Cli).unwrap();
    e.set("Notes!A2", "2", Actor::Cli).unwrap();
    // Written as a reference or as recalc names it, it's one cell.
    e.set("'My Notes'!A1", "99", Actor::Cli).unwrap();
    e.set("My Notes!A1", "100", Actor::Cli).unwrap();
    assert_eq!(e.sheets().others, ["Notes", "My Notes"]);
    // On the first sheet: braced, bare and quoted references to other sheets.
    e.set(
        "B1",
        "= {Notes!A1} + Notes!A2 + {'My Notes'!A1}",
        Actor::Cli,
    )
    .unwrap();
    assert_eq!(value(&e, "B1"), "103");
    // On Notes: an unqualified A1 means Notes; the first sheet by its name.
    e.set(
        "Notes!B1",
        "= {A1} + {A2} + {Sheet1!A1} + sum({A1:A2})",
        Actor::Cli,
    )
    .unwrap();
    assert_eq!(value(&e, "Notes!B1"), "16");
    let deps = &e.cell("Notes!B1").unwrap().deps;
    assert!(
        deps.contains(&"Notes!A1".to_string()) && deps.contains(&"A1".to_string()),
        "{deps:?}"
    );
    // Changes flow across sheets.
    e.set("Notes!A1", "5", Actor::Cli).unwrap();
    assert_eq!(value(&e, "B1"), "107");
    assert_eq!(value(&e, "Notes!B1"), "24");
    e.set("A1", "20", Actor::Cli).unwrap();
    assert_eq!(value(&e, "Notes!B1"), "34");
    // A named cell is workbook-wide; from a sheet too.
    e.set("rate", "2", Actor::Cli).unwrap();
    e.set("Notes!C1", "= {rate} * {A1}", Actor::Cli).unwrap();
    assert_eq!(value(&e, "Notes!C1"), "10");
    // A sheet that doesn't exist reads as empty, like a missing cell.
    e.set("D1", "= {Nope!A1} == ''", Actor::Cli).unwrap();
    assert_eq!(value(&e, "D1"), "true");
}

#[tokio::test]
async fn inserting_rows_on_a_sheet_moves_that_sheet_and_references_into_it() {
    let (_d, mut e) = engine();
    for (n, v) in [
        ("A1", "1"),
        ("A2", "2"),
        ("Notes!A1", "10"),
        ("Notes!A2", "20"),
        ("Notes!A3", "30"),
    ] {
        e.set(n, v, Actor::Cli).unwrap();
    }
    e.set("B1", "= {Notes!A2} + {A2}", Actor::Cli).unwrap();
    e.set("B2", "= sum({Notes!A1:A3})", Actor::Cli).unwrap();
    e.set("Notes!B2", "= {A2} * 2 + {Sheet1!A2}", Actor::Cli)
        .unwrap();
    e.insert(Some("Notes"), Axis::Rows, 2, 1, Actor::Cli)
        .unwrap();
    // Notes' cells moved; the first sheet's didn't.
    assert_eq!(value(&e, "Notes!A3"), "20");
    assert!(e.cell("Notes!A2").is_none());
    assert_eq!(value(&e, "A2"), "2");
    // References into Notes followed, from either sheet; others stayed.
    assert_eq!(
        e.cell("B1").unwrap().source.as_deref(),
        Some("= {Notes!A3} + {A2}")
    );
    assert_eq!(
        e.cell("B2").unwrap().source.as_deref(),
        Some("= sum({Notes!A1:A4})")
    );
    assert_eq!(
        e.cell("Notes!B3").unwrap().source.as_deref(),
        Some("= {A3} * 2 + {Sheet1!A2}")
    );
    // And the first sheet's insert leaves Notes alone.
    e.insert(None, Axis::Rows, 1, 1, Actor::Cli).unwrap();
    assert_eq!(
        e.cell("B3").unwrap().source.as_deref(),
        Some("= sum({Notes!A1:A4})"),
        "a range into Notes doesn't move"
    );
    assert_eq!(
        e.cell("B2").unwrap().source.as_deref(),
        Some("= {Notes!A3} + {A3}")
    );
    assert_eq!(
        e.cell("Notes!B3").unwrap().source.as_deref(),
        Some("= {A3} * 2 + {Sheet1!A3}")
    );
    // Deleting the row a reference names on Notes breaks only that reference.
    e.delete(Some("Notes"), Axis::Rows, 3, 1, Actor::Cli)
        .unwrap();
    assert_eq!(
        e.cell("B2").unwrap().source.as_deref(),
        Some("= {#REF!} + {A3}")
    );
}

#[tokio::test]
async fn renaming_a_sheet_rewrites_references_and_deleting_one_breaks_them() {
    let (_d, mut e) = engine();
    e.set("Notes!A1", "5", Actor::Cli).unwrap();
    e.set("Notes!A2", "6", Actor::Cli).unwrap();
    e.set(
        "B1",
        "= {Notes!A1} + notes!A2 + sum({Notes!A1:A2})",
        Actor::Cli,
    )
    .unwrap();
    e.set("B2", "$ echo {Notes!A1:raw} {NotesX!A1}", Actor::Cli)
        .unwrap();
    e.set("Notes!B1", "= {A1} * 2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "B1"), "22");
    e.rename_sheet("Notes", "My Notes").unwrap();
    assert_eq!(
        e.cell("B1").unwrap().source.as_deref(),
        Some("= {'My Notes'!A1} + 'My Notes'!A2 + sum({'My Notes'!A1:A2})")
    );
    assert_eq!(
        e.cell("B2").unwrap().source.as_deref(),
        Some("$ echo {'My Notes'!A1:raw} {NotesX!A1}"),
        "only that sheet"
    );
    assert_eq!(value(&e, "B1"), "22", "still reads the same cells");
    assert_eq!(value(&e, "My Notes!B1"), "10");
    e.rename_sheet("Sheet1", "Board").unwrap();
    e.set("My Notes!C1", "= {Board!B1} + 1", Actor::Cli)
        .unwrap();
    assert_eq!(
        value(&e, "My Notes!C1"),
        "23",
        "the first sheet by its new name"
    );
    e.delete_sheet("My Notes").unwrap();
    assert_eq!(
        e.cell("B1").unwrap().source.as_deref(),
        Some("= {#REF!} + #REF! + sum({#REF!})")
    );
    assert!(
        matches!(e.cell("B1").unwrap().state, CellState::Error(_)),
        "a broken reference is an error, not a zero"
    );
}

#[tokio::test]
async fn paste_sort_and_command_rows_stay_on_their_sheet() {
    let (d, mut e) = engine();
    // Paste onto a sheet: targets on it, unqualified refs meaning it.
    e.set("A1", "main", Actor::Cli).unwrap();
    e.paste_sources(
        "Notes!B2",
        "A1",
        &[
            vec![Some("3".into()), Some("= {A1} + 1".into())],
            vec![Some("1".into()), None],
        ],
        Actor::Cli,
    )
    .unwrap();
    assert_eq!(value(&e, "Notes!B2"), "3");
    assert_eq!(
        e.cell("Notes!C2").unwrap().source.as_deref(),
        Some("= {B2} + 1")
    );
    assert_eq!(
        value(&e, "Notes!C2"),
        "4",
        "reads Notes!B2, not the first sheet's"
    );
    e.paste_text("Notes!B4", "2\n9", Actor::Cli).unwrap();
    assert_eq!(value(&e, "Notes!B5"), "9");
    assert!(e.cell("B5").is_none());
    // Sort a sheet's rows: the first sheet doesn't move.
    e.sort_rows(Some("Notes"), 2, 2, 5, false, Actor::Cli)
        .unwrap();
    let col: Vec<String> = (2..=5).map(|r| value(&e, &format!("Notes!B{r}"))).collect();
    assert_eq!(col, ["1", "2", "3", "9"]);
    assert_eq!(value(&e, "A1"), "main");

    // A command on a sheet writes its rows on that sheet.
    std::fs::write(
        d.path().join("prs.json"),
        r#"[{"n":7,"t":"fix login"},{"n":9,"t":"add cache"}]"#,
    )
    .unwrap();
    e.set(
        "PRs!H1",
        "$ cat prs.json # rows: A2 key=n cols=n,t",
        Actor::Human,
    )
    .unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "PRs!A2"), "7");
    assert_eq!(value(&e, "PRs!B3"), "add cache");
    assert!(e.cell("A2").is_none(), "nothing lands on the first sheet");
}

#[tokio::test]
async fn references_reconnect_when_their_sheet_appears_or_is_renamed() {
    let (_d, mut e) = engine();
    // Written before Notes existed, in other capitals.
    e.set("B1", "= {notes!A1} + 1", Actor::Cli).unwrap();
    e.set("Notes!A1", "5", Actor::Cli).unwrap();
    assert_eq!(value(&e, "B1"), "6");
    // Agent writes in other capitals reach the same sheet.
    e.set("NOTES!A1", "7", Actor::Cli).unwrap();
    assert_eq!(value(&e, "B1"), "8");
    // The first sheet by a name it gets later.
    e.set("A1", "40", Actor::Cli).unwrap();
    e.set("Notes!B1", "= {Board!A1} + 0", Actor::Cli).unwrap();
    assert_ne!(value(&e, "Notes!B1"), "40");
    e.rename_sheet("Sheet1", "Board").unwrap();
    assert_eq!(value(&e, "Notes!B1"), "40");
    e.set("A1", "41", Actor::Cli).unwrap();
    assert_eq!(value(&e, "Notes!B1"), "41", "and keeps listening");
}

#[tokio::test]
async fn another_engine_sees_sheets_renamed_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let mut active = Engine::open(dir.path()).unwrap();
    let mut passive = Engine::open(dir.path()).unwrap();
    let mut watch = passive.store.watch().unwrap();
    active.set("Notes!A1", "1", Actor::Cli).unwrap();
    active.set("Notes!A2", "2", Actor::Cli).unwrap();
    let settle = |e: &mut Engine, w: &mut crate::store::Watch, want: &dyn Fn(&Engine) -> bool| {
        for _ in 0..40 {
            e.pump(w).unwrap();
            if want(e) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    };
    assert!(settle(&mut passive, &mut watch, &|e| e
        .cell("Notes!A2")
        .is_some()));
    active.rename_sheet("Notes", "Rules").unwrap();
    assert!(
        settle(&mut passive, &mut watch, &|e| e.cell("Rules!A2").is_some()
            && e.cell("Notes!A1").is_none()),
        "the rename reaches it: {:?}",
        passive.cells.keys().collect::<Vec<_>>()
    );
    assert_eq!(passive.sheets().others, ["Rules"]);
    active.delete_sheet("Rules").unwrap();
    assert!(
        settle(&mut passive, &mut watch, &|e| !e
            .cells
            .keys()
            .any(|n| n.contains('!'))),
        "no ghost cells: {:?}",
        passive.cells.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn retargeting_a_sheet_moves_references_only() {
    let (_d, mut e) = engine();
    e.set("Notes!A1", "5", Actor::Cli).unwrap();
    e.set("Notes!A2", "6", Actor::Cli).unwrap();
    // Text that only looks like a reference stays: a command's words, a
    // prompt, a formula's string, and Unicode around it (once a panic).
    e.set(
        "C1",
        "$ echo İ see Notes!é Notes!A1 for the rule",
        Actor::Cli,
    )
    .unwrap();
    e.set(
        "C2",
        "@claude('Check Notes!A1 against the code')",
        Actor::Cli,
    )
    .unwrap();
    e.set("C3", "= concat('Notes!A1 says ', {Notes!A1})", Actor::Cli)
        .unwrap();
    // Both corners qualified, quoted though it needn't be, bare.
    e.set(
        "D1",
        "= sum({Notes!A1:Notes!A2}) + 'Notes'!A1 + Notes!A2",
        Actor::Cli,
    )
    .unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "D1"), "22");
    e.rename_sheet("Notes", "Bob's").unwrap();
    assert_eq!(
        e.cell("D1").unwrap().source.as_deref(),
        Some("= sum({'Bob''s'!A1:'Bob''s'!A2}) + 'Bob''s'!A1 + 'Bob''s'!A2")
    );
    assert_eq!(
        value(&e, "D1"),
        "22",
        "an apostrophe in a quoted sheet still reads"
    );
    assert_eq!(
        e.cell("C1").unwrap().source.as_deref(),
        Some("$ echo İ see Notes!é Notes!A1 for the rule")
    );
    assert_eq!(
        e.cell("C2").unwrap().source.as_deref(),
        Some("@claude('Check Notes!A1 against the code')")
    );
    assert_eq!(
        e.cell("C3").unwrap().source.as_deref(),
        Some("= concat('Notes!A1 says ', {'Bob''s'!A1})")
    );
    e.delete_sheet("Bob's").unwrap();
    assert_eq!(
        e.cell("D1").unwrap().source.as_deref(),
        Some("= sum({#REF!}) + #REF! + #REF!")
    );
    assert_eq!(
        e.cell("C3").unwrap().source.as_deref(),
        Some("= concat('Notes!A1 says ', {#REF!})")
    );
}

#[tokio::test]
async fn fill_on_a_sheet_shifts_references_as_excel_does() {
    let (_d, mut e) = engine();
    for r in 1..=3 {
        e.set(&format!("Notes!A{r}"), &r.to_string(), Actor::Cli)
            .unwrap();
        e.set(&format!("A{r}"), &(r * 10).to_string(), Actor::Cli)
            .unwrap();
    }
    e.set("Notes!B1", "= {A1} + {Sheet1!A1}", Actor::Cli)
        .unwrap();
    e.fill("Notes!B1", "Notes!B3", Fill::Down, Actor::Cli)
        .unwrap();
    assert_eq!(
        e.cell("Notes!B3").unwrap().source.as_deref(),
        Some("= {A3} + {Sheet1!A3}")
    );
    assert_eq!(value(&e, "Notes!B3"), "33");
    assert!(
        e.fill("Notes!B1", "B3", Fill::Down, Actor::Cli).is_err(),
        "one sheet at a time"
    );
}

#[test]
fn an_agent_prompt_on_a_sheet_reads_its_own_sheet_and_others() {
    let (_d, mut e) = engine();
    e.set(
        "Notes!B2",
        "@claude('{A1} then {Sheet1!C3} then {Rules!A1:A2} and {rate}')",
        Actor::Cli,
    )
    .unwrap();
    let deps = &e.cell("Notes!B2").unwrap().deps;
    for want in ["Notes!A1", "C3", "Rules!A1", "Rules!A2", "rate"] {
        assert!(deps.contains(&want.to_string()), "{want} in {deps:?}");
    }
}

#[tokio::test]
async fn commands_and_prompts_read_other_sheets() {
    let (_d, mut e) = engine();
    e.set("Notes!A1", "hello", Actor::Cli).unwrap();
    e.set("Notes!A2", "world", Actor::Cli).unwrap();
    e.set("v1.2!A1", "dotted", Actor::Cli).unwrap();
    e.set("B1", "$ echo {Notes!A1} {v1.2!A1}", Actor::Cli)
        .unwrap();
    e.set("Notes!B1", "$ echo {A1:A2}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "B1"), "hello dotted");
    assert_eq!(value(&e, "Notes!B1"), "hello world");
}

#[test]
fn a_name_that_differs_only_in_capitals_is_refused_not_merged() {
    let (_d, mut e) = engine();
    e.set("total", "1", Actor::Cli).unwrap();
    let err = e.set("Total", "2", Actor::Cli).unwrap_err().to_string();
    assert!(
        err.contains("Total and total differ only in capital letters"),
        "{err}"
    );
    let err = e
        .set("TOTAL", "= 1 + 1", Actor::Cli)
        .unwrap_err()
        .to_string();
    assert!(err.contains("use total"), "{err}");
    assert_eq!(value(&e, "total"), "1", "the first cell keeps its value");
    assert!(e.cell("Total").is_none());
    // The same spelling still writes, and once it's gone the other spelling is free.
    e.set("total", "3", Actor::Cli).unwrap();
    e.delete_many(&["total".to_string()], Actor::Cli).unwrap();
    e.set("Total", "4", Actor::Cli).unwrap();
    assert_eq!(value(&e, "Total"), "4");
}

#[tokio::test]
async fn a_saved_board_keeps_its_sheets() {
    let (_d, mut e) = engine();
    e.add_sheet("Empty one").unwrap();
    e.set("My Notes!A1", "5", Actor::Cli).unwrap();
    e.set("My Notes!A2", "two\nlines", Actor::Cli).unwrap();
    e.set("B1", "= {'My Notes'!A1} + 1", Actor::Cli).unwrap();
    e.set_series("My Notes!A1", true).unwrap();
    e.set_answer_type(
        "My Notes!A1",
        Some(parse_answer_type_spec("number").unwrap()),
    )
    .unwrap();
    let saved = e.save_template(None);
    assert!(
        saved.contains("@sheet Empty one\n@sheet My Notes\n"),
        "{saved}"
    );
    assert!(saved.contains("@series 'My Notes'!A1"), "{saved}");
    let (_d2, mut f) = engine();
    f.apply_template(&saved, Actor::Cli).unwrap();
    assert_eq!(f.sheets().others, ["Empty one", "My Notes"]);
    assert_eq!(value(&f, "B1"), "6");
    assert_eq!(value(&f, "My Notes!A2"), "two\nlines");
    assert!(f.cell("My Notes!A1").unwrap().series);
    assert!(f.cell("My Notes!A1").unwrap().answer_type.is_some());
    assert_eq!(f.save_template(None), saved);
}

#[tokio::test]
async fn a_saved_board_keeps_its_first_sheets_name() {
    let (_d, mut e) = engine();
    e.rename_sheet("Sheet1", "Board").unwrap();
    e.set("A1", "7", Actor::Cli).unwrap();
    e.set("Notes!A1", "= {Board!A1} * 2", Actor::Cli).unwrap();
    assert_eq!(value(&e, "Notes!A1"), "14");
    let saved = e.save_template(None);
    assert!(saved.starts_with("@first Board\n"), "{saved}");
    // Into an empty board: it takes the name.
    let (_d2, mut f) = engine();
    f.apply_template(&saved, Actor::Cli).unwrap();
    assert_eq!(f.sheets().first, "Board");
    assert_eq!(value(&f, "Notes!A1"), "14");
    // Into a board with cells: references by that name mean its first sheet.
    let (_d3, mut g) = engine();
    g.set("Z9", "keep", Actor::Cli).unwrap();
    g.apply_template(&saved, Actor::Cli).unwrap();
    assert_eq!(g.sheets().first, "Sheet1");
    assert_eq!(
        g.cell("Notes!A1").unwrap().source.as_deref(),
        Some("= {Sheet1!A1} * 2")
    );
    assert_eq!(value(&g, "Notes!A1"), "14");
}

#[tokio::test]
async fn every_value_survives_a_save_and_a_load() {
    let (_d, mut e) = engine();
    let awkward = [
        ("notes", "line one\nline two"),
        ("blank_lines", "a\n\n\nb"),
        ("ends_in_newline", "x\n"),
        ("trailing", "spaced  "),
        ("leading", "  indented"),
        ("hash", "# not a comment"),
        ("at", "@series looks like a directive"),
        ("eq", "a = b = c"),
        ("heredoc_lookalike", "x <<END"),
        ("has_end", "first\nEND\nlast"),
        ("quotes", "say \"hi\" and 'bye'"),
        ("unicode", "naïve café — 日本 🎉"),
        ("tabs", "a\tb\t"),
    ];
    for (name, v) in awkward {
        e.set(name, v, Actor::Cli).unwrap();
    }
    e.set("multi_cmd", "$ printf 'a\\n'\necho b", Actor::Cli)
        .unwrap();
    let saved = e.save_template(None);
    assert!(!saved.contains("skipped"), "{saved}");
    let (_d2, mut f) = engine();
    f.apply_template(&saved, Actor::Cli).unwrap();
    for (name, v) in awkward {
        let (a, b) = (e.cell(name).unwrap(), f.cell(name).unwrap());
        assert_eq!(
            (&b.source, &b.kind),
            (&a.source, &a.kind),
            "{name} in\n{saved}"
        );
        if a.kind == CellKind::Literal {
            assert_eq!(b.value, v, "{name} in\n{saved}");
        }
    }
    assert_eq!(
        f.cell("multi_cmd").and_then(|c| c.source.as_deref()),
        Some("$ printf 'a\\n'\necho b")
    );
    assert_eq!(
        f.save_template(None),
        saved,
        "a second save is the same file"
    );
    // A file edited on Windows still reads; the cost is a value's own \r\n, read back as \n.
    f.apply_template("crlf = one\r\nnext <<END\r\na\r\nb\r\nEND\r\n", Actor::Cli)
        .unwrap();
    assert_eq!(value(&f, "crlf"), "one");
    assert_eq!(value(&f, "next"), "a\nb");
    let err = f
        .apply_template("x <<END\nnever closed\n", Actor::Cli)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("line 1") && err.contains("closing END"),
        "{err}"
    );
}

#[test]
fn apply_template_sets_cells_comments_and_series() {
    let (_d, mut e) = engine();
    let text = "# a note\nA1 = 3\nB1 = = {A1} * 2\nempty =\n@series A1\n";
    e.apply_template(text, Actor::Cli).unwrap();
    assert_eq!(value(&e, "B1"), "6");
    assert_eq!(value(&e, "empty"), "");
    assert!(e.cells.get("A1").unwrap().series);
    let err = e.apply_template("garbage line", Actor::Cli).unwrap_err();
    assert!(err.to_string().contains("line 1"));

    // save_template round-trips: defs, empties, series markers.
    let saved = e.save_template(Some("a note"));
    assert_eq!(
        saved,
        "# a note\nA1 = 3\nB1 = = {A1} * 2\nempty =\n@series A1\n"
    );

    // @answer_type lines type cells (the old @shape spelling too); they
    // group by spec on the way out.
    e.apply_template(
        "C2 =\nC3 =\nD2 =\n@answer_type C2 C3 = choice:Low|Medium|Not stated here\n@answer_type D2 = number~\n",
        Actor::Cli,
    )
    .unwrap();
    let c2 = e.cells.get("C2").unwrap().answer_type.clone().unwrap();
    assert_eq!(c2.kind, crate::model::AnswerKind::Choice);
    assert_eq!(c2.options, ["Low", "Medium", "Not stated here"]);
    assert!(c2.strict);
    assert!(
        !e.cells
            .get("D2")
            .unwrap()
            .answer_type
            .as_ref()
            .unwrap()
            .strict
    );
    let saved = e.save_template(None);
    assert!(
        saved.contains("@answer_type C2 C3 = choice:Low|Medium|Not stated here\n"),
        "{saved}"
    );
    assert!(saved.contains("@answer_type D2 = number~\n"), "{saved}");
    let err = e
        .apply_template("@answer_type C2 = chart\n", Actor::Cli)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("line 1") && err.contains("answer type must be"),
        "{err}"
    );
}

#[test]
fn mirror_updates_on_change() {
    let (_d, mut e) = engine();
    e.set("A1", "1", Actor::Cli).unwrap();
    e.set("B1", "= {A1}", Actor::Cli).unwrap();
    assert_eq!(value(&e, "B1"), "1");
    e.set("A1", "2", Actor::Cli).unwrap();
    assert_eq!(value(&e, "B1"), "2");
    // Persisted, not just in-memory.
    assert_eq!(e.store.get("B1").unwrap().unwrap().value, "2");
}

#[test]
fn chain_propagates_in_order() {
    let (_d, mut e) = engine();
    e.set("a", "1", Actor::Cli).unwrap();
    e.set("b", "= {a} + 1", Actor::Cli).unwrap();
    e.set("c", "= {b} * 2", Actor::Cli).unwrap();
    assert_eq!(value(&e, "c"), "4");
    e.set("a", "10", Actor::Cli).unwrap();
    assert_eq!(value(&e, "b"), "11");
    assert_eq!(value(&e, "c"), "22");
}

#[test]
fn cycle_rejected_with_path() {
    let (_d, mut e) = engine();
    e.set("a", "= {b}", Actor::Cli).unwrap();
    let err = e.set("b", "= {a}", Actor::Cli).unwrap_err().to_string();
    assert!(err.contains("cycle"), "{err}");
    assert!(
        err.contains("b -> a -> b") || err.contains("b -> a"),
        "{err}"
    );
    // The offending source was not applied.
    assert_eq!(e.store.get("b").unwrap().unwrap().source, None);
    assert!(matches!(
        e.cells.get("b").unwrap().state,
        CellState::Error(_)
    ));
}

#[test]
fn self_cycle_rejected() {
    let (_d, mut e) = engine();
    assert!(e.set("x", "= {x} + 1", Actor::Cli).is_err());
}

#[test]
fn named_range_members_are_live_deps() {
    let (_d, mut e) = engine();
    e.set("A1", "1", Actor::Cli).unwrap();
    e.set("A2", "2", Actor::Cli).unwrap();
    e.set("r", "A1:A2", Actor::Cli).unwrap();
    e.set("s", "= sum({r})", Actor::Cli).unwrap();
    assert_eq!(value(&e, "s"), "3");
    // Eval discovered the members: a member change recomputes.
    e.set("A2", "10", Actor::Cli).unwrap();
    assert_eq!(value(&e, "s"), "11");
    // Retargeting the range re-discovers: new members are live too.
    e.set("A3", "100", Actor::Cli).unwrap();
    e.set("r", "A1:A3", Actor::Cli).unwrap();
    assert_eq!(value(&e, "s"), "111");
    e.set("A3", "200", Actor::Cli).unwrap();
    assert_eq!(value(&e, "s"), "211");
    assert!(e.cells["s"].deps.iter().any(|d| d == "A3"));
}

#[test]
fn named_range_cycle_is_rejected_not_looped() {
    let (_d, mut e) = engine();
    e.set("r", "B1:B1", Actor::Cli).unwrap();
    e.set("s", "= sum({r})", Actor::Cli).unwrap();
    // s's discovered deps include B1, so B1 = {s} is a cycle.
    let err = e.set("B1", "= {s}", Actor::Cli).unwrap_err().to_string();
    assert!(err.contains("cycle"), "{err}");
}

#[test]
fn lazy_branches_are_not_deps() {
    let (_d, mut e) = engine();
    e.set("c", "true", Actor::Cli).unwrap();
    e.set("a", "1", Actor::Cli).unwrap();
    e.set("b", "2", Actor::Cli).unwrap();
    e.set("pick", "= if({c}, {a}, {b})", Actor::Cli).unwrap();
    assert_eq!(value(&e, "pick"), "1");
    // Only the taken branch is a dependency.
    assert_eq!(e.cells["pick"].deps, ["c", "a"]);
    e.set("c", "false", Actor::Cli).unwrap();
    assert_eq!(value(&e, "pick"), "2");
    assert_eq!(e.cells["pick"].deps, ["c", "b"]);
}

#[test]
fn csv_parsing_handles_quotes_and_newlines() {
    assert_eq!(
        parse_csv("a,b\n1,2\n"),
        [["a", "b"], ["1", "2"]],
        "final newline is a terminator"
    );
    assert!(parse_csv("").is_empty());
    assert_eq!(
        parse_csv("\"x, y\",\"say \"\"hi\"\"\"\r\n\"two\nlines\",z"),
        [
            vec!["x, y".to_string(), "say \"hi\"".to_string()],
            vec!["two\nlines".to_string(), "z".to_string()]
        ]
    );
    assert_eq!(parse_csv("solo"), [["solo"]]);
}

#[tokio::test]
async fn import_lands_literals_and_recomputes() {
    let (_d, mut e) = engine();
    e.set("total", "= sum({B2:B3})", Actor::Cli).unwrap();
    let changed = e
        .import_csv("name,n\nalpha,3\nbeta,4\n", "A1", Actor::Cli)
        .unwrap();
    assert_eq!(changed.len(), 6);
    assert_eq!(value(&e, "A1"), "name");
    assert_eq!(value(&e, "B3"), "4");
    assert_eq!(value(&e, "total"), "7");
    // A field that looks like a def imports as text.
    e.import_csv("= 1 + 1,$ rm -rf x\n", "D1", Actor::Cli)
        .unwrap();
    assert_eq!(value(&e, "D1"), "= 1 + 1");
    assert_eq!(e.cells["D1"].kind, CellKind::Literal);
    assert_eq!(e.cells["E1"].kind, CellKind::Literal);
    // A bad anchor is refused; an empty file touches nothing.
    assert!(e.import_csv("x", "total", Actor::Cli).is_err());
    assert!(e.import_csv("", "A1", Actor::Cli).unwrap().is_empty());
    assert_eq!(value(&e, "A1"), "name");

    // A def whose source equals the field is still flattened to text.
    e.set("F1", "= 1 + 1", Actor::Cli).unwrap();
    e.import_csv("= 1 + 1\n", "F1", Actor::Cli).unwrap();
    assert_eq!(e.cells["F1"].kind, CellKind::Literal);
    assert_eq!(value(&e, "F1"), "= 1 + 1");
}

#[test]
fn range_member_change_recomputes_aggregate() {
    let (_d, mut e) = engine();
    e.set("A1", "1", Actor::Cli).unwrap();
    e.set("A2", "2", Actor::Cli).unwrap();
    e.set("total", "= sum({A1:A3})", Actor::Cli).unwrap();
    assert_eq!(value(&e, "total"), "3");
    e.set("A3", "10", Actor::Cli).unwrap();
    assert_eq!(value(&e, "total"), "13");
}

#[test]
fn missing_dep_is_empty_string() {
    let (_d, mut e) = engine();
    e.set("check", "= {ghost} == ''", Actor::Cli).unwrap();
    assert_eq!(value(&e, "check"), "true");
}

#[test]
fn expr_error_keeps_last_value() {
    let (_d, mut e) = engine();
    e.set("a", "5", Actor::Cli).unwrap();
    e.set("b", "= {a} * 2", Actor::Cli).unwrap();
    assert_eq!(value(&e, "b"), "10");
    e.set("a", "word", Actor::Cli).unwrap();
    assert!(matches!(
        e.cells.get("b").unwrap().state,
        CellState::Error(_)
    ));
    assert_eq!(value(&e, "b"), "10");
}

#[test]
fn malformed_expr_is_error_state() {
    let (_d, mut e) = engine();
    e.set("bad", "= 1 +", Actor::Cli).unwrap();
    assert!(matches!(
        e.cells.get("bad").unwrap().state,
        CellState::Error(_)
    ));
}

#[tokio::test]
async fn command_reflects_its_dependency() {
    let (_d, mut e) = engine();
    e.set("A1", "hello", Actor::Cli).unwrap();
    e.set("echo", "$ echo {A1}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "echo"), "hello");
    assert_eq!(e.cells.get("echo").unwrap().deps, ["A1"]);
    e.set("A1", "world", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "echo"), "world");
}

#[tokio::test]
async fn quoted_interpolation_is_injection_inert() {
    let (_d, mut e) = engine();
    let evil = "x'; echo INJECTED; 'y";
    e.set("inj", evil, Actor::Cli).unwrap();
    e.set("out", "$ echo {inj}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "out"), evil);
}

#[tokio::test]
async fn file_form_resolves_to_backing_path() {
    let (_d, mut e) = engine();
    e.set("raw", "line one\nline two", Actor::Cli).unwrap();
    e.set("path", "$ echo {raw:file}", Actor::Cli).unwrap();
    e.set("lines", "$ wc -l < {raw:file}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert!(value(&e, "path").ends_with(".recalc/cells/raw"));
    // Value files end with one newline, so wc sees both lines.
    assert_eq!(value(&e, "lines"), "2");
}

#[tokio::test]
async fn exit_code_captured_not_an_error() {
    let (_d, mut e) = engine();
    e.set("t", "$ echo out; exit 3", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    let cell = e.cells.get("t").unwrap();
    assert_eq!(cell.state, CellState::Ok);
    assert_eq!(cell.exit_code, Some(3));
    assert_eq!(cell.value, "out");
    // Downstream can test {t.code}.
    e.set("check", "$ echo code={t.code}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "check"), "code=3");
}

#[tokio::test]
async fn cwd_defaults_to_launch_dir_and_annotation_overrides() {
    let (d, mut e) = engine();
    e.set("here", "$ pwd", Actor::Cli).unwrap();
    e.set("root", "$ pwd # cwd: /", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    let real = d.path().canonicalize().unwrap();
    assert_eq!(value(&e, "here"), real.to_string_lossy());
    assert_eq!(value(&e, "root"), "/");
}

#[tokio::test]
async fn superseded_run_never_overwrites_newer() {
    let (_d, mut e) = engine();
    e.set("A1", "1", Actor::Cli).unwrap();
    e.set("slowfast", "$ sleep 0.{A1} && echo {A1}", Actor::Cli)
        .unwrap();
    // Gen-2 (A1=3, sleep 0.3) must win though gen-1 finishes first.
    e.set("A1", "3", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "slowfast"), "3");
}

#[tokio::test]
async fn stale_run_does_not_overwrite_direct_write() {
    let (_d, mut e) = engine();
    e.set("x", "$ sleep 0.2 && echo old", Actor::Cli).unwrap();
    // Overwrite with a literal while the run is still in flight.
    e.set("x", "new", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "x"), "new");
    assert_eq!(e.cells.get("x").unwrap().kind, CellKind::Literal);
}

#[tokio::test]
async fn command_output_flows_into_expressions() {
    let (_d, mut e) = engine();
    e.set("failed", "$ echo 0", Actor::Cli).unwrap();
    e.set("status", "= {failed} == 0", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "status"), "true");
}

#[tokio::test]
async fn action_runs_only_on_trigger() {
    let (_d, mut e) = engine();
    e.set("msg", "one", Actor::Cli).unwrap();
    e.set("act", "! echo {msg}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "act"), "");
    assert_eq!(e.cells.get("act").unwrap().state, CellState::Stale);

    e.trigger("act", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "act"), "one");
    assert_eq!(e.cells.get("act").unwrap().state, CellState::Ok);

    // Upstream change re-stales it but never fires it.
    e.set("msg", "two", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "act"), "one");
    assert_eq!(e.cells.get("act").unwrap().state, CellState::Stale);
}

#[tokio::test]
async fn reload_never_fires_actions() {
    let (d, mut e) = engine();
    e.set("act", "! echo ran", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    drop(e);
    let mut e = Engine::open(d.path()).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "act"), "");
    assert_eq!(e.cells.get("act").unwrap().state, CellState::Stale);
}

#[tokio::test]
async fn a_run_goes_with_the_engine_that_started_it() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("finished");
    {
        let mut e = Engine::open(dir.path()).unwrap();
        let cmd = format!("! sleep 2 && touch {}", marker.display());
        e.set("slow", &cmd, Actor::Cli).unwrap();
        e.trigger("slow", Actor::Cli).unwrap();
        assert_eq!(e.cells["slow"].state, CellState::Computing);
    }
    let e = Engine::open(dir.path()).unwrap();
    assert_eq!(e.cells["slow"].state, CellState::Stale);
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert!(!marker.exists(), "the run was killed, not orphaned");
}

#[tokio::test]
async fn a_run_whose_owner_died_is_not_running_forever() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    e.set("act", "! sleep 30", Actor::Cli).unwrap();
    e.trigger("act", Actor::Cli).unwrap();
    e.groups.kill_all();
    // The process crashed: no shutdown, and its pid is gone. (The
    // release stands in for the dead process's claim lapsing.)
    e.store.release_engine();
    std::mem::forget(e);
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    std::fs::write(dir.path().join(".recalc/running/act"), dead.to_string()).unwrap();

    let mut e = Engine::open(dir.path()).unwrap();
    assert_eq!(e.cells["act"].state, CellState::Stale);
    e.set("act", "! echo again", Actor::Cli).unwrap();
    e.trigger("act", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "act"), "again");
}

#[tokio::test]
async fn a_suggestion_can_be_turned_down() {
    use crate::model::MarkKind;
    let (_d, mut e) = engine();
    e.set("x", "mine", Actor::Human).unwrap();
    e.set_mark(
        "x",
        Some(Mark {
            kind: MarkKind::Verified,
            note: None,
            by: Actor::Human,
            at: 1,
        }),
    )
    .unwrap();
    e.set("x", "theirs", Actor::Agent("a".into())).unwrap();
    e.set("x", "other", Actor::Agent("b".into())).unwrap();
    assert_eq!(e.cells["x"].pending_suggestions.len(), 2);
    e.dismiss_suggestions("x", Some("theirs"), Actor::Human)
        .unwrap();
    assert_eq!(e.cells["x"].pending_suggestions.len(), 1);
    assert_eq!(e.cells["x"].pending_suggestions[0].value, "other");
    e.dismiss_suggestions("x", None, Actor::Human).unwrap();
    assert!(e.cells["x"].pending_suggestions.is_empty());
    assert_eq!(value(&e, "x"), "mine");
}

#[tokio::test]
async fn dangerous_command_is_quarantined() {
    let (_d, mut e) = engine();
    e.set("bad", "$ rm -rf x", Actor::Cli).unwrap();
    e.set("log", "$ make build > out.log", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cells.get("bad").unwrap().state, CellState::Quarantined);
    assert_eq!(e.cells.get("log").unwrap().state, CellState::Quarantined);
    assert_eq!(value(&e, "bad"), "");
}

#[tokio::test]
async fn read_only_idioms_not_quarantined() {
    let (_d, mut e) = engine();
    e.set("t", "$ echo done 2>&1", Actor::Cli).unwrap();
    e.set("q", "$ echo noisy > /dev/null", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "t"), "done");
    assert_eq!(e.cells.get("q").unwrap().state, CellState::Ok);
}

#[tokio::test]
async fn human_trigger_confirms_quarantine() {
    let (_d, mut e) = engine();
    e.set("n", "1", Actor::Cli).unwrap();
    // A harmless command the scan flags because it redirects to a path.
    e.set("w", "$ echo {n} > /dev/stdout", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cells.get("w").unwrap().state, CellState::Quarantined);

    e.trigger("w", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "w"), "1");

    // Confirmed once: dep changes auto-run it again.
    e.set("n", "2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "w"), "2");

    // Editing the source resets the confirmation.
    e.set("w", "$ echo {n} > /dev/stdout # edited", Actor::Cli)
        .unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cells.get("w").unwrap().state, CellState::Quarantined);
}

#[tokio::test]
async fn agent_trigger_does_not_confirm() {
    let (_d, mut e) = engine();
    e.set("n", "1", Actor::Cli).unwrap();
    e.set("w", "$ echo {n} > /dev/stdout", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    e.trigger("w", Actor::Agent("a1".into())).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "w"), "1"); // explicit run happens
    e.set("n", "2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    // …but auto-runs stay quarantined until a human confirms.
    assert_eq!(e.cells.get("w").unwrap().state, CellState::Quarantined);
    assert_eq!(value(&e, "w"), "1");
}

/// Write a value file the way `echo x > .recalc/cells/name` would:
/// no meta, no engine.
fn raw_write(dir: &tempfile::TempDir, name: &str, value: &str) {
    std::fs::write(
        dir.path().join(".recalc/cells").join(name),
        format!("{value}\n"),
    )
    .unwrap();
}

#[tokio::test]
async fn external_literal_write_dirties_dependents() {
    let (d, mut e) = engine();
    e.set("A1", "1", Actor::Cli).unwrap();
    e.set("B1", "= {A1} + 1", Actor::Cli).unwrap();
    raw_write(&d, "A1", "5");
    e.reconcile("A1").unwrap();
    assert_eq!(value(&e, "A1"), "5");
    assert_eq!(value(&e, "B1"), "6");
}

#[tokio::test]
async fn external_write_to_agent_owned_cell_becomes_suggestion() {
    let (d, mut e) = engine();
    e.set("status", "= 1 == 1", Actor::Agent("a1".into()))
        .unwrap();
    assert_eq!(value(&e, "status"), "true");
    raw_write(&d, "status", "nope");
    e.reconcile("status").unwrap();
    let cell = e.cells.get("status").unwrap();
    // The value is NOT silently overridden…
    assert_eq!(cell.value, "true");
    assert_eq!(e.store.get("status").unwrap().unwrap().value, "true");
    // …the write is held as a pending suggestion for the owner.
    assert_eq!(cell.pending_suggestions.len(), 1);
    assert_eq!(cell.pending_suggestions[0].value, "nope");
    assert_eq!(cell.pending_suggestions[0].from, Actor::Human);

    // Re-delivered event (our own restore write) doesn't duplicate it.
    e.reconcile("status").unwrap();
    assert_eq!(e.cells.get("status").unwrap().pending_suggestions.len(), 1);

    // The owner adopting the value clears the queue.
    e.set("status", "nope", Actor::Agent("a1".into())).unwrap();
    assert!(e
        .cells
        .get("status")
        .unwrap()
        .pending_suggestions
        .is_empty());
}

#[tokio::test]
async fn external_write_to_unowned_computed_cell_pins() {
    let (d, mut e) = engine();
    e.set("total", "= 1 + 1", Actor::Cli).unwrap();
    e.set("double", "= {total} * 2", Actor::Cli).unwrap();
    raw_write(&d, "total", "10");
    e.reconcile("total").unwrap();
    let cell = e.cells.get("total").unwrap();
    assert_eq!(cell.kind, CellKind::Literal);
    assert_eq!(cell.value, "10");
    assert_eq!(cell.source, None);
    assert_eq!(value(&e, "double"), "20");
}

#[tokio::test]
async fn external_def_write_installs() {
    let (d, mut e) = engine();
    e.set("A1", "2", Actor::Cli).unwrap();
    std::fs::write(d.path().join(".recalc/defs/twice"), "= {A1} * 2\n").unwrap();
    e.reconcile("twice").unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "twice"), "4");
}

#[tokio::test]
async fn external_def_edit_to_action_does_not_run() {
    let (d, mut e) = engine();
    e.set("job", "$ echo ran", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "job"), "ran");
    // vim defs/job: the `$` becomes a `!`. Stale meta still says Command;
    // the reinstall must not auto-run the action.
    std::fs::write(d.path().join(".recalc/defs/job"), "! echo fired\n").unwrap();
    e.reconcile("job").unwrap();
    e.drain().await.unwrap();
    let cell = e.cells.get("job").unwrap();
    assert_eq!(cell.kind, CellKind::Action);
    assert_eq!(cell.state, CellState::Stale);
    assert_eq!(cell.value, "ran");
}

#[tokio::test]
async fn external_pin_from_another_process_adopts() {
    let (d, mut e) = engine();
    e.set("total", "= 1 + 1", Actor::Cli).unwrap();
    e.set("double", "= {total} * 2", Actor::Cli).unwrap();
    // `recalc set total 10` in another shell: value written, def removed.
    let other = Store::open(d.path()).unwrap();
    other.set_value("total", "10", Actor::Cli).unwrap();
    e.reconcile("total").unwrap();
    assert_eq!(e.cells.get("total").unwrap().kind, CellKind::Literal);
    assert_eq!(value(&e, "total"), "10");
    assert_eq!(value(&e, "double"), "20");
}

#[tokio::test]
async fn a_write_from_an_agents_process_is_logged_under_the_agent() {
    let (d, mut e) = engine();
    e.set("A1", "1", Actor::Human).unwrap();
    let other = Store::open(d.path()).unwrap();
    other
        .set_value("A1", "2", Actor::Agent("claude-code".into()))
        .unwrap();
    other
        .set_value("B1", "new", Actor::Agent("claude-code".into()))
        .unwrap();
    e.reconcile("A1").unwrap();
    e.reconcile("B1").unwrap();
    let by_agent: Vec<_> = e
        .activity()
        .filter(|a| a.by.as_deref() == Some("agent:claude-code"))
        .map(|a| a.text.as_str())
        .collect();
    assert_eq!(
        by_agent,
        ["agent:claude-code set A1", "agent:claude-code set B1"]
    );
    // Its own writes, seen again on disk, aren't logged twice.
    e.reconcile("A1").unwrap();
    assert_eq!(
        e.activity().filter(|a| a.text.ends_with("set A1")).count(),
        2
    );
}

#[tokio::test]
async fn external_cycle_def_marks_error_without_looping() {
    let (d, mut e) = engine();
    e.set("a", "= {b}", Actor::Cli).unwrap();
    std::fs::write(d.path().join(".recalc/defs/b"), "= {a}\n").unwrap();
    e.reconcile("b").unwrap();
    assert!(matches!(
        e.cells.get("b").unwrap().state,
        CellState::Error(_)
    ));
}

#[tokio::test]
async fn watch_loop_reacts_to_external_writes() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    e.set("A1", "1", Actor::Cli).unwrap();
    e.set("B1", "= {A1} + 1", Actor::Cli).unwrap();
    let mut watch = e.store.watch().unwrap();
    let handle = tokio::spawn(async move {
        // Pump-style serving, the way the app does.
        loop {
            e.pump(&mut watch).unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    });

    // Simulate `echo 41 > .recalc/cells/A1` from another shell.
    std::fs::write(dir.path().join(".recalc/cells/A1"), "41\n").unwrap();

    // Poll through a second store handle, as another process would.
    let store = Store::open(dir.path()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let b1 = store.get("B1").unwrap().unwrap();
        if b1.value == "42" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "B1 stuck at {:?}",
            b1.value
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    handle.abort();
}

#[tokio::test]
async fn edit_computed_opens_system_decision() {
    let (_d, mut e) = engine();
    e.set("status", "= 1 == 0", Actor::Agent("a1".into()))
        .unwrap();
    assert_eq!(value(&e, "status"), "false");
    e.edit_computed("status", "true".into()).unwrap();
    let cell = e.cells.get("status").unwrap();
    assert_eq!(cell.state, CellState::Awaiting);
    let d = cell.decision.as_ref().unwrap();
    assert_eq!(d.asker, Actor::System);
    assert_eq!(d.held.as_deref(), Some("true"));
    // Agent attached → Suggest is option 1 and the default.
    assert_eq!(d.options[0].value, "suggest");
    // The typed value is held, not applied.
    assert_eq!(cell.value, "false");
    // A dep-driven recompute won't clobber the pending decision.
    e.recompute(vec!["status".to_string()]).unwrap();
    assert_eq!(e.cells.get("status").unwrap().state, CellState::Awaiting);
}

#[tokio::test]
async fn resolve_suggest_records_suggestion() {
    let (_d, mut e) = engine();
    e.set("status", "= 1 == 0", Actor::Agent("a1".into()))
        .unwrap();
    e.edit_computed("status", "true".into()).unwrap();
    e.resolve_decision("status", Choice::Index(0), Actor::Human)
        .unwrap();
    let cell = e.cells.get("status").unwrap();
    assert_eq!(cell.state, CellState::Ok);
    assert!(cell.decision.is_none());
    assert_eq!(cell.value, "false"); // unchanged
    assert_eq!(cell.pending_suggestions[0].value, "true");
}

#[tokio::test]
async fn resolve_freetext_suggests_with_note() {
    let (_d, mut e) = engine();
    e.set("status", "= 1 == 0", Actor::Agent("a1".into()))
        .unwrap();
    e.edit_computed("status", "true".into()).unwrap();
    e.resolve_decision(
        "status",
        Choice::Text("tests are flaky, ignore".into()),
        Actor::Human,
    )
    .unwrap();
    let s = &e.cells.get("status").unwrap().pending_suggestions[0];
    assert_eq!(s.note.as_deref(), Some("tests are flaky, ignore"));
}

#[tokio::test]
async fn resolve_pin_and_override() {
    let (_d, mut e) = engine();
    // No agent owner → no Suggest option; Pin is the default.
    e.set("total", "= 1 + 1", Actor::Cli).unwrap();
    e.edit_computed("total", "10".into()).unwrap();
    let d = e.cells.get("total").unwrap().decision.clone().unwrap();
    assert_eq!(d.options[0].value, "pin");
    e.resolve_decision("total", Choice::Index(0), Actor::Human)
        .unwrap();
    let cell = e.cells.get("total").unwrap();
    assert_eq!(cell.value, "10");
    assert_eq!(cell.kind, CellKind::Literal);
    assert_eq!(cell.source, None);

    // Override once: value now, source reclaims on next recompute.
    e.set("a", "1", Actor::Cli).unwrap();
    e.set("b", "= {a} + 1", Actor::Cli).unwrap();
    e.set("c", "= {b} * 10", Actor::Cli).unwrap();
    e.edit_computed("b", "7".into()).unwrap();
    let idx = e
        .cells
        .get("b")
        .unwrap()
        .decision
        .as_ref()
        .unwrap()
        .options
        .iter()
        .position(|o| o.value == "override")
        .unwrap();
    e.resolve_decision("b", Choice::Index(idx), Actor::Human)
        .unwrap();
    assert_eq!(value(&e, "b"), "7");
    assert_eq!(value(&e, "c"), "70");
    assert_eq!(
        e.cells.get("b").unwrap().source.as_deref(),
        Some("= {a} + 1")
    );
    e.set("a", "5", Actor::Cli).unwrap(); // source reclaims
    assert_eq!(value(&e, "b"), "6");
    assert_eq!(value(&e, "c"), "60");
}

#[tokio::test]
async fn resolve_cancel_reverts() {
    let (_d, mut e) = engine();
    e.set("total", "= 1 + 1", Actor::Cli).unwrap();
    e.edit_computed("total", "10".into()).unwrap();
    let idx = e
        .cells
        .get("total")
        .unwrap()
        .decision
        .as_ref()
        .unwrap()
        .options
        .iter()
        .position(|o| o.value == "cancel")
        .unwrap();
    e.resolve_decision("total", Choice::Index(idx), Actor::Human)
        .unwrap();
    let cell = e.cells.get("total").unwrap();
    assert_eq!(cell.value, "2");
    assert_eq!(cell.state, CellState::Ok);
    assert!(cell.decision.is_none());
    assert!(cell.pending_suggestions.is_empty());
}

#[tokio::test]
async fn agent_decision_resolves_to_value() {
    let (_d, mut e) = engine();
    let prov = Provenance::now(Actor::Agent("a2".into()));
    e.pose_decision(
        "migrations",
        crate::model::Decision {
            question: "delete old migrations?".into(),
            options: vec![
                crate::model::DecisionOption {
                    label: "yes".into(),
                    value: "yes".into(),
                    preview: None,
                },
                crate::model::DecisionOption {
                    label: "no".into(),
                    value: "no".into(),
                    preview: None,
                },
            ],
            allow_freetext: true,
            recommended: Some(1),
            asker: Actor::Agent("a2".into()),
            posed_at: prov.at,
            held: None,
            from_run: false,
            answerable_by: None,
        },
    )
    .unwrap();
    assert_eq!(
        e.cells.get("migrations").unwrap().state,
        CellState::Awaiting
    );
    e.resolve_decision("migrations", Choice::Index(0), Actor::Human)
        .unwrap();
    let cell = e.cells.get("migrations").unwrap();
    assert_eq!(cell.value, "yes");
    assert_eq!(cell.state, CellState::Ok);
    assert!(cell.decision.is_none());
    assert_eq!(cell.provenance.by, Actor::Human);
}

#[tokio::test]
async fn the_drivers_runs_are_counted_from_claude_codes_own_record() {
    let (dir, mut e) = engine();
    let state = dir.path().join("claude-state.json");
    e.claude_state = Some(state.clone());
    let board = dir.path().canonicalize().unwrap();
    // Claude Code rewrites its state file as each session ends.
    let ended = |session: &str, started: u64, cost: f64| {
        let json = serde_json::json!({"projects": {
            board.to_string_lossy(): {
                "lastSessionId": session, "lastStartTime": started, "lastCost": cost,
                "lastTotalInputTokens": 10, "lastTotalOutputTokens": 90,
                "lastTotalCacheCreationInputTokens": 100, "lastTotalCacheReadInputTokens": 800,
                "lastModelUsage": {"claude-opus-5-5": {"costUSD": cost}}
            },
            "/somewhere/else": {"lastSessionId": "d-1", "lastStartTime": 1, "lastCost": 99.0}
        }});
        std::fs::write(&state, json.to_string()).unwrap();
    };
    let check = |e: &mut Engine| {
        e.driver_checked = std::time::Instant::now() - std::time::Duration::from_secs(10);
        e.claude_state_stamp = None;
        e.collect_driver_usage();
    };
    e.set_driver_session("d-1").unwrap();
    check(&mut e);
    assert_eq!(e.driver_usage(), (0.0, 0), "nothing has ended yet");
    ended("d-1", 100, 1.25);
    check(&mut e);
    assert_eq!(e.driver_usage(), (1.25, 1000));
    assert!(e.take_news(), "the window is told");
    // Seen again, it isn't counted twice; resumed later, the new run adds.
    check(&mut e);
    assert_eq!(e.driver_usage().0, 1.25);
    ended("d-1", 200, 0.5);
    check(&mut e);
    assert_eq!(e.driver_usage().0, 1.75);
    // A person's own claude session in the folder isn't the Driver's.
    ended("someone-else", 300, 7.0);
    check(&mut e);
    assert_eq!(e.driver_usage().0, 1.75);
    // Start new: the old conversation still counts when it ends after.
    e.set_driver_session("d-2").unwrap();
    check(&mut e);
    ended("d-1", 400, 0.25);
    check(&mut e);
    assert_eq!(e.driver_usage().0, 2.0);
    assert!(e.activity().any(|a| a.text.contains("$0.25 at API prices")));
}

#[test]
fn a_sheet_duplicates_as_a_fresh_workpaper() {
    let (_d, mut e) = engine();
    e.add_sheet("Harbor").unwrap();
    e.add_sheet("Summary").unwrap();
    e.set("rate", "0.5", Actor::Cli).unwrap();
    e.set("Harbor!B1", "clients/harbor", Actor::Cli).unwrap();
    e.set("Harbor!A2", "= 1 + 1", Actor::Cli).unwrap();
    e.set("Harbor!B2", "= {A2} * {rate}", Actor::Cli).unwrap();
    e.set("Harbor!C2", "@claude('Read {B1}')", Actor::Cli)
        .unwrap();
    e.set("Harbor!C2", "an answer", Actor::Agent("Harbor!C2".into()))
        .unwrap();
    e.set_answer_type(
        "Harbor!D2",
        Some(crate::model::AnswerType::new(
            crate::model::AnswerKind::Number,
            &[],
        )),
    )
    .unwrap();
    e.duplicate_sheet("harbor", "Pinecrest", Actor::Human)
        .unwrap();
    // Right after the one it copies.
    assert_eq!(e.sheets().others, ["Harbor", "Pinecrest", "Summary"]);
    // Formulas mean the copy's own cells; the named cell stays the board's.
    e.set("Pinecrest!A2", "10", Actor::Cli).unwrap();
    assert_eq!(value(&e, "Pinecrest!B2"), "5");
    assert_eq!(value(&e, "Harbor!B2"), "1");
    assert!(e.cell("Pinecrest!rate").is_none());
    // The prompt comes along; the answer doesn't.
    let c2 = e.cell("Pinecrest!C2").unwrap();
    assert_eq!(c2.source.as_deref(), Some("@claude('Read {B1}')"));
    assert_eq!(c2.value, "");
    assert!(e.cell("Pinecrest!D2").unwrap().answer_type.is_some());
    assert!(
        e.duplicate_sheet("Harbor", "pinecrest", Actor::Human)
            .is_err(),
        "the name is taken"
    );
    assert!(e.duplicate_sheet("Nope", "X", Actor::Human).is_err());
}

#[tokio::test]
async fn a_formula_over_answers_not_in_yet_waits_instead_of_erroring() {
    let (_d, mut e) = engine_with_agents("[agents]\nseven = \"echo 7\"\n");
    e.set("D2", "@seven('the first figure')", Actor::Cli)
        .unwrap();
    e.set("E2", "@seven('the second figure')", Actor::Cli)
        .unwrap();
    e.set("F2", "= {D2} - {E2}", Actor::Cli).unwrap();
    e.set("G2", "= if(abs({F2}) < 1, 'ties', 'mismatch')", Actor::Cli)
        .unwrap();
    e.set("J2", "= {D2} + {E2}", Actor::Cli).unwrap();
    e.set("K2", "= sum({D2:E2})", Actor::Cli).unwrap();
    // Nothing has run: the formulas wait, + included, and nothing is an error.
    for name in ["F2", "G2", "J2"] {
        assert_eq!(e.cell(name).unwrap().state, CellState::Stale, "{name}");
    }
    // A function over a range adds what's there, as a progress count does.
    assert_eq!(value(&e, "K2"), "0");
    e.set("H2", "= {A9} * 2", Actor::Cli).unwrap();
    e.set("A9", "abc", Actor::Cli).unwrap();
    assert!(
        matches!(e.cell("H2").unwrap().state, CellState::Error(_)),
        "a value that isn't a number is still an error"
    );
    e.trigger("D2", Actor::Cli).unwrap();
    e.trigger("E2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "F2"), "0");
    assert_eq!(value(&e, "G2"), "ties");
    assert_eq!(value(&e, "J2"), "14");
    assert_eq!(value(&e, "K2"), "14");
    // An input whose run failed with no answer is the formula's error too.
    let (_d, mut e) = engine_with_agents("[agents]\nbroken = \"exit 3\"\n");
    e.set("D2", "@broken('a figure')", Actor::Cli).unwrap();
    e.set("F2", "= {D2} * 2", Actor::Cli).unwrap();
    e.trigger("D2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert!(
        matches!(&e.cell("F2").unwrap().state, CellState::Error(m) if m.contains("its run failed")),
        "{:?}",
        e.cell("F2").unwrap().state
    );
}

#[test]
fn a_flagged_answer_is_rerun_knowing_why() {
    let (_d, mut e) = engine();
    e.set("A2", "contracts/volcon.md", Actor::Cli).unwrap();
    e.set(
        "B2",
        "@claude('Which law governs? Document: {A2}')",
        Actor::Cli,
    )
    .unwrap();
    // Its agent answered.
    e.set("B2", "Singapore", Actor::Agent("B2".into())).unwrap();
    assert_eq!(value(&e, "B2"), "Singapore");
    e.set_mark(
        "B2",
        Some(Mark {
            kind: crate::model::MarkKind::Flagged,
            note: Some("Section 18 says Texas".into()),
            by: Actor::Human,
            at: 0,
        }),
    )
    .unwrap();
    let source = e.cell("B2").unwrap().source.clone().unwrap();
    let call = agent::parse(def_body(&source)).unwrap();
    let task = e.agent_task("B2", &call, false);
    assert!(
        task.contains("A reviewer flagged the previous answer, \"Singapore\", saying: \"Section 18 says Texas\". Take the note as the reviewer's guidance on what the question means, look again, and answer."),
        "{task}"
    );
    // Unflagged, the next run is asked plainly.
    e.set_mark("B2", None).unwrap();
    assert!(!e.agent_task("B2", &call, false).contains("flagged"));
}

#[test]
fn a_question_from_a_run_whose_process_died_is_taken_down() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut e = Engine::open(dir.path()).unwrap();
        e.set("B2", "@claude('Read the file')", Actor::Cli).unwrap();
        // The run asks to read a file, then its process is killed.
        let mut cell = e.cell("B2").unwrap().clone();
        cell.state = CellState::Computing;
        cell.decision = Some(Decision {
            question: "Allow Read?".into(),
            options: vec![],
            allow_freetext: true,
            recommended: None,
            asker: Actor::Agent("B2's agent".into()),
            posed_at: 0,
            held: None,
            from_run: true,
            answerable_by: None,
        });
        e.store.update(&cell).unwrap();
        e.shutdown();
    }
    std::fs::create_dir_all(dir.path().join(".recalc/running")).unwrap();
    std::fs::write(dir.path().join(".recalc/running/B2"), "999999").unwrap();
    let e = Engine::open(dir.path()).unwrap();
    let cell = e.cell("B2").unwrap();
    assert_eq!(cell.state, CellState::Stale);
    assert!(cell.decision.is_none(), "no one is left to read the answer");
}

#[test]
fn an_agent_may_read_the_folder_of_a_document_outside_the_board() {
    let (dir, mut e) = engine();
    // A document grid over a folder picked elsewhere: absolute paths.
    let elsewhere = tempfile::tempdir().unwrap();
    let doc = elsewhere.path().join("acme-supply.md");
    std::fs::write(&doc, "Governed by the laws of Delaware.").unwrap();
    std::fs::create_dir_all(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join("docs/inside.md"), "x").unwrap();
    e.set("A2", &doc.to_string_lossy(), Actor::Cli).unwrap();
    e.set("A3", "docs/inside.md", Actor::Cli).unwrap();
    e.set("A4", "Delaware, New York", Actor::Cli).unwrap();
    for (cell, input) in [("B2", "A2"), ("B3", "A3"), ("B4", "A4")] {
        e.set(
            cell,
            &format!("@claude('Governing law? Document: {{{input}}}')"),
            Actor::Cli,
        )
        .unwrap();
    }
    let folder = elsewhere.path().canonicalize().unwrap();
    let flag = format!("--add-dir '{}'", folder.display());
    let b2 = e
        .agent_command("B2", e.cell("B2").unwrap().source.as_deref().unwrap())
        .unwrap();
    assert!(b2.contains(&flag), "its document's folder: {b2}");
    // Inside the board, or not a file at all: nothing added.
    for cell in ["B3", "B4"] {
        let cmd = e
            .agent_command(cell, e.cell(cell).unwrap().source.as_deref().unwrap())
            .unwrap();
        assert!(!cmd.contains("--add-dir"), "{cell}: {cmd}");
    }
    // Resuming the conversation keeps the access.
    e.set_session_id("B2", Some("s-1".into())).unwrap();
    assert!(e.resume_info("B2").unwrap().0.contains(&flag));
}

#[tokio::test]
async fn a_cell_read_halfway_through_a_write_never_crashes_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    e.set("A1", "1", Actor::Cli).unwrap();
    e.set("C1", "= {A1} + 1", Actor::Cli).unwrap();
    e.set("C2", "$ echo {A1}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    // Another engine turning C1 and C2 into values deletes their formulas
    // before it rewrites their meta. An engine reading in between saw a
    // formula cell with no formula, and panicked recomputing it.
    std::fs::remove_file(dir.path().join(".recalc/defs/C1")).unwrap();
    std::fs::remove_file(dir.path().join(".recalc/defs/C2")).unwrap();
    let mut other = Engine::open(dir.path()).unwrap();
    other.set("A1", "5", Actor::Cli).unwrap();
    other.drain().await.unwrap();
    // A cell with no formula on disk is a value, whatever its meta says.
    for name in ["C1", "C2"] {
        let cell = other.cell(name).unwrap();
        assert_eq!(
            (cell.kind, cell.source.as_deref()),
            (CellKind::Literal, None),
            "{name}"
        );
    }
    // And a formula written to it later computes as usual.
    other.set("C1", "= {A1} * 3", Actor::Cli).unwrap();
    assert_eq!(value(&other, "C1"), "15");
}

#[tokio::test]
async fn passive_engine_never_runs_commands_reactively() {
    let dir = tempfile::tempdir().unwrap();
    let _active = Engine::open(dir.path()).unwrap(); // holds the election
    let mut passive = Engine::open(dir.path()).unwrap();
    assert!(!passive.active);
    passive.set("n", "1", Actor::Cli).unwrap();
    passive.set("echo", "$ echo {n}", Actor::Cli).unwrap();
    passive.drain().await.unwrap();
    // No local run: the active engine owns reactive execution.
    assert_eq!(value(&passive, "echo"), "");
    assert_eq!(passive.cells.get("echo").unwrap().state, CellState::Stale);
    // …but an explicit trigger always runs, wherever it's issued.
    passive.trigger("echo", Actor::Cli).unwrap();
    passive.drain().await.unwrap();
    assert_eq!(value(&passive, "echo"), "1");
}

#[test]
fn election_passes_when_owner_drops() {
    let dir = tempfile::tempdir().unwrap();
    let first = Engine::open(dir.path()).unwrap();
    assert!(first.active);
    drop(first);
    let second = Engine::open(dir.path()).unwrap();
    assert!(second.active);
}

#[tokio::test]
async fn passive_engine_takes_over_when_active_exits() {
    let dir = tempfile::tempdir().unwrap();
    let active = Engine::open(dir.path()).unwrap();
    let mut passive = Engine::open(dir.path()).unwrap();
    let mut watch = passive.store.watch().unwrap();
    passive.set("n", "1", Actor::Cli).unwrap();
    passive.set("echo", "$ echo {n}", Actor::Cli).unwrap();
    assert!(!passive.active);
    drop(active);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    passive.pump(&mut watch).unwrap();
    assert!(passive.active);
    // The takeover runs what passivity left stale.
    passive.drain().await.unwrap();
    assert_eq!(value(&passive, "echo"), "1");
}

#[tokio::test]
async fn repeated_recompute_builds_history() {
    let (_d, mut e) = engine();
    for n in ["3", "2", "0"] {
        e.set("n", n, Actor::Cli).unwrap();
        e.set("failed", &format!("$ echo {n}"), Actor::Cli).unwrap();
        e.drain().await.unwrap();
    }
    let history = e.history("failed", 10).unwrap();
    let values: Vec<&str> = history.iter().map(|(_, v)| v.as_str()).collect();
    assert_eq!(values, ["3", "2", "0"]);
    // Unchanged recompute appends nothing.
    e.set("failed", "$ echo 0 # rerun", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.history("failed", 10).unwrap().len(), 3);
    // Series tracking persists across a literal overwrite.
    e.set_series("failed", true).unwrap();
    e.set("failed", "1", Actor::Cli).unwrap();
    assert!(e.cells.get("failed").unwrap().series);
}

#[test]
fn multiline_history_round_trips() {
    let (_d, mut e) = engine();
    e.set("log", "a\tb\nc\\d", Actor::Cli).unwrap();
    e.set("log", "second", Actor::Cli).unwrap();
    let history = e.history("log", 10).unwrap();
    assert_eq!(history[0].1, "a\tb\nc\\d");
    assert_eq!(history[1].1, "second");
}

#[test]
fn shift_refs_rewrites_relatively() {
    // Plain refs, ranges, and the suffix forms all shift.
    assert_eq!(shift_refs("= {A1} + {B2}", 0, 1), "= {A2} + {B3}");
    assert_eq!(shift_refs("= sum({A1:A10})", 1, 0), "= sum({B1:B10})");
    assert_eq!(
        shift_refs("$ wc -l {A2:file} {A2.code}", 0, 2),
        "$ wc -l {A4:file} {A4.code}"
    );
    // Named cells are absolute; shell syntax and non-refs pass through.
    assert_eq!(shift_refs("= {total} + {A1}", 0, 3), "= {total} + {A4}");
    assert_eq!(
        shift_refs("$ echo ${HOME} {raw:file} 'awk {print $1}'", 0, 1),
        "$ echo ${HOME} {raw:file} 'awk {print $1}'"
    );
    assert_eq!(shift_refs("echo {unclosed", 0, 1), "echo {unclosed");
}

#[tokio::test]
async fn fill_survives_a_cycle_in_one_target() {
    let (_d, mut e) = engine();
    e.set("D2", "= {B2}", Actor::Cli).unwrap();
    e.set("B1", "= {D1}", Actor::Cli).unwrap();
    // B2 would become `= {D2}`, a cycle through D2. That cell shows the
    // error; B3 still fills, and only real changes reach the undo pairs.
    let changed = e.fill("B1", "B3", Fill::Down, Actor::Human).unwrap();
    assert!(matches!(e.cells["B2"].state, CellState::Error(_)));
    assert_eq!(e.cells["B3"].source.as_deref(), Some("= {D3}"));
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].0, "B3");
}

#[tokio::test]
async fn fill_down_shifts_and_computes() {
    let (_d, mut e) = engine();
    for (name, v) in [("A1", "1"), ("A2", "2"), ("A3", "3")] {
        e.set(name, v, Actor::Cli).unwrap();
    }
    e.set("B1", "= {A1} * 2", Actor::Cli).unwrap();
    let changed = e.fill("B1", "B3", Fill::Down, Actor::Human).unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!(
        e.cells["B2"].source.as_deref(),
        Some("= {A2} * 2"),
        "ref should shift with the row"
    );
    assert_eq!(value(&e, "B2"), "4");
    assert_eq!(value(&e, "B3"), "6");
    // The returned pairs are undo material: (target, previous content).
    assert_eq!(changed[0], ("B2".to_string(), String::new()));
}

#[tokio::test]
async fn fill_right_shifts_ranges_and_named_refs_stay() {
    let (_d, mut e) = engine();
    e.set("A1", "1", Actor::Cli).unwrap();
    e.set("B1", "10", Actor::Cli).unwrap();
    e.set("total", "5", Actor::Cli).unwrap();
    e.set("A3", "= sum({A1:A2}) + {total}", Actor::Cli).unwrap();
    e.fill("A3", "B3", Fill::Right, Actor::Human).unwrap();
    assert_eq!(
        e.cells["B3"].source.as_deref(),
        Some("= sum({B1:B2}) + {total}")
    );
    assert_eq!(value(&e, "B3"), "15");
}

#[tokio::test]
async fn fill_single_row_copies_from_above() {
    let (_d, mut e) = engine();
    e.set("B1", "= 1 + 1", Actor::Cli).unwrap();
    e.fill("B2", "B2", Fill::Down, Actor::Human).unwrap();
    assert_eq!(e.cells["B2"].source.as_deref(), Some("= 1 + 1"));
    // Row 1 has nothing above it.
    assert!(e.fill("C1", "C1", Fill::Down, Actor::Human).is_err());
    // A lane with no source cell is skipped, not an error.
    let changed = e.fill("Z1", "Z3", Fill::Down, Actor::Human).unwrap();
    assert!(changed.is_empty());
}

#[tokio::test]
async fn fill_copies_literals_verbatim_and_actions_stay_stale() {
    let (_d, mut e) = engine();
    // A literal that merely looks like a ref is not rewritten.
    e.set("A1", "see {A1}", Actor::Cli).unwrap();
    e.fill("A1", "A2", Fill::Down, Actor::Human).unwrap();
    assert_eq!(value(&e, "A2"), "see {A1}");

    e.set("B1", "! echo {A1}", Actor::Cli).unwrap();
    e.fill("B1", "B2", Fill::Down, Actor::Human).unwrap();
    let b2 = &e.cells["B2"];
    assert_eq!(b2.source.as_deref(), Some("! echo {A2}"));
    assert_eq!(b2.state, CellState::Stale, "filled actions must not run");
    e.drain().await.unwrap();
    assert_eq!(value(&e, "B2"), "");
}

#[test]
fn fill_rejects_non_a1_corners() {
    let (_d, mut e) = engine();
    assert!(e.fill("total", "B3", Fill::Down, Actor::Human).is_err());
}

#[tokio::test]
async fn file_ref_resolves_to_absolute_path_and_registers_no_cell_dep() {
    let (d, mut e) = engine();
    std::fs::write(d.path().join("notes.txt"), "hello\n").unwrap();
    e.set("n", "$ cat {file:notes.txt}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "n"), "hello");
    assert!(
        e.cells["n"].deps.is_empty(),
        "a repo file is not a cell dep"
    );
    let info = e.console_info("n").unwrap();
    assert!(
        info.command.contains("notes.txt") && info.command.starts_with("cat '/"),
        "expected an absolute quoted path, got {:?}",
        info.command
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn file_ref_reruns_when_the_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "one\n").unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set("n", "$ cat {file:notes.txt}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "n"), "one");

    e.pump(&mut watch).unwrap(); // repo watcher exists before the edit
    std::fs::write(dir.path().join("notes.txt"), "two\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "n") != "two" {
        assert!(
            std::time::Instant::now() < deadline,
            "stuck at {:?}",
            value(&e, "n")
        );
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn glob_ref_lists_matches_and_sees_new_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.txt"), "").unwrap();
    std::fs::write(dir.path().join("src/skip.log"), "").unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set("files", "$ echo {glob:src/*.txt}", Actor::Cli)
        .unwrap();
    e.drain().await.unwrap();
    let v = value(&e, "files");
    assert!(v.contains("src/a.txt") && !v.contains("skip.log"), "{v}");

    // A new matching file re-runs the cell and appears in the list.
    e.pump(&mut watch).unwrap();
    std::fs::write(dir.path().join("src/b.txt"), "").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !value(&e, "files").contains("src/b.txt") {
        assert!(std::time::Instant::now() < deadline, "never saw b.txt");
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // Sorted: a before b.
    let v = value(&e, "files");
    assert!(v.find("a.txt").unwrap() < v.find("b.txt").unwrap());
}

#[tokio::test]
async fn glob_ref_matches_a_literal_path_and_quotes_spaces() {
    let (d, mut e) = engine();
    std::fs::write(d.path().join("my notes.txt"), "spaced\n").unwrap();
    e.set("lit", "$ cat {glob:my notes.txt}", Actor::Cli)
        .unwrap();
    e.set("f", "$ cat {file:my notes.txt}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "lit"), "spaced");
    assert_eq!(value(&e, "f"), "spaced");
}

#[tokio::test]
async fn repo_refs_outside_the_board_are_errors() {
    let (_d, mut e) = engine();
    for src in [
        "$ cat {file:/etc/passwd}",
        "$ cat {file:../secrets}",
        "$ ls {glob:/**}",
    ] {
        e.set("x", src, Actor::Cli).unwrap();
        assert!(
            matches!(&e.cells["x"].state, CellState::Error(m) if m.contains("board directory")),
            "{src} should be an install error, got {:?}",
            e.cells["x"].state
        );
        assert!(e.trigger("x", Actor::Cli).is_err());
        // The error must be on disk too, or the stale meta `set_def` wrote comes
        // back over it in a mirroring process.
        assert!(
            matches!(&e.store.get("x").unwrap().unwrap().state, CellState::Error(m) if m.contains("board directory")),
            "{src}: the install error is persisted"
        );
        // And the recompute wave after the install must not lose it.
        e.drain().await.unwrap();
        assert!(
            matches!(&e.cells["x"].state, CellState::Error(m) if m.contains("board directory")),
            "{src} should still show its error after the wave, got {:?}",
            e.cells["x"].state
        );
    }
}

/// A run a passive engine queued and then left behind is adopted by the
/// active engine, not marked stale.
#[tokio::test]
async fn a_run_queued_by_a_departed_process_is_adopted_by_the_active_engine() {
    let tools = tempfile::tempdir().unwrap();
    let claude = tools.path().join("claude");
    std::fs::write(
        &claude,
        "#!/bin/sh\nread -r line\nsleep 0.3\necho '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ran\",\"session_id\":\"s\",\"total_cost_usd\":0.01,\"num_turns\":1}'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (d, mut active) = engine_with_agents(&format!(
        "max_agents = 1\n[paths]\nclaude = \"{}\"\n",
        claude.display()
    ));
    assert!(active.active);
    let mut passive = Engine::open(d.path()).unwrap();
    assert!(!passive.active, "the second engine on the board is passive");
    passive.set("x", "@claude('one')", Actor::Cli).unwrap();
    passive.set("y", "@claude('two')", Actor::Cli).unwrap();
    // The window has seen the cells before they run, as it would.
    active.reconcile("x").unwrap();
    active.reconcile("y").unwrap();
    passive.trigger("x", Actor::Cli).unwrap();
    passive.trigger("y", Actor::Cli).unwrap();
    assert_eq!(
        passive.cells["y"].state,
        CellState::Queued,
        "one slot: y waits"
    );
    passive.shutdown();
    assert_eq!(
        passive.cells["y"].state,
        CellState::Queued,
        "shutdown leaves a queued run queued"
    );
    // Its owner is gone: the marker names a pid that no longer exists.
    std::fs::write(d.path().join(".recalc/running/y"), "999999").unwrap();
    active.reconcile("x").unwrap();
    active.reconcile("y").unwrap();
    assert_eq!(
        active.cells["y"].state,
        CellState::Queued,
        "the window mirrors the queued state"
    );
    active.reset_orphaned_runs().unwrap();
    assert!(
        active.agent_queue.iter().any(|n| n == "y")
            || active.cells["y"].state == CellState::Computing,
        "y is queued here now: {:?}",
        active.cells["y"].state
    );
    active.drain().await.unwrap();
    assert_eq!(value(&active, "y"), "ran");
    assert_eq!(active.cells["y"].state, CellState::Ok);
}

#[tokio::test]
async fn action_file_ref_resolves_but_never_watches() {
    let (d, mut e) = engine();
    std::fs::write(d.path().join("in.txt"), "payload\n").unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set("act", "! cat {file:in.txt}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cells["act"].state, CellState::Stale);
    e.pump(&mut watch).unwrap();
    assert!(e.repo_watch.is_none(), "only $ cells register repo watches");
    e.trigger("act", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "act"), "payload");
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_git_watch_reruns_but_broad_globs_ignore_git() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".git/HEAD"), "ref: main\n").unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set("head", "$ cat .git/HEAD # watch: .git/HEAD", Actor::Cli)
        .unwrap();
    e.set("all", "$ echo scan # watch: **", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "head"), "ref: main");
    e.pump(&mut watch).unwrap();
    let all_ran_at = e.cells["all"].provenance.at;

    std::fs::write(dir.path().join(".git/HEAD"), "ref: feature\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "head") != "ref: feature" {
        assert!(
            std::time::Instant::now() < deadline,
            "git watch never fired"
        );
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // The `**` cell saw none of it: .git only reaches explicit globs.
    assert_eq!(e.cells["all"].provenance.at, all_ran_at);
}

#[test]
fn annotation_splitting() {
    let (cmd, ann) = split_annotations("cargo test 2>&1 # watch: src/**/*.rs # cwd: sub");
    assert_eq!(cmd, "cargo test 2>&1");
    assert_eq!(ann.watch, Some("src/**/*.rs"));
    assert_eq!(ann.cwd, Some("sub"));
    assert_eq!(ann.every, None);

    // Unknown keys are ordinary shell comments and stay put.
    let (cmd, ann) = split_annotations("echo a # note: keep # every: 250");
    assert_eq!(cmd, "echo a # note: keep");
    assert_eq!(ann.every, Some("250"));

    // A colon-free comment ends the scan.
    let (cmd, ann) = split_annotations("echo x # plain comment");
    assert_eq!(cmd, "echo x # plain comment");
    assert_eq!(ann, Annotations::default());

    assert_eq!(
        split_annotations("grep -c ERROR log").0,
        "grep -c ERROR log"
    );
}

#[test]
fn glob_roots() {
    assert_eq!(glob_root("src/**/*.rs"), Path::new("src"));
    assert_eq!(glob_root("*.rs"), Path::new(""));
    assert_eq!(glob_root("Cargo.toml"), Path::new("Cargo.toml"));
}

#[tokio::test]
async fn console_shows_annotations_and_strips_them_from_the_command() {
    let (_d, mut e) = engine();
    e.set("w", "$ pwd # watch: src/**/*.rs # every: 500", Actor::Cli)
        .unwrap();
    e.drain().await.unwrap();
    let info = e.console_info("w").unwrap();
    assert_eq!(info.command, "pwd");
    assert_eq!(info.watch.as_deref(), Some("src/**/*.rs"));
    assert_eq!(info.every, Some(500));
}

#[tokio::test]
async fn annotation_refs_are_not_deps() {
    let (_d, mut e) = engine();
    e.set("A1", "x", Actor::Cli).unwrap();
    e.set("w", "$ echo {A1} # cwd: {B1}", Actor::Cli).unwrap();
    assert_eq!(e.cells.get("w").unwrap().deps, ["A1"]);
    e.drain().await.unwrap();
}

#[test]
fn watch_on_action_is_error_and_wont_trigger() {
    let (_d, mut e) = engine();
    e.set("a", "! echo hi # watch: src/**", Actor::Cli).unwrap();
    assert!(
        matches!(&e.cells.get("a").unwrap().state, CellState::Error(m) if m.contains("only $ cells")),
        "got {:?}",
        e.cells.get("a").unwrap().state
    );
    let err = e.trigger("a", Actor::Cli).unwrap_err().to_string();
    assert!(err.contains("only $ cells"), "{err}");

    // A dep change must keep the error, not flip it to "needs run".
    e.set("dep", "1", Actor::Cli).unwrap();
    e.set("b", "! echo {dep} # every: 100", Actor::Cli).unwrap();
    e.set("dep", "2", Actor::Cli).unwrap();
    assert!(matches!(
        e.cells.get("b").unwrap().state,
        CellState::Error(_)
    ));
}

#[test]
fn malformed_watch_annotations_are_errors() {
    let (_d, mut e) = engine();
    e.set("t", "$ echo hi # every: soon", Actor::Cli).unwrap();
    assert!(matches!(&e.cells.get("t").unwrap().state, CellState::Error(m) if m.contains("every")),);
    e.set("g", "$ echo hi # watch: src/[", Actor::Cli).unwrap();
    assert!(matches!(&e.cells.get("g").unwrap().state, CellState::Error(m) if m.contains("watch")),);
    e.set("abs", "$ echo hi # watch: /var/log/*.log", Actor::Cli)
        .unwrap();
    assert!(
        matches!(&e.cells.get("abs").unwrap().state, CellState::Error(m) if m.contains("relative")),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_annotation_reruns_on_matching_file_change() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/data.txt"), "one\n").unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set("data", "$ cat src/data.txt # watch: src/*.txt", Actor::Cli)
        .unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "data"), "one");

    // One pump so the repo watcher exists before the file changes.
    e.pump(&mut watch).unwrap();
    std::fs::write(dir.path().join("src/data.txt"), "two\n").unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "data") != "two" {
        assert!(
            std::time::Instant::now() < deadline,
            "stuck at {:?}",
            value(&e, "data")
        );
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// A watched file can be one that appears later, in a hidden folder: the S-corp
/// demo's Form map waits for `.venv/ready`, which its Setup action writes.
#[tokio::test(flavor = "multi_thread")]
async fn watch_reruns_when_a_file_appears_in_a_hidden_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set(
        "ready",
        "$ test -f .venv/ready && echo yes || echo no # watch: .venv/ready",
        Actor::Cli,
    )
    .unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "ready"), "no");

    e.pump(&mut watch).unwrap();
    std::fs::create_dir(dir.path().join(".venv")).unwrap();
    std::fs::write(dir.path().join(".venv/ready"), "").unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "ready") != "yes" {
        assert!(std::time::Instant::now() < deadline, "never re-ran");
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_board_reopened_on_changed_code_reruns_its_watched_checks() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/data.txt"), "one\n").unwrap();
    std::fs::write(dir.path().join("src/other.txt"), "x\n").unwrap();
    {
        let mut e = Engine::open(dir.path()).unwrap();
        e.set(
            "data",
            "$ cat src/data.txt # watch: src/data.txt",
            Actor::Cli,
        )
        .unwrap();
        e.set("calm", "$ date +%s%N # watch: src/other.txt", Actor::Cli)
            .unwrap();
        e.drain().await.unwrap();
        assert_eq!(value(&e, "data"), "one");
    }
    // Nothing watching: the code changes (an editor, a git pull).
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(dir.path().join("src/data.txt"), "two\n").unwrap();

    let mut e = Engine::open(dir.path()).unwrap();
    let calm = value(&e, "calm");
    let mut watch = e.store.watch().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "data") != "two" {
        assert!(
            std::time::Instant::now() < deadline,
            "reopened showing {:?}",
            value(&e, "data")
        );
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // A check whose files didn't change isn't run again.
    assert_eq!(value(&e, "calm"), calm);
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_ignores_non_matching_paths() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set("n", "$ echo ran # watch: src/*.txt", Actor::Cli)
        .unwrap();
    e.drain().await.unwrap();
    e.pump(&mut watch).unwrap();
    let ran_at = e.cells["n"].provenance.at;

    std::fs::write(dir.path().join("src/other.log"), "x\n").unwrap();
    for _ in 0..10 {
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        e.cells["n"].provenance.at, ran_at,
        "non-matching path ran the cell"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_annotation_reruns_on_interval() {
    let (_d, mut e) = engine();
    let mut watch = e.store.watch().unwrap();
    e.set("tick", "$ date +%s # every: 30", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    let first = value(&e, "tick");
    assert!(!first.is_empty());

    // date has 1s resolution, so a re-run shows within ~1s of ticking.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "tick") == first {
        assert!(std::time::Instant::now() < deadline, "never re-ran");
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn quarantined_watched_cell_never_auto_runs() {
    let (_d, mut e) = engine();
    let mut watch = e.store.watch().unwrap();
    e.set("bad", "$ rm -rf junk # every: 20", Actor::Cli)
        .unwrap();
    e.drain().await.unwrap();
    for _ in 0..8 {
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(e.cells.get("bad").unwrap().state, CellState::Quarantined);
    assert_eq!(value(&e, "bad"), "");
}

#[tokio::test(flavor = "multi_thread")]
async fn passive_engine_never_watches() {
    let dir = tempfile::tempdir().unwrap();
    let _active = Engine::open(dir.path()).unwrap(); // holds the election
    let mut passive = Engine::open(dir.path()).unwrap();
    assert!(!passive.active);
    let mut watch = passive.store.watch().unwrap();
    passive
        .set("t", "$ echo hi # watch: src/** # every: 20", Actor::Cli)
        .unwrap();
    for _ in 0..8 {
        passive.pump(&mut watch).unwrap();
        passive.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(passive.repo_watch.is_none());
    assert_eq!(passive.cells.get("t").unwrap().state, CellState::Stale);
    assert_eq!(value(&passive, "t"), "");
}

#[test]
fn trigger_rejects_non_commands() {
    let (_d, mut e) = engine();
    e.set("x", "42", Actor::Cli).unwrap();
    assert!(e.trigger("x", Actor::Cli).is_err());
    assert!(e.trigger("ghost", Actor::Cli).is_err());
}

#[test]
fn action_cells_register_deps_and_go_stale() {
    let (_d, mut e) = engine();
    e.set("errors", "0", Actor::Cli).unwrap();
    e.set("commit", "! git commit -am {errors}", Actor::Cli)
        .unwrap();
    assert_eq!(e.cells.get("commit").unwrap().deps, ["errors"]);
    assert_eq!(e.cells.get("commit").unwrap().state, CellState::Stale);
    // Dirtying an action never runs it.
    e.set("errors", "1", Actor::Cli).unwrap();
    assert_eq!(e.cells.get("commit").unwrap().state, CellState::Stale);
    assert_eq!(e.cells.get("commit").unwrap().value, "");
}

#[test]
fn literal_overwrite_of_expr_recomputes_dependents() {
    let (_d, mut e) = engine();
    e.set("a", "1", Actor::Cli).unwrap();
    e.set("b", "= {a}", Actor::Cli).unwrap();
    e.set("c", "= {b} + 1", Actor::Cli).unwrap();
    e.set("b", "41", Actor::Cli).unwrap(); // pin b to a literal
    assert_eq!(value(&e, "c"), "42");
    assert_eq!(e.cells.get("b").unwrap().kind, CellKind::Literal);
}

#[tokio::test]
async fn agent_runs_queue_under_max_agents() {
    let (_d, mut e) = engine_with_agents(
        "max_agents = 1\n[agents]\nslow = \"sleep 0.2 && echo ran: {task} | head -1\"\n",
    );
    e.set("a", "@slow('one')", Actor::Cli).unwrap();
    e.set("b", "@slow('two')", Actor::Cli).unwrap();
    e.trigger("a", Actor::Human).unwrap();
    e.trigger("b", Actor::Human).unwrap();
    assert_eq!(e.cells["a"].state, CellState::Computing);
    assert_eq!(e.cells["b"].state, CellState::Queued);
    // Re-triggering a queued cell is refused like a running one.
    let err = e.trigger("b", Actor::Human).unwrap_err().to_string();
    assert!(err.contains("already queued"), "{err}");
    // A dep change while queued keeps it queued; the run picks up the
    // new value when its slot frees.
    e.set("b", "@slow('two {n}')", Actor::Cli).unwrap();
    assert_eq!(e.cells["b"].state, CellState::Stale, "a rewrite un-queues");
    e.trigger("b", Actor::Human).unwrap();
    e.set("n", "3", Actor::Cli).unwrap();
    assert_eq!(e.cells["b"].state, CellState::Queued);
    e.drain().await.unwrap();
    assert_eq!(value(&e, "a"), "ran: one");
    assert_eq!(value(&e, "b"), "ran: two 3");
    assert_eq!(e.agents_running, 0);
    assert!(e.agent_queue.is_empty());
}

/// A `claude` on [paths] that replays a recorded stream-json session,
/// reading a line of stdin at each permission request.
fn fake_streaming_claude(dir: &std::path::Path, recording: &str) -> std::path::PathBuf {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/agents/claude-2.1.280")
        .join(recording);
    let script = dir.join("claude");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nread -r line\nwhile IFS= read -r ev <&3; do\n  printf '%s\\n' \"$ev\"\n  case \"$ev\" in *'\"control_request\"'*) read -r line; echo \"$line\" > {answer};; esac\ndone 3< {src}\n",
            answer = dir.join("answer.json").display(),
            src = src.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[tokio::test]
async fn a_running_agents_question_lands_on_its_cell_and_the_answer_goes_back() {
    let tools = tempfile::tempdir().unwrap();
    let claude = fake_streaming_claude(tools.path(), "stream-permission-allowed.jsonl");
    let (_d, mut e) = engine_with_agents(&format!("[paths]\nclaude = \"{}\"\n", claude.display()));
    e.set("B2", "@claude('make a dir')", Actor::Cli).unwrap();
    e.trigger("B2", Actor::Human).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while e.cells["B2"].decision.is_none() {
        assert!(std::time::Instant::now() < deadline, "no question arrived");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        while let Ok((name, gen, ev)) = e.stream_rx.try_recv() {
            e.stream_event(name, gen, ev).unwrap();
        }
    }
    let cell = &e.cells["B2"];
    assert_eq!(cell.state, CellState::Computing, "the run is still going");
    assert_eq!(cell.activity.as_deref(), Some("Bash: mkdir probe-dir"));
    let d = cell.decision.as_ref().unwrap();
    assert!(d.question.contains("Bash: mkdir probe-dir"));
    assert_eq!(
        d.asker,
        Actor::Agent("B2's agent".into()),
        "not the id its MCP writes as"
    );
    e.resolve_decision("B2", Choice::Index(0), Actor::Human)
        .unwrap();
    assert!(e.cells["B2"].decision.is_none());
    e.drain().await.unwrap();
    assert_eq!(value(&e, "B2"), "done");
    assert!(e.cells["B2"].activity.is_none());
    let sent = std::fs::read_to_string(tools.path().join("answer.json")).unwrap();
    assert!(sent.contains("\"behavior\":\"allow\""), "{sent}");
}

#[tokio::test]
async fn an_answer_given_in_another_process_reaches_the_run() {
    let tools = tempfile::tempdir().unwrap();
    let claude = fake_streaming_claude(tools.path(), "stream-permission-allowed.jsonl");
    let (dir, mut e) = engine_with_agents(&format!("[paths]\nclaude = \"{}\"\n", claude.display()));
    e.set("B2", "@claude('make a dir')", Actor::Cli).unwrap();
    // The run starts in this engine, as it would in a driver's `recalc mcp`.
    e.trigger("B2", Actor::Human).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while e.cells["B2"].decision.is_none() {
        assert!(std::time::Instant::now() < deadline, "no question arrived");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        while let Ok((name, gen, ev)) = e.stream_rx.try_recv() {
            e.stream_event(name, gen, ev).unwrap();
        }
    }
    // The window is another engine on the same board.
    let mut window = Engine::open(dir.path()).unwrap();
    assert!(window.cells["B2"].decision.as_ref().unwrap().from_run);
    window
        .resolve_decision("B2", Choice::Index(0), Actor::Human)
        .unwrap();
    assert!(window.cells["B2"].decision.is_none());
    assert_ne!(
        value(&window, "B2"),
        "Allow once",
        "an answer to a run is not the cell's value"
    );
    assert!(e.collect_answers().unwrap());
    e.drain().await.unwrap();
    assert_eq!(value(&e, "B2"), "done");
    let sent = std::fs::read_to_string(tools.path().join("answer.json")).unwrap();
    assert!(sent.contains("\"behavior\":\"allow\""), "{sent}");
}

#[tokio::test]
async fn a_run_waiting_on_a_person_frees_its_slot() {
    let tools = tempfile::tempdir().unwrap();
    let claude = fake_streaming_claude(tools.path(), "stream-ask.jsonl");
    let (_d, mut e) = engine_with_agents(&format!(
        "max_agents = 1\n[paths]\nclaude = \"{}\"\n[agents]\nfine = \"echo ok\"\n",
        claude.display()
    ));
    e.set("B2", "@claude('ask me')", Actor::Cli).unwrap();
    e.set("B3", "@fine('go')", Actor::Cli).unwrap();
    e.trigger("B2", Actor::Human).unwrap();
    e.trigger("B3", Actor::Human).unwrap();
    assert_eq!(e.cells["B3"].state, CellState::Queued, "one slot, taken");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while e.cells["B2"].decision.is_none() {
        assert!(std::time::Instant::now() < deadline, "no question arrived");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        while let Ok((name, gen, ev)) = e.stream_rx.try_recv() {
            e.stream_event(name, gen, ev).unwrap();
        }
    }
    // B2 waits on a person; B3 runs in the slot it isn't using.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "B3") != "ok" {
        assert!(
            std::time::Instant::now() < deadline,
            "B3 never ran: {:?}",
            e.cells["B3"].state
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        while let Ok(done) = e.runs_rx.try_recv() {
            e.in_flight -= 1;
            e.apply_run(done).unwrap();
        }
    }
    e.stop("B2", Actor::Human).unwrap();
    e.drain().await.unwrap();
}

#[tokio::test]
async fn stopping_a_run_takes_its_question_down() {
    let tools = tempfile::tempdir().unwrap();
    let claude = fake_streaming_claude(tools.path(), "stream-ask.jsonl");
    let (_d, mut e) = engine_with_agents(&format!("[paths]\nclaude = \"{}\"\n", claude.display()));
    e.set("B2", "@claude('ask me')", Actor::Cli).unwrap();
    e.trigger("B2", Actor::Human).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while e.cells["B2"].decision.is_none() {
        assert!(std::time::Instant::now() < deadline, "no question arrived");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        while let Ok((name, gen, ev)) = e.stream_rx.try_recv() {
            e.stream_event(name, gen, ev).unwrap();
        }
    }
    assert_eq!(
        e.cells["B2"].decision.as_ref().unwrap().question,
        "Do you prefer tea or coffee?"
    );
    e.stop("B2", Actor::Human).unwrap();
    assert!(e.cells["B2"].decision.is_none());
    assert!(e.pending_asks.is_empty());
    e.drain().await.unwrap();
}

#[tokio::test]
async fn a_rate_limit_holds_the_queue_instead_of_failing_it() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/agents/claude-2.1.280/rate-limit.synthetic.json"
    );
    // Limited on the first call, answering on the next, as an account
    // is once its limit resets.
    let (_d, mut e) = engine_with_agents(&format!(
        "max_agents = 1\n[agents]\nlimited = \"if [ -f .tried ]; then echo answered; else touch .tried; cat {fixture}; fi\"\nfine = \"echo ok\"\n"
    ));
    e.set("a", "@limited('x')", Actor::Cli).unwrap();
    e.set("b", "@fine('y')", Actor::Cli).unwrap();
    e.set("c", "@fine('z')", Actor::Cli).unwrap();
    e.trigger("a", Actor::Human).unwrap();
    e.trigger("b", Actor::Human).unwrap();
    e.drain().await.unwrap();
    // The cell that hit the limit isn't an error: it waits first in line.
    assert_eq!(
        e.cells["a"].state,
        CellState::Queued,
        "{:?}",
        e.cells["a"].state
    );
    assert_eq!(e.agent_queue.front().map(String::as_str), Some("a"));
    assert!(e.agents_paused_until().is_some());
    assert_eq!(
        e.cells["b"].state,
        CellState::Queued,
        "held, not sent into the limit"
    );
    // An agent's trigger joins the queue behind the hold…
    e.trigger("c", Actor::Agent("driver".into())).unwrap();
    assert_eq!(e.cells["c"].state, CellState::Queued);
    // …an agent can't jump the line, but a person can.
    assert!(e.trigger("c", Actor::Agent("driver".into())).is_err());
    e.trigger("c", Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "c"), "ok", "run by hand during the hold");
    assert!(!e.agent_queue.contains(&"c".to_string()));
    // …and when the hold runs out, the queue starts again.
    e.agents_paused_until = Some(1);
    assert!(e.lift_expired_hold().unwrap());
    e.drain().await.unwrap();
    assert_eq!(value(&e, "a"), "answered", "the limited cell ran again");
    assert_eq!(value(&e, "b"), "ok");
}

#[tokio::test]
async fn a_person_running_an_agent_lifts_the_hold() {
    let (_d, mut e) = engine_with_agents("max_agents = 1\n[agents]\nfine = \"echo ok\"\n");
    e.set("a", "@fine('x')", Actor::Cli).unwrap();
    e.agents_paused_until = Some(u64::MAX);
    e.trigger("a", Actor::Agent("driver".into())).unwrap();
    assert_eq!(e.cells["a"].state, CellState::Queued);
    e.stop("a", Actor::Human).unwrap();
    e.trigger("a", Actor::Human).unwrap();
    assert!(e.agents_paused_until().is_none());
    e.drain().await.unwrap();
    assert_eq!(value(&e, "a"), "ok");
}

#[tokio::test]
async fn stop_kills_a_run_and_unqueues_the_rest() {
    let (_d, mut e) =
        engine_with_agents("max_agents = 1\n[agents]\nslow = \"sleep 30; echo done\"\n");
    e.set("a", "@slow('one')", Actor::Cli).unwrap();
    e.set("b", "@slow('two')", Actor::Cli).unwrap();
    e.trigger("a", Actor::Human).unwrap();
    e.trigger("b", Actor::Human).unwrap();
    assert_eq!(e.cells["b"].state, CellState::Queued);
    // Stopping the queued one just drops it from the line.
    assert!(e.stop("b", Actor::Human).unwrap());
    assert_eq!(e.cells["b"].state, CellState::Stale);
    // Stopping the running one kills it; its dead result changes nothing.
    let started = std::time::Instant::now();
    assert!(e.stop("a", Actor::Human).unwrap());
    assert_eq!(e.cells["a"].state, CellState::Stale);
    e.drain().await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(value(&e, "a"), "");
    assert_eq!(e.agent_load(), (0, 0));
    assert!(!e.stop("a", Actor::Human).unwrap(), "nothing left to stop");
    // A stopped cell runs again on request.
    e.trigger("a", Actor::Human).unwrap();
    assert_eq!(e.stop_all(Actor::Human).unwrap(), 1);
    e.drain().await.unwrap();
}

#[tokio::test]
async fn a_citation_backs_one_answer() {
    let (_d, mut e) = engine_with_agents("[agents]\nstub = \"echo answered; : {task}\"\n");
    let a1 = Actor::Agent("a1".into());
    let cite = |quote: &str| Cite {
        file: "contracts/msa.md".into(),
        quote: quote.into(),
        location: Some("§7.2".into()),
        by: a1.clone(),
        at: 1,
        found: None,
    };
    // No cell, no citation; a path outside the board is refused.
    assert!(e.set_cite("B2", Some(cite("x"))).is_err());
    e.set("B2", "Consent required", a1.clone()).unwrap();
    let mut outside = cite("x");
    outside.file = "../secrets.md".into();
    assert!(e.set_cite("B2", Some(outside)).is_err());
    e.set_cite("B2", Some(cite("Neither party may assign")))
        .unwrap();
    assert_eq!(
        e.cells["B2"].cite.as_ref().unwrap().quote,
        "Neither party may assign"
    );
    // Meta survives a reopen.
    let dir = e.dir().to_path_buf();
    drop(e);
    let mut e = Engine::open(&dir).unwrap();
    assert!(e.cells["B2"].cite.is_some());
    // A human overwriting the value drops the citation: it backed the
    // old answer. A cell moving keeps it.
    e.set("B2", "Notice only", Actor::Human).unwrap();
    assert!(e.cells["B2"].cite.is_none());
    e.set_cite("B2", Some(cite("upon notice"))).unwrap();
    e.insert(None, Axis::Rows, 1, 1, Actor::Human).unwrap();
    assert_eq!(e.cells["B3"].cite.as_ref().unwrap().quote, "upon notice");
    // Running the cell again clears it before the agent starts.
    e.set("C1", "@stub('q')", Actor::Human).unwrap();
    e.trigger("C1", Actor::Human).unwrap();
    e.drain().await.unwrap();
    e.set_cite("C1", Some(cite("first run"))).unwrap();
    e.trigger("C1", Actor::Human).unwrap();
    assert!(e.cells["C1"].cite.is_none());
    e.drain().await.unwrap();
    assert_eq!(value(&e, "C1"), "answered");
    assert!(e.cells["C1"].cite.is_none());
}

#[tokio::test]
async fn watch_loop_mirrors_meta_only_writes() {
    // A sibling's meta-only write moves no value file; the watch must see meta
    // on its own.
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    e.set("A1", "Delaware", Actor::Cli).unwrap();
    let mut watch = e.store.watch().unwrap();
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = seen.clone();
    let handle = tokio::spawn(async move {
        loop {
            e.pump(&mut watch).unwrap();
            if e.cells["A1"].cite.is_some() {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    });
    let sibling = Store::open(dir.path()).unwrap();
    let mut copy = sibling.get("A1").unwrap().unwrap();
    copy.cite = Some(Cite {
        file: "docs/msa.md".into(),
        quote: "laws of Delaware".into(),
        location: None,
        by: Actor::Agent("a1".into()),
        at: 1,
        found: None,
    });
    sibling.set_state(&copy).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !seen.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(
            std::time::Instant::now() < deadline,
            "meta-only write never reached the mirror"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    handle.abort();
}

#[tokio::test]
async fn a_spent_budget_holds_the_queue_but_not_a_cell_run_by_hand() {
    let (_d, mut e) = engine_with_agents("max_agents = 1\n[agents]\nfake = \"echo done\"\n");
    for n in ["A1", "A2", "A3"] {
        e.set(n, "@fake('go')", Actor::Cli).unwrap();
    }
    // A1 has run already and reported what it cost.
    e.cells.get_mut("A1").unwrap().usage = Some(crate::model::Usage {
        input_tokens: 1,
        output_tokens: 1,
        cost_usd: Some(0.6),
        turns: None,
        model: None,
    });
    e.cells.get_mut("A1").unwrap().state = CellState::Ok;
    e.set_budget(Some(0.5), Actor::Human).unwrap();
    assert!(e.over_budget());
    let names: Vec<String> = ["A2", "A3"].iter().map(|s| s.to_string()).collect();
    let out = e.trigger_many(&names, Actor::Human, false);
    assert_eq!(out.ran, ["A2", "A3"]);
    assert_eq!(e.cells["A2"].state, CellState::Queued, "held, not started");
    assert!(e.activity().any(|l| l.text.contains("agent budget")));
    // A cell a person runs on its own still runs.
    e.set("A4", "@fake('go')", Actor::Cli).unwrap();
    e.trigger("A4", Actor::Human).unwrap();
    assert_eq!(e.cells["A4"].state, CellState::Computing);
    e.drain().await.unwrap();
    assert_eq!(e.cells["A2"].state, CellState::Queued, "still held");
    // Raising the budget lets the queue go.
    e.set_budget(Some(5.0), Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "A2"), "done");
}

#[tokio::test]
async fn a_selection_runs_a_cell_after_the_cells_it_reads() {
    // K2 explains J2; selected together, K2 must wait for J2.
    let (_d, mut e) = engine_with_agents(
        "max_agents = 4\n[agents]\nslow = \"sleep 0.3; echo High\"\nfake = \"echo rated: {task} | head -1\"\n",
    );
    e.set("J2", "@slow('rate it')", Actor::Cli).unwrap();
    e.set("n", "= concat({J2}, '!')", Actor::Cli).unwrap();
    e.set("K2", "@fake('why {n}')", Actor::Cli).unwrap();
    let names = vec!["K2".to_string(), "J2".to_string()];
    e.trigger_many(&names, Actor::Human, false);
    e.drain().await.unwrap();
    assert_eq!(value(&e, "J2"), "High");
    assert!(value(&e, "K2").contains("why High!"), "{}", value(&e, "K2"));
    assert_eq!(e.cells["K2"].state, CellState::Ok);
}

#[tokio::test]
async fn a_passive_engine_never_pins_an_answer_it_reads_half_written() {
    // A passive engine reading between the active engine's value and meta
    // writes must not take the answer for a raw edit and pin it.
    let (dir, mut e) = engine_with_agents("[agents]\nfake = \"echo Globex\"\n");
    e.set("B3", "@fake('who')", Actor::Cli).unwrap();
    e.set("J2", "@fake('rate')", Actor::Cli).unwrap();
    let mut sibling = Engine::open(dir.path()).unwrap();
    assert!(!sibling.active);
    std::fs::write(dir.path().join(".recalc/cells/B3"), "Globex").unwrap();
    sibling.reconcile("B3").unwrap();
    assert!(
        dir.path().join(".recalc/defs/B3").exists(),
        "the def was dropped"
    );
    assert_eq!(sibling.cells["B3"].kind, CellKind::AgentCall);
    assert_eq!(value(&sibling, "B3"), "Globex");

    // Nor does it sweep: a run that just ended looks orphaned from its snapshot.
    sibling.cells.get_mut("J2").unwrap().state = CellState::Computing;
    sibling.reset_orphaned_runs().unwrap();
    e.trigger("J2", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cells["J2"].state, CellState::Ok);
    let on_disk = crate::store::Store::open(dir.path())
        .unwrap()
        .get("J2")
        .unwrap()
        .unwrap();
    assert_eq!(on_disk.state, CellState::Ok);

    // A mark the sibling writes starts from what's on disk, not from
    // its trailing copy (which still has J2 empty).
    sibling
        .set_mark(
            "J2",
            Some(Mark {
                kind: crate::model::MarkKind::Verified,
                note: None,
                by: Actor::Agent("driver".into()),
                at: 1,
            }),
        )
        .unwrap();
    let on_disk = crate::store::Store::open(dir.path())
        .unwrap()
        .get("J2")
        .unwrap()
        .unwrap();
    assert_eq!(on_disk.value, "Globex");
    assert_eq!(on_disk.state, CellState::Ok);
}

#[tokio::test]
async fn a_sibling_process_can_cite_a_running_cell() {
    // The agent's MCP server is another process on the same directory:
    // it writes the citation to meta while this engine owns the run.
    let (dir, mut e) = engine_with_agents("[agents]\nslow = \"sleep 0.3; echo Delaware\"\n");
    e.set("C1", "@slow('law')", Actor::Human).unwrap();
    e.trigger("C1", Actor::Human).unwrap();
    assert_eq!(e.cells["C1"].state, CellState::Computing);
    let sibling = crate::store::Store::open(dir.path()).unwrap();
    let mut copy = sibling.get("C1").unwrap().unwrap();
    copy.state = CellState::Ok; // the sibling's stale idea of the state
    copy.cite = Some(Cite {
        file: "contracts/msa.md".into(),
        quote: "laws of the State of Delaware".into(),
        location: None,
        by: Actor::Agent("a1".into()),
        at: 1,
        found: None,
    });
    sibling.set_state(&copy).unwrap();
    // Mirroring the meta keeps the run's state and takes the citation.
    e.reconcile("C1").unwrap();
    assert_eq!(e.cells["C1"].state, CellState::Computing);
    assert!(e.cells["C1"].cite.is_some());
    // Even if the mirror missed it, the finished run adopts what's on disk.
    e.cells.get_mut("C1").unwrap().cite = None;
    e.drain().await.unwrap();
    assert_eq!(value(&e, "C1"), "Delaware");
    assert_eq!(
        e.cells["C1"].cite.as_ref().unwrap().quote,
        "laws of the State of Delaware"
    );
    assert!(sibling.get("C1").unwrap().unwrap().cite.is_some());
}

#[tokio::test]
async fn verified_cell_turns_agent_writes_into_suggestions() {
    let (_d, mut e) = engine();
    let a1 = Actor::Agent("a1".into());
    e.set("B2", "yes", a1.clone()).unwrap();
    e.set_mark(
        "B2",
        Some(Mark {
            kind: crate::model::MarkKind::Verified,
            note: Some("checked against §4.2".into()),
            by: Actor::Human,
            at: 1,
        }),
    )
    .unwrap();
    // The agent's rewrite is held, not applied.
    e.set_noted("B2", "no", a1.clone(), Some("re-read the clause".into()))
        .unwrap();
    let cell = &e.cells["B2"];
    assert_eq!(cell.value, "yes");
    assert_eq!(cell.pending_suggestions.len(), 1);
    assert_eq!(cell.pending_suggestions[0].value, "no");
    assert_eq!(
        cell.pending_suggestions[0].note.as_deref(),
        Some("re-read the clause")
    );
    // The human's own write lands and keeps the mark.
    e.set("B2", "no", Actor::Human).unwrap();
    assert_eq!(value(&e, "B2"), "no");
    assert!(e.cells["B2"].verified());
    // Fill over a verified cell leaves it alone.
    e.set("B1", "= 1 + 1", Actor::Human).unwrap();
    let changed = e.fill("B1", "B3", Fill::Down, Actor::Human).unwrap();
    assert_eq!(
        changed.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["B3"]
    );
    assert_eq!(value(&e, "B2"), "no");
    // Clearing the mark releases the cell; marking an empty cell creates it.
    e.set_mark("B2", None).unwrap();
    e.set("B2", "maybe", a1).unwrap();
    assert_eq!(value(&e, "B2"), "maybe");
    e.set_mark(
        "Z9",
        Some(Mark {
            kind: crate::model::MarkKind::Flagged,
            note: None,
            by: Actor::Human,
            at: 2,
        }),
    )
    .unwrap();
    assert!(e.cells["Z9"].mark.is_some());
    assert!(e.set_mark("bad name", None).is_err());
}

#[tokio::test]
async fn trigger_many_runs_a_selection_and_skips_what_it_should() {
    let (_d, mut e) = engine_with_agents("[agents]\nfake = \"echo {task} | head -1\"\n");
    e.set("A1", "header", Actor::Cli).unwrap();
    e.set("B1", "@fake('one')", Actor::Cli).unwrap();
    e.set("B2", "@fake('two')", Actor::Cli).unwrap();
    e.set("B3", "@fake('three')", Actor::Cli).unwrap();
    e.set("B4", "$ rm -rf junk", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    e.set_mark(
        "B3",
        Some(Mark {
            kind: crate::model::MarkKind::Verified,
            note: None,
            by: Actor::Human,
            at: 1,
        }),
    )
    .unwrap();
    let names: Vec<String> = ["A1", "B1", "B2", "B3", "B4", "Q9"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let out = e.trigger_many(&names, Actor::Human, false);
    assert_eq!(out.ran, ["B1", "B2"]);
    assert_eq!(out.need_confirm, ["B4"]);
    e.drain().await.unwrap();
    assert_eq!(value(&e, "B1"), "one");
    assert_eq!(value(&e, "B2"), "two");
    assert_eq!(value(&e, "B3"), "", "verified cells are not re-run in bulk");
    // A second pass leaves finished answers alone, unless asked to
    // run them all; running cells skip either way.
    let out = e.trigger_many(&names, Actor::Human, false);
    assert!(out.ran.is_empty(), "{:?}", out.ran);
    assert_eq!(out.up_to_date, ["B1", "B2"]);
    e.trigger("B1", Actor::Human).unwrap();
    let out = e.trigger_many(&names, Actor::Human, true);
    assert_eq!(out.ran, ["B2"]);
    e.drain().await.unwrap();
}

#[tokio::test]
async fn fill_honors_dollar_anchors() {
    // The review-table shape: a question per column header, a document
    // per row, one prompt filled across the whole block.
    let (_d, mut e) = engine_with_agents("[agents]\nfake = \"echo {task} | head -1\"\n");
    e.set("A2", "doc-a", Actor::Cli).unwrap();
    e.set("A3", "doc-b", Actor::Cli).unwrap();
    e.set("B1", "governing law?", Actor::Cli).unwrap();
    e.set("C1", "term?", Actor::Cli).unwrap();
    e.set("B2", "@fake('{B$1} in {$A2}')", Actor::Cli).unwrap();
    e.fill("B2", "B3", Fill::Down, Actor::Human).unwrap();
    e.fill("B2", "C3", Fill::Right, Actor::Human).unwrap();
    assert_eq!(
        e.cells["B3"].source.as_deref(),
        Some("@fake('{B$1} in {$A3}')")
    );
    assert_eq!(
        e.cells["C2"].source.as_deref(),
        Some("@fake('{C$1} in {$A2}')")
    );
    assert_eq!(
        e.cells["C3"].source.as_deref(),
        Some("@fake('{C$1} in {$A3}')")
    );
    let names: Vec<String> = ["B2", "B3", "C2", "C3"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    e.trigger_many(&names, Actor::Human, false);
    e.drain().await.unwrap();
    assert_eq!(value(&e, "C3"), "term? in doc-b");
    assert_eq!(e.cells["C3"].deps, ["C1", "A3"]);
}

#[test]
fn set_many_recomputes_once_and_reports_every_refusal() {
    let (_d, mut e) = engine();
    e.set("total", "= sum({A1:A3})", Actor::Cli).unwrap();
    e.set("loop", "= {x}", Actor::Cli).unwrap();
    let err = e
        .set_many(
            vec![
                ("A1".into(), "1".into()),
                ("A2".into(), "2".into()),
                ("x".into(), "= {loop}".into()),
                ("A3".into(), "3".into()),
                ("bad name".into(), "v".into()),
            ],
            Actor::Agent("a1".into()),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("cycle") && err.contains("bad name"), "{err}");
    assert_eq!(value(&e, "total"), "6", "the good writes still landed");
}

#[test]
fn take_changed_reports_touched_cells_once() {
    let (_d, mut e) = engine();
    e.set("a", "1", Actor::Cli).unwrap();
    e.set("b", "= {a} + 1", Actor::Cli).unwrap();
    e.take_changed();
    e.set("a", "2", Actor::Cli).unwrap();
    let mut changed: Vec<String> = e.take_changed().into_iter().collect();
    changed.sort();
    assert_eq!(changed, ["a", "b"]);
    assert!(e.take_changed().is_empty());
    e.set_series("b", true).unwrap();
    assert!(e.take_changed().contains("b"));
}

#[test]
fn delete_removes_cells_and_recomputes_dependents() {
    let (d, mut e) = engine();
    e.set("a", "1", Actor::Cli).unwrap();
    e.set("b", "= {a} + 1", Actor::Cli).unwrap();
    let removed = e
        .delete_many(&["a".to_string(), "ghost".to_string()], Actor::Human)
        .unwrap();
    assert_eq!(removed, [("a".to_string(), "1".to_string())]);
    assert!(e.cell("a").is_none());
    assert!(!d.path().join(".recalc/cells/a").exists());
    assert_eq!(
        value(&e, "b"),
        "1",
        "a missing cell is a blank, 0 in arithmetic"
    );
    assert!(e.take_changed().contains("a"));
}

#[test]
fn paste_sources_shifts_like_fill_and_respects_gaps() {
    let (_d, mut e) = engine();
    for (n, v) in [("A1", "1"), ("A2", "2"), ("A5", "10"), ("A6", "20")] {
        e.set(n, v, Actor::Cli).unwrap();
    }
    e.set("B1", "= {A1} * 2", Actor::Cli).unwrap();
    e.set("B2", "= {$A$1} + {A2}", Actor::Cli).unwrap();
    // Copy B1:B2 (with a gap in the block) and paste at B5.
    let block = vec![
        vec![Some("= {A1} * 2".to_string())],
        vec![Some("= {$A$1} + {A2}".to_string())],
        vec![None],
    ];
    let changed = e.paste_sources("B5", "B1", &block, Actor::Human).unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!(e.cells["B5"].source.as_deref(), Some("= {A5} * 2"));
    assert_eq!(e.cells["B6"].source.as_deref(), Some("= {$A$1} + {A6}"));
    assert_eq!(value(&e, "B5"), "20");
    assert_eq!(value(&e, "B6"), "21");
    assert!(e.cell("B7").is_none(), "a gap pastes nothing");
    // Pasting above the origin: a ref that would leave the sheet stays.
    e.paste_sources(
        "C1",
        "B2",
        &[vec![Some("= {A2} + {A1}".to_string())]],
        Actor::Human,
    )
    .unwrap();
    assert_eq!(e.cells["C1"].source.as_deref(), Some("= {B1} + {A1}"));
}

#[test]
fn paste_text_lands_literals_tsv_or_csv() {
    let (_d, mut e) = engine();
    e.paste_text("A1", "x\t= 1 + 1\ny\tz\n", Actor::Human)
        .unwrap();
    assert_eq!(value(&e, "B1"), "= 1 + 1");
    assert_eq!(e.cells["B1"].kind, CellKind::Literal);
    assert_eq!(value(&e, "B2"), "z");
    e.paste_text("D1", "\"a, b\",c", Actor::Human).unwrap();
    assert_eq!(value(&e, "D1"), "a, b");
    assert_eq!(value(&e, "E1"), "c");
}

#[tokio::test]
async fn insert_rows_moves_cells_and_references() {
    let (d, mut e) = engine();
    e.set("A1", "header", Actor::Cli).unwrap();
    e.set("A2", "1", Actor::Cli).unwrap();
    e.set("A3", "2", Actor::Cli).unwrap();
    e.set("B3", "= {A3} * 10", Actor::Cli).unwrap();
    e.set("total", "= sum({A2:A3}) + {$A$2}", Actor::Cli)
        .unwrap();
    e.set_series("A3", true).unwrap();
    e.set_mark(
        "B3",
        Some(Mark {
            kind: crate::model::MarkKind::Verified,
            note: None,
            by: Actor::Human,
            at: 1,
        }),
    )
    .unwrap();
    e.set("A3", "3", Actor::Cli).unwrap(); // some history for A3
    assert_eq!(value(&e, "total"), "5");

    e.insert(None, Axis::Rows, 2, 2, Actor::Human).unwrap();
    // Row 1 stays; rows 2.. moved down two.
    assert_eq!(value(&e, "A1"), "header");
    assert!(e.cell("A2").is_none() && e.cell("A3").is_none());
    assert_eq!(value(&e, "A4"), "1");
    assert_eq!(value(&e, "A5"), "3");
    assert_eq!(e.cells["B5"].source.as_deref(), Some("= {A5} * 10"));
    assert_eq!(value(&e, "B5"), "30", "values move without recomputing");
    assert!(e.cells["B5"].verified(), "marks travel with the cell");
    assert!(e.cells["A5"].series, "series travels with the cell");
    assert_eq!(e.history("A5", 10).unwrap().len(), 2, "history travels");
    assert_eq!(
        e.cells["total"].source.as_deref(),
        Some("= sum({A4:A5}) + {$A$4}")
    );
    assert_eq!(e.cells["total"].deps, ["A4", "A5"]);
    // Still live: a change in the moved input propagates.
    e.set("A4", "10", Actor::Cli).unwrap();
    assert_eq!(value(&e, "total"), "23");
    // Disk agrees with the mirror.
    let store = Store::open(d.path()).unwrap();
    assert_eq!(store.get("A5").unwrap().unwrap().value, "3");
    assert!(store.get("A3").unwrap().is_none());
    assert!(!d.path().join(".recalc/history/A3.log").exists());
}

#[tokio::test]
async fn deleting_a_ranges_last_row_shrinks_it_instead_of_taking_the_next() {
    let (_d, mut e) = engine();
    for (n, v) in [("A1", "1"), ("A2", "2"), ("A3", "100"), ("A4", "5")] {
        e.set(n, v, Actor::Cli).unwrap();
    }
    e.set("B1", "= sum({A1:A2})", Actor::Cli).unwrap();
    e.set("C1", "= sum({A2:A2})", Actor::Cli).unwrap();
    e.set("D1", "= sum({A2:A4})", Actor::Cli).unwrap();
    e.set("E1", "= sum({$A$1:$A$2})", Actor::Cli).unwrap();
    e.delete(None, Axis::Rows, 2, 1, Actor::Human).unwrap();
    // A3 (100) moved up into row 2; a range that ended on the deleted
    // row must not take it in.
    assert_eq!(e.cells["B1"].source.as_deref(), Some("= sum({A1:A1})"));
    assert_eq!(value(&e, "B1"), "1");
    assert_eq!(e.cells["E1"].source.as_deref(), Some("= sum({$A$1:$A$1})"));
    // A range wholly in the band is gone, and says so.
    assert_eq!(e.cells["C1"].source.as_deref(), Some("= sum({#REF!})"));
    assert!(matches!(e.cells["C1"].state, CellState::Error(_)));
    // One that started on it keeps the rows after.
    assert_eq!(e.cells["D1"].source.as_deref(), Some("= sum({A2:A3})"));
    assert_eq!(value(&e, "D1"), "105");
    // Row 1 deleted: a range on it alone is gone, one starting on it
    // keeps the rows after.
    e.set("G9", "= sum({A1:A1})", Actor::Cli).unwrap();
    e.set("H9", "= sum({A1:A3})", Actor::Cli).unwrap();
    e.delete(None, Axis::Rows, 1, 1, Actor::Human).unwrap();
    assert_eq!(e.cells["G8"].source.as_deref(), Some("= sum({#REF!})"));
    assert_eq!(e.cells["H8"].source.as_deref(), Some("= sum({A1:A2})"));
    assert_eq!(value(&e, "H8"), "105");
}

#[tokio::test]
async fn formulas_read_run_and_review_facts_and_follow_them() {
    let (_d, mut e) = engine();
    e.set("chk", "$ echo nope >&2; exit 3", Actor::Cli).unwrap();
    e.set("passed", "= {chk.code} == 0", Actor::Cli).unwrap();
    e.set("why", "= {chk.err}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "passed"), "false");
    assert_eq!(value(&e, "why"), "nope");
    assert_eq!(e.cells["passed"].deps, ["chk"]);

    // A review column as a formula: verifying refreshes it, and a
    // command reading the same cells' values doesn't run again.
    for (n, v) in [("B2", "Yes"), ("B3", "No")] {
        e.set(n, v, Actor::Human).unwrap();
    }
    e.set("done", "= countif({B2:B3.mark}, 'verified')", Actor::Cli)
        .unwrap();
    e.set(
        "val",
        "$ echo {B2} $(python3 -c 'import time; print(time.time_ns())')",
        Actor::Cli,
    )
    .unwrap();
    e.drain().await.unwrap();
    let ran = value(&e, "val");
    assert_eq!(value(&e, "done"), "0");
    let verified = crate::model::Mark {
        kind: crate::model::MarkKind::Verified,
        note: None,
        by: Actor::Human,
        at: 1,
    };
    e.set_mark("B2", Some(verified)).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "done"), "1");
    assert_eq!(value(&e, "val"), ran, "a value reader didn't run again");
    e.set("by", "= {B3.by}", Actor::Cli).unwrap();
    assert_eq!(value(&e, "by"), "human");

    // Fill and delete move a suffixed range like any other.
    e.set("C2", "= countif({B2:B3.mark}, 'verified')", Actor::Cli)
        .unwrap();
    e.fill("C2", "D2", Fill::Right, Actor::Human).unwrap();
    assert_eq!(
        e.cells["D2"].source.as_deref(),
        Some("= countif({C2:C3.mark}, 'verified')")
    );
    e.drain().await.unwrap();
    e.delete(None, Axis::Rows, 3, 1, Actor::Human).unwrap();
    assert_eq!(
        e.cells["C2"].source.as_deref(),
        Some("= countif({B2:B2.mark}, 'verified')")
    );
}

#[test]
fn lists_move_item_by_item_and_shell_braces_stay() {
    assert_eq!(
        shift_refs("= countif({A2, plan_1, B2.mark}, 'done')", 0, 1),
        "= countif({A3, plan_1, B3.mark}, 'done')"
    );
    for shell in [
        "$ cp f{1,2} x",
        "$ echo '{\"a\": 1, \"b\": 2}'",
        "$ awk '{print $1, $2}'",
    ] {
        assert_eq!(shift_refs(shell, 0, 1), shell);
    }
    let down = |c: u32, r: u32| (r != 2).then_some((c, if r > 2 { r - 1 } else { r }));
    let corner = |c: u32, r: u32, _: bool, _: bool| (c, r);
    assert_eq!(
        remap_refs("= count({A1, A2, A3})", &down, &corner, &|_| true),
        "= count({A1, #REF!, A2})"
    );
    assert_eq!(
        remap_refs("$ cp f{1,2} x", &down, &corner, &|_| true),
        "$ cp f{1,2} x"
    );
}

#[tokio::test]
async fn a_command_reads_a_block_as_one_word_per_cell() {
    let (_d, mut e) = engine();
    e.set("A1", "it's", Actor::Cli).unwrap();
    e.set("A2", "two words", Actor::Cli).unwrap();
    e.set("B1", "3", Actor::Cli).unwrap();
    e.set("s", "$ printf '[%s]' {A1:B2}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    // Row by row, every cell its own argument, an empty one included.
    assert_eq!(value(&e, "s"), "[it's][3][two words][]");
    assert_eq!(e.cells["s"].deps, ["A1", "B1", "A2", "B2"]);
    // A member changing re-runs it.
    e.set("B2", "x", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "s"), "[it's][3][two words][x]");
    // Fill moves the block like any reference, and a row delete shrinks it.
    e.set("C1", "$ echo {A1:A2} | wc -w", Actor::Cli).unwrap();
    e.fill("C1", "C2", Fill::Down, Actor::Human).unwrap();
    assert_eq!(
        e.cells["C2"].source.as_deref(),
        Some("$ echo {A2:A3} | wc -w")
    );
    e.drain().await.unwrap();
    e.delete(None, Axis::Rows, 2, 1, Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(
        e.cells["C1"].source.as_deref(),
        Some("$ echo {A1:A1} | wc -w")
    );
    assert_eq!(value(&e, "C1").trim(), "1");
}

#[tokio::test]
async fn delete_rows_closes_the_gap_and_shrinks_ranges() {
    let (_d, mut e) = engine();
    for (n, v) in [("A1", "1"), ("A2", "2"), ("A3", "3"), ("A4", "4")] {
        e.set(n, v, Actor::Cli).unwrap();
    }
    e.set("B1", "= sum({A1:A4})", Actor::Cli).unwrap();
    e.set("C1", "= {A3}", Actor::Cli).unwrap();
    e.set("D1", "= {A4}", Actor::Cli).unwrap();
    e.delete(None, Axis::Rows, 2, 2, Actor::Human).unwrap();
    assert_eq!(value(&e, "A1"), "1");
    assert_eq!(value(&e, "A2"), "4");
    assert!(e.cell("A3").is_none() && e.cell("A4").is_none());
    // The range lost two rows, the ref to A4 followed it, and the ref to
    // deleted A3 is #REF!.
    assert_eq!(e.cells["B1"].source.as_deref(), Some("= sum({A1:A2})"));
    assert_eq!(e.cells["D1"].source.as_deref(), Some("= {A2}"));
    assert_eq!(e.cells["C1"].source.as_deref(), Some("= {#REF!}"));
    // The rewritten formulas computed again with the delete itself.
    assert_eq!(value(&e, "B1"), "5");
    assert_eq!(value(&e, "D1"), "4");
    assert!(
        matches!(e.cells["C1"].state, CellState::Error(_)),
        "{:?}",
        e.cells["C1"].state
    );
    // Writing a new A3 doesn't revive the reference.
    e.set("A3", "later", Actor::Cli).unwrap();
    assert_eq!(
        e.cells["C1"].source.as_deref(),
        Some("= {#REF!}"),
        "a new A3 is not the deleted one"
    );
    e.set("A2", "40", Actor::Cli).unwrap();
    assert_eq!(value(&e, "B1"), "41");
    assert_eq!(value(&e, "D1"), "40");
}

#[tokio::test]
async fn a_command_or_prompt_reading_a_deleted_cell_refuses_to_run() {
    let (_d, mut e) = engine();
    e.set("A2", "doc.md", Actor::Cli).unwrap();
    e.set("cmd", "$ echo {A2}", Actor::Cli).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "cmd"), "doc.md");
    e.delete(None, Axis::Rows, 2, 1, Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cells["cmd"].source.as_deref(), Some("$ echo {#REF!}"));
    assert!(
        matches!(&e.cells["cmd"].state, CellState::Error(m) if m.contains("deleted")),
        "{:?}",
        e.cells["cmd"].state
    );
    assert!(e.trigger("cmd", Actor::Human).is_err());
}

#[tokio::test]
async fn insert_columns_and_refusal_while_running() {
    let (_d, mut e) = engine_with_agents("[agents]\nslow = \"sleep 30\"\n");
    e.set("A1", "x", Actor::Cli).unwrap();
    e.set("B1", "= {A1} & 'y'", Actor::Cli).unwrap();
    e.set("named", "= {B1}", Actor::Cli).unwrap();
    e.insert(None, Axis::Cols, 1, 1, Actor::Human).unwrap();
    assert_eq!(value(&e, "B1"), "x");
    assert_eq!(e.cells["C1"].source.as_deref(), Some("= {B1} & 'y'"));
    assert_eq!(e.cells["named"].source.as_deref(), Some("= {C1}"));
    assert_eq!(value(&e, "named"), "xy");
    e.delete(None, Axis::Cols, 1, 1, Actor::Human).unwrap();
    assert_eq!(e.cells["named"].source.as_deref(), Some("= {B1}"));

    e.set("run", "@slow('go')", Actor::Cli).unwrap();
    e.trigger("run", Actor::Human).unwrap();
    let err = e
        .insert(None, Axis::Rows, 1, 1, Actor::Human)
        .unwrap_err()
        .to_string();
    assert!(err.contains("still running"), "{err}");
    e.stop_all(Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert!(e.insert(None, Axis::Rows, 1, 1, Actor::Human).is_ok());
    assert!(e.insert(None, Axis::Rows, 0, 1, Actor::Human).is_err());
}

#[tokio::test]
async fn sort_rows_permutes_data_rows_and_keeps_ranges() {
    let (_d, mut e) = engine();
    e.set("A1", "name", Actor::Cli).unwrap();
    e.set("B1", "score", Actor::Cli).unwrap();
    for (r, (n, v)) in [("carol", "7"), ("alice", ""), ("bob", "12"), ("dave", "x")]
        .iter()
        .enumerate()
    {
        let row = r as u32 + 2;
        e.set(&a1_name(1, row), n, Actor::Cli).unwrap();
        e.set(&a1_name(2, row), v, Actor::Cli).unwrap();
        e.set(&a1_name(3, row), &format!("= {{B{row}}} + 1"), Actor::Cli)
            .unwrap();
    }
    e.set("total", "= sum({B2:B5})", Actor::Cli).unwrap();
    e.set("top", "= {A2}", Actor::Cli).unwrap();
    assert_eq!(value(&e, "total"), "19");

    e.sort_rows(None, 2, 2, 5, false, Actor::Human).unwrap();
    // Numbers ascending, then text, blanks last.
    let names: Vec<String> = (2..=5).map(|r| value(&e, &a1_name(1, r))).collect();
    assert_eq!(names, ["carol", "bob", "dave", "alice"]);
    // Row-local formulas moved with their rows and still point at them.
    assert_eq!(e.cells["C3"].source.as_deref(), Some("= {B3} + 1"));
    assert_eq!(value(&e, "C3"), "13");
    // A range over the block is untouched; a ref into a moved row follows it.
    assert_eq!(e.cells["total"].source.as_deref(), Some("= sum({B2:B5})"));
    assert_eq!(value(&e, "total"), "19");
    // carol was already first, so A2 didn't move and neither did the ref.
    assert_eq!(e.cells["top"].source.as_deref(), Some("= {A2}"));
    assert_eq!(value(&e, "top"), "carol");

    e.sort_rows(None, 2, 2, 5, true, Actor::Human).unwrap();
    let names: Vec<String> = (2..=5).map(|r| value(&e, &a1_name(1, r))).collect();
    assert_eq!(
        names,
        ["dave", "bob", "carol", "alice"],
        "descending, blanks still last"
    );
    // carol moved from row 2 to row 4; the ref followed her.
    assert_eq!(e.cells["top"].source.as_deref(), Some("= {A4}"));
    assert_eq!(value(&e, "top"), "carol");
    // Sorting an already-sorted block is a no-op that doesn't touch disk.
    e.take_changed();
    e.sort_rows(None, 2, 2, 5, true, Actor::Human).unwrap();
    assert!(e.take_changed().is_empty());
    assert!(e.sort_rows(None, 2, 0, 5, true, Actor::Human).is_err());
}

#[tokio::test]
async fn formats_persist_survive_writes_and_move() {
    let (d, mut e) = engine();
    let bold = Format {
        bold: true,
        align: Some(crate::model::Align::Right),
        num: Some(crate::model::NumFormat::Currency),
        ..Format::default()
    };
    e.set_format("B2", Some(bold.clone())).unwrap();
    assert_eq!(
        e.cells["B2"].format,
        Some(bold.clone()),
        "formatting an empty cell creates it"
    );
    e.set("B2", "3500", Actor::Agent("a1".into())).unwrap();
    assert_eq!(
        e.cells["B2"].format,
        Some(bold.clone()),
        "a value write keeps the format"
    );
    e.insert(None, Axis::Rows, 1, 1, Actor::Human).unwrap();
    assert_eq!(
        e.cells["B3"].format,
        Some(bold.clone()),
        "the format moves with the cell"
    );
    let store = Store::open(d.path()).unwrap();
    assert_eq!(store.get("B3").unwrap().unwrap().format, Some(bold));
    // Clearing to plain drops it; vocabulary outside the enums doesn't parse.
    e.set_format("B3", Some(Format::default())).unwrap();
    assert!(e.cells["B3"].format.is_none());
    assert!(serde_json::from_str::<Format>(r#"{"num": "hex"}"#).is_err());
}

#[test]
fn remap_refs_keeps_anchors_suffixes_and_shell_syntax() {
    let down = |c: u32, r: u32| Some((c, if r >= 2 { r + 1 } else { r }));
    let corner = |c: u32, r: u32, _: bool, _: bool| (c, if r >= 2 { r + 1 } else { r });
    assert_eq!(
        remap_refs(
            "$ cat {A2:file} {$B$2} {A1} ${HOME} {A2.code} {A1:C5}",
            &down,
            &corner,
            &|_| true
        ),
        "$ cat {A3:file} {$B$3} {A1} ${HOME} {A3.code} {A1:C6}"
    );
}

#[test]
fn dependents_index_tracks_installs_rewrites_and_removals() {
    let (d, mut e) = engine();
    e.set("a", "1", Actor::Cli).unwrap();
    e.set("b", "= {a}", Actor::Cli).unwrap();
    e.set("c", "= {a} + {b}", Actor::Cli).unwrap();
    assert_eq!(e.dependents_of("a"), ["b", "c"]);
    e.set("c", "= {b}", Actor::Cli).unwrap();
    assert_eq!(e.dependents_of("a"), ["b"]);
    e.set("b", "5", Actor::Cli).unwrap(); // pinned: no deps
    assert!(e.dependents_of("a").is_empty());
    assert_eq!(value(&e, "c"), "5");
    std::fs::remove_file(d.path().join(".recalc/cells/c")).unwrap();
    std::fs::remove_file(d.path().join(".recalc/defs/c")).unwrap();
    e.reconcile("c").unwrap();
    assert!(e.dependents_of("b").is_empty());
}

mod perf_probe {
    // Timing only: `cargo test --release --lib time_structural -- --ignored --nocapture`.
    use super::super::*;
    #[tokio::test]
    #[ignore]
    async fn time_structural_edits_on_1000_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(dir.path()).unwrap();
        let mut entries = Vec::new();
        for r in 2..=1001u32 {
            entries.push((format!("A{r}"), format!("docs/file-{r}.md")));
            entries.push((format!("B{r}"), r.to_string()));
            entries.push((format!("C{r}"), (r * 2).to_string()));
            entries.push((format!("D{r}"), format!("note {r} lorem ipsum")));
            entries.push((format!("E{r}"), format!("= {{B{r}}} + {{C{r}}}")));
        }
        e.set_many(entries, Actor::Cli).unwrap();
        e.drain().await.unwrap();
        let t = std::time::Instant::now();
        e.sort_rows(None, 2, 2, 1001, true, Actor::Human).unwrap();
        eprintln!("sort 1000 rows: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        e.delete(None, Axis::Rows, 2, 100, Actor::Human).unwrap();
        eprintln!("delete 100 rows: {:?}", t.elapsed());
    }
}

mod history_moves {
    use super::super::*;
    #[tokio::test]
    async fn a_sort_or_delete_keeps_each_rows_own_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(dir.path()).unwrap();
        for v in ["1", "2"] {
            for (r, row) in [(2, "a"), (3, "b"), (4, "c")] {
                e.set(&format!("B{r}"), &format!("{row}{v}"), Actor::Cli)
                    .unwrap();
            }
        }
        let history = |e: &Engine, n: &str| -> Vec<String> {
            e.history(n, 10)
                .unwrap()
                .into_iter()
                .map(|(_, v)| v)
                .collect()
        };
        e.sort_rows(None, 2, 2, 4, true, Actor::Human).unwrap();
        // Descending: c, b, a. Each row's past went with it.
        assert_eq!(history(&e, "B2"), ["c1", "c2"]);
        assert_eq!(history(&e, "B3"), ["b1", "b2"]);
        assert_eq!(history(&e, "B4"), ["a1", "a2"]);
        // A deleted row's history goes with it, and the row below moves up
        // with its own.
        e.delete(None, Axis::Rows, 3, 1, Actor::Human).unwrap();
        assert_eq!(history(&e, "B3"), ["a1", "a2"]);
        assert!(
            history(&e, "B4").is_empty(),
            "nothing left at the old last row"
        );
    }
}

#[tokio::test]
async fn a_commands_json_becomes_rows_that_stay_matched_by_key() {
    let (d, mut e) = engine();
    // A cell nothing was written to reads as empty.
    let value = |e: &Engine, n: &str| e.cell(n).map(|c| c.value.clone()).unwrap_or_default();
    let data = d.path().join("prs.json");
    std::fs::write(
        &data,
        r#"[{"n":7,"t":"fix login"},{"n":9,"t":"add cache"}]"#,
    )
    .unwrap();
    e.set("A1", "PR", Actor::Human).unwrap();
    e.set(
        "prs",
        "$ cat prs.json # rows: A2 key=n cols=n,t gone=D",
        Actor::Human,
    )
    .unwrap();
    e.drain().await.unwrap();
    assert_eq!(
        (value(&e, "A2"), value(&e, "B2")),
        ("7".into(), "fix login".into())
    );
    assert_eq!(value(&e, "A3"), "9");
    // An answer column beside the rows, and a mark on one answer.
    e.set("C2", "= {A2} * 10", Actor::Human).unwrap();
    e.set("C3", "= {A3} * 10", Actor::Human).unwrap();
    // A blank in the row above doesn't stop the fill: F is past an empty E.
    e.set("F3", "= {A3} + 1", Actor::Human).unwrap();
    e.set("note", "keep", Actor::Human).unwrap();

    // 9 changes title and moves first, 12 arrives, 7 stays.
    std::fs::write(
        &data,
        r#"[{"n":9,"t":"add LRU cache"},{"n":12,"t":"bump deps"},{"n":7,"t":"fix login"}]"#,
    )
    .unwrap();
    e.trigger("prs", Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "A2"), "7", "rows keep their place");
    assert_eq!(
        value(&e, "B3"),
        "add LRU cache",
        "a field follows its record"
    );
    assert_eq!(
        (value(&e, "A4"), value(&e, "B4")),
        ("12".into(), "bump deps".into())
    );
    assert_eq!(
        value(&e, "C4"),
        "120",
        "a new row takes the formulas of the row above"
    );
    assert_eq!(value(&e, "F4"), "13", "past a blank cell too");

    // 7 leaves: its row stays, marked, with its answer.
    std::fs::write(
        &data,
        r#"[{"n":9,"t":"add LRU cache"},{"n":12,"t":"bump deps"}]"#,
    )
    .unwrap();
    e.trigger("prs", Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(
        (value(&e, "A2"), value(&e, "C2"), value(&e, "D2")),
        ("7".into(), "70".into(), "gone".into())
    );
    assert_eq!(value(&e, "D3"), "");
    assert_eq!(
        e.cell("A2").unwrap().format.as_ref().and_then(|f| f.fill),
        Some(crate::model::Fill::Gray)
    );

    // A failed run leaves the rows alone instead of marking them all gone.
    std::fs::remove_file(&data).unwrap();
    e.trigger("prs", Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "D3"), "");
    // Back again: no longer gone.
    std::fs::write(&data, r#"[{"n":7,"t":"fix login"}]"#).unwrap();
    e.trigger("prs", Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "D2"), "");
    assert_eq!(e.cell("A2").unwrap().format, None);
    assert_eq!(value(&e, "D3"), "gone");
}

#[tokio::test]
async fn a_formula_that_stopped_early_still_hears_from_every_cell_it_names() {
    // `and` stops at the first false, and erred on an empty input part-way:
    // its later inputs must still wake it.
    let (_d, mut e) = engine();
    e.set("A", "false", Actor::Human).unwrap();
    e.set("ready", "= and({A}, {B}, {C})", Actor::Human)
        .unwrap();
    assert_eq!(value(&e, "ready"), "false");
    e.set("A", "true", Actor::Human).unwrap(); // reads B, which is empty: an error
    assert!(matches!(
        e.cell("ready").unwrap().state,
        CellState::Error(_)
    ));
    e.set("B", "true", Actor::Human).unwrap();
    e.set("C", "true", Actor::Human).unwrap();
    assert_eq!(value(&e, "ready"), "true");
    assert_eq!(e.cell("ready").unwrap().state, CellState::Ok);
}

#[test]
fn the_driver_resumes_its_conversation_and_a_new_agent_starts_over() {
    let (_d, mut e) = engine();
    e.store.set_driver_session("d-1").unwrap();
    let (cmd, _) = e.driver_resume_info("d-1").unwrap();
    assert!(
        cmd.starts_with("export RECALC_AGENT='driver'; claude --resume 'd-1'"),
        "{cmd}"
    );
    assert!(
        cmd.contains("--append-system-prompt-file"),
        "keeps its role: {cmd}"
    );
    assert!(e.driver_takes_id());
    // Codex can't resume Claude's conversation: switching forgets it.
    e.set_driver("codex", Actor::Human).unwrap();
    assert_eq!(e.driver_session(), None);
    assert!(!e.driver_takes_id(), "Codex picks its own id");
    let (cmd, _) = e.driver_resume_info("c-9").unwrap();
    assert!(
        cmd.contains("codex resume 'c-9'") && cmd.contains("developer_instructions"),
        "{cmd}"
    );
    // Seating the same agent again keeps it.
    e.store.set_driver_session("c-9").unwrap();
    e.set_driver("codex", Actor::Human).unwrap();
    assert_eq!(e.driver_session().as_deref(), Some("c-9"));
}

#[test]
fn a_custom_claude_driver_resumes_as_its_own_template() {
    // FDA Bench's Driver: Opus, with the web and a shell, under its own id.
    let (_d, e) = engine_with_agents(
        "driver = \"mine\"\n[agents]\nmine = \"claude --model claude-opus-5-5 --session-id {session} --mcp-config {mcp} --append-system-prompt-file {prompt} --allowedTools WebFetch\"\nbare = \"claude {task}\"\nother = \"tool {task}\"\n[resume]\nother = \"tool --again {session}\"\n",
    );
    // Only templates that can reach the board are offered for the seat.
    assert_eq!(e.driver_choices(), ["claude", "codex", "pi", "mine"]);
    let (cmd, _) = e.driver_resume_info("d-1").unwrap();
    assert!(
        cmd.starts_with("export RECALC_AGENT='driver'; claude --model claude-opus-5-5 --resume 'd-1' --mcp-config "),
        "its model, tools and board connection carry over: {cmd}"
    );
    assert!(
        cmd.contains("--allowedTools WebFetch") && !cmd.contains("--session-id"),
        "{cmd}"
    );
    // One that names no conversation can't be resumed unless told how.
    let (_d, e) = engine_with_agents("driver = \"bare\"\n[agents]\nbare = \"claude {task}\"\n");
    assert!(e
        .driver_resume_info("d-1")
        .unwrap_err()
        .contains("[resume]"));
    let (_d, e) = engine_with_agents(
        "driver = \"other\"\n[agents]\nother = \"tool {task}\"\n[resume]\nother = \"tool --again {session}\"\n",
    );
    assert!(e
        .driver_resume_info("d-1")
        .unwrap()
        .0
        .ends_with("tool --again 'd-1'"));
}

#[tokio::test]
async fn a_check_that_reads_its_watched_file_runs_once() {
    // On Linux, opening a file is an event too; it must not count as a change.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "hello\n").unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set("c", "$ cat note.txt # watch: note.txt", Actor::Cli)
        .unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "c"), "hello");
    e.pump(&mut watch).unwrap();
    e.drain().await.unwrap();
    let ran_at = e.cells["c"].provenance.at;
    for _ in 0..30 {
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        e.cells["c"].provenance.at, ran_at,
        "reading the file re-ran the check"
    );
    // A real change still re-runs it.
    std::fs::write(dir.path().join("note.txt"), "changed\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "c") != "changed" {
        assert!(
            std::time::Instant::now() < deadline,
            "the change never re-ran it"
        );
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_cell_agent_answering_in_its_own_cell_keeps_the_prompt() {
    let (_d, mut e) = engine();
    e.set("B2", "@codex('Pick a number')", Actor::Cli).unwrap();
    e.set("C2", "= {B2}", Actor::Cli).unwrap();
    // A live session's agent answers in its own cell.
    e.set_noted("B2", "9371", Actor::Agent("B2".into()), None)
        .unwrap();
    let b2 = e.cell("B2").unwrap();
    assert_eq!(b2.value, "9371");
    assert_eq!(b2.kind, CellKind::AgentCall, "still an agent cell");
    assert_eq!(b2.source.as_deref(), Some("@codex('Pick a number')"));
    assert_eq!(b2.state, CellState::Ok);
    assert_eq!(value(&e, "C2"), "9371", "what reads it sees the answer");
    // It's kept on disk too: a reopened board still has the prompt.
    let again = Engine::open(_d.path()).unwrap();
    assert_eq!(
        again.cell("B2").unwrap().source.as_deref(),
        Some("@codex('Pick a number')")
    );
    assert_eq!(again.cell("B2").unwrap().value, "9371");
    // A person typing a value over it still replaces the prompt.
    e.set("B2", "42", Actor::Human).unwrap();
    assert_eq!(e.cell("B2").unwrap().kind, CellKind::Literal);
}

#[test]
fn a_live_session_is_told_to_set_its_answer_and_a_run_to_print_it() {
    let (_d, mut e) = engine();
    e.set("B2", "@codex('Pick a number')", Actor::Cli).unwrap();
    let call = agent::parse(def_body("@codex('Pick a number')")).unwrap();
    let live = e.agent_task("B2", &call, true);
    let run = e.agent_task("B2", &call, false);
    assert!(
        live.contains("what you print isn't saved") && live.contains("set_cell, name=\"B2\""),
        "{live}"
    );
    assert!(run.contains("what you print becomes its value"), "{run}");
    assert!(!live.contains("{answer}") && !run.contains("{answer}"));
}

#[tokio::test]
async fn a_sessions_answer_while_its_cell_runs_elsewhere_doesnt_leave_it_computing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".recalc")).unwrap();
    std::fs::write(
        dir.path().join(".recalc/config.toml"),
        "[agents]\nslow = \"sleep 0.5 && echo late\"\n",
    )
    .unwrap();
    let mut app = Engine::open(dir.path()).unwrap();
    let mut watch = app.store.watch().unwrap();
    app.set("B2", "@slow('go')", Actor::Cli).unwrap();
    app.trigger("B2", Actor::Human).unwrap();
    assert_eq!(app.cell("B2").unwrap().state, CellState::Computing);
    // The cell's live session answers through its MCP server, another process.
    let mut session = Engine::open(dir.path()).unwrap();
    session
        .set_noted("B2", "9371", Actor::Agent("B2".into()), None)
        .unwrap();
    for _ in 0..40 {
        app.pump(&mut watch).unwrap();
        if app.cell("B2").unwrap().value == "9371" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    app.drain().await.unwrap();
    let b2 = app.cell("B2").unwrap();
    assert_eq!(b2.value, "9371", "the session's answer stands");
    assert_eq!(b2.state, CellState::Ok, "not left computing");
    assert_eq!(b2.source.as_deref(), Some("@slow('go')"));
    assert!(
        !dir.path().join(".recalc/running/B2").exists(),
        "no run marker left behind"
    );
}

#[tokio::test]
async fn a_value_written_elsewhere_over_a_queued_agent_cell_isnt_left_queued() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".recalc")).unwrap();
    std::fs::write(
        dir.path().join(".recalc/config.toml"),
        "max_agents = 1\n[agents]\nslow = \"sleep 0.6 && echo late\"\n",
    )
    .unwrap();
    let mut app = Engine::open(dir.path()).unwrap();
    let mut watch = app.store.watch().unwrap();
    for n in ["K2", "K3", "K5"] {
        app.set(n, "@slow('review')", Actor::Cli).unwrap();
    }
    // The driver's MCP server, another process, queues the column, then
    // fills it with values.
    let mut driver = Engine::open(dir.path()).unwrap();
    for n in ["K2", "K3", "K5"] {
        driver.trigger(n, Actor::Agent("driver".into())).unwrap();
    }
    for _ in 0..20 {
        app.pump(&mut watch).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    driver
        .set("K3", "no", Actor::Agent("driver".into()))
        .unwrap();
    driver
        .set("K5", "no", Actor::Agent("driver".into()))
        .unwrap();
    for _ in 0..40 {
        app.pump(&mut watch).unwrap();
        if app.cell("K5").is_some_and(|c| c.value == "no") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    app.drain().await.unwrap();
    for n in ["K3", "K5"] {
        let c = app.cell(n).unwrap();
        assert_eq!(c.value, "no", "{n}");
        assert!(
            !matches!(c.state, CellState::Queued | CellState::Computing),
            "{n} is a value now, not {:?}",
            c.state
        );
    }
}

#[test]
fn highlights_match_values_and_formats_refuse_what_cant_display() {
    use crate::model::{Fill, Format, Highlight, When};
    let rule = |when, value: &str| Highlight {
        when,
        value: value.into(),
        fill: Fill::Red,
    };
    assert!(rule(When::Is, "fail").matches(" FAIL "));
    assert!(!rule(When::Is, "fail").matches("failed"));
    assert!(rule(When::Contains, "error").matches("Parse error at line 3"));
    assert!(
        !rule(When::Contains, "").matches("anything"),
        "an empty contains matches nothing"
    );
    assert!(rule(When::Above, "90").matches("$1,000"));
    assert!(
        !rule(When::Below, "0.5").matches("25%"),
        "25 is not below 0.5"
    );
    assert!(!rule(When::Above, "90").matches("n/a"));
    let f = Format {
        fill: Some(Fill::Gray),
        highlight: vec![
            rule(When::Is, "fail"),
            Highlight {
                when: When::Contains,
                value: "fail".into(),
                fill: Fill::Yellow,
            },
        ],
        ..Format::default()
    };
    assert_eq!(f.fill_for("fail"), Some(Fill::Red), "the first match wins");
    assert_eq!(f.fill_for("failed twice"), Some(Fill::Yellow));
    assert_eq!(
        f.fill_for("pass"),
        Some(Fill::Gray),
        "no match: the fixed fill"
    );
    let (_d, mut e) = engine();
    let mut bad = |f: Format| e_set(&mut e, f);
    fn e_set(e: &mut Engine, f: Format) -> String {
        e.set_format("A1", Some(f)).unwrap_err().to_string()
    }
    assert!(bad(Format {
        currency: Some("euro".into()),
        ..Format::default()
    })
    .contains("ISO code"));
    assert!(bad(Format {
        decimals: Some(11),
        ..Format::default()
    })
    .contains("0 to 10"));
    assert!(bad(Format {
        highlight: vec![rule(When::Above, "lots")],
        ..Format::default()
    })
    .contains("needs a number"));
}

#[tokio::test]
async fn the_driver_answers_an_agent_cell_and_its_prompt_stays() {
    let (_d, mut e) = engine();
    e.set("B2", "@claude('Governing law?')", Actor::Cli)
        .unwrap();
    e.set("C2", "= {B2}", Actor::Cli).unwrap();
    e.answer_cell(
        "B2",
        "Delaware",
        Actor::Agent("driver".into()),
        Some("from §12".into()),
    )
    .unwrap();
    let b2 = e.cell("B2").unwrap();
    assert_eq!(b2.value, "Delaware");
    assert_eq!(
        b2.source.as_deref(),
        Some("@claude('Governing law?')"),
        "the prompt stays"
    );
    assert_eq!(b2.provenance.by, Actor::Agent("driver".into()));
    assert_eq!(b2.state, CellState::Ok);
    assert_eq!(value(&e, "C2"), "Delaware");
    // Not for plain cells or sources; a verified cell takes it as a suggestion.
    e.set("D2", "plain", Actor::Cli).unwrap();
    assert!(e
        .answer_cell("D2", "x", Actor::Agent("driver".into()), None)
        .unwrap_err()
        .to_string()
        .contains("isn't an agent cell"));
    assert!(e
        .answer_cell("B2", "= 1", Actor::Agent("driver".into()), None)
        .unwrap_err()
        .to_string()
        .contains("an answer is a value"));
    e.set_mark(
        "B2",
        Some(crate::model::Mark {
            kind: crate::model::MarkKind::Verified,
            note: None,
            by: Actor::Human,
            at: 1,
        }),
    )
    .unwrap();
    e.answer_cell("B2", "New York", Actor::Agent("driver".into()), None)
        .unwrap();
    assert_eq!(
        e.cell("B2").unwrap().value,
        "Delaware",
        "verified: held as a suggestion"
    );
    assert_eq!(e.cell("B2").unwrap().pending_suggestions.len(), 1);
    // The next write is an ordinary one again: answering marks one write only.
    e.set("E2", "@claude('x')", Actor::Cli).unwrap();
    e.set("E2", "plain now", Actor::Agent("driver".into()))
        .unwrap();
    assert_eq!(e.cell("E2").unwrap().kind, CellKind::Literal);
}

#[tokio::test]
async fn a_shut_down_engine_never_takes_its_board_back() {
    // The window shuts a board's engine before moving the folder, and its
    // pump can reach that engine once more. Taking over the board it let go
    // of re-ran a stopped command and recreated the old folder.
    let (d, mut e) = engine();
    let mut watch = e.store.watch().unwrap();
    e.set("slow", "$ sleep 30", Actor::Human).unwrap();
    assert_eq!(e.cells["slow"].state, CellState::Computing);
    e.shutdown();
    assert_eq!(e.cells["slow"].state, CellState::Stale);
    // Past the election interval, when a passive engine would take over.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    e.pump(&mut watch).unwrap();
    assert!(!e.is_active(), "it doesn't claim the board again");
    assert_eq!(e.cells["slow"].state, CellState::Stale, "nothing re-runs");
    assert!(
        !d.path().join(".recalc/running/slow").exists(),
        "no run marker written"
    );
}

#[test]
fn an_edited_config_applies_without_reopening_and_a_broken_one_keeps_the_last() {
    let (d, mut e) = engine();
    let mut watch = e.store.watch().unwrap();
    assert!(!e.config().agents.contains_key("mine"));
    std::fs::write(
        d.path().join(".recalc/config.toml"),
        "max_agents = 2\n[agents]\nmine = \"echo {task}\"\n",
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    e.pump(&mut watch).unwrap();
    assert_eq!(e.config().max_agents, 2);
    assert!(
        e.config().agents.contains_key("mine"),
        "picked up without reopening"
    );
    assert!(e.activity().any(|a| a.text.contains("settings reloaded")));

    std::fs::write(d.path().join(".recalc/config.toml"), "max_agents = [oops\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    e.pump(&mut watch).unwrap();
    assert_eq!(
        e.config().max_agents,
        2,
        "a broken file keeps what was in force"
    );
    assert!(e
        .activity()
        .any(|a| a.text.contains("config.toml didn't load")));

    // The engine's own writes aren't news.
    std::fs::write(d.path().join(".recalc/config.toml"), "max_agents = 2\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    e.pump(&mut watch).unwrap();
    let reloads = |e: &Engine| {
        e.activity()
            .filter(|a| a.text.contains("settings reloaded"))
            .count()
    };
    let before = reloads(&e);
    e.set_budget(Some(1.0), Actor::Human).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    e.pump(&mut watch).unwrap();
    assert_eq!(
        reloads(&e),
        before,
        "a budget set from the app isn't logged as a reload"
    );
    assert_eq!(e.config().budget_usd, Some(1.0));
}

#[test]
fn an_agent_without_the_board_tools_isnt_told_to_use_them() {
    let (_d, mut e) = engine_with_agents(
        "[agents]\nplain = \"mine -p {task}\"\nwired = \"mine --mcp {mcp} {task}\"\n",
    );
    e.set("A1", "Status", Actor::Human).unwrap();
    for (agent, wants_tools) in [("plain", false), ("wired", true), ("claude", true)] {
        let cell = format!("{agent}_cell");
        e.set(&cell, &format!("@{agent}('Report status.')"), Actor::Human)
            .unwrap();
        e.set_answer_type(
            &cell,
            Some(crate::model::AnswerType {
                kind: crate::model::AnswerKind::Choice,
                options: vec!["done".into(), "partial".into(), "failed".into()],
                strict: true,
                allows_not_found: true,
            }),
        )
        .unwrap();
        let call = crate::agent::parse(&format!("{agent}('Report status.')")).unwrap();
        let task = e.agent_task(&cell, &call, false);
        for word in ["cite_cell", "get_cell"] {
            assert_eq!(
                task.contains(word),
                wants_tools,
                "{agent}: {word} in {task:?}"
            );
        }
        assert!(
            task.contains("print only the answer"),
            "{agent}: still told how to answer"
        );
        assert!(
            task.contains("exactly one of: done, partial, failed"),
            "{agent}: still told the answer type: {task:?}"
        );
    }
}

#[tokio::test]
async fn model_on_a_call_reaches_the_agent_and_the_model_that_ran_is_recorded() {
    let tools = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    // A claude that reports the model it was given, as Claude Code's modelUsage does.
    let claude = tools.path().join("claude");
    std::fs::write(&claude, "#!/bin/sh\nread -r line\nm=default\nwhile [ $# -gt 0 ]; do [ \"$1\" = --model ] && m=\"$2\"; shift; done\nprintf '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ran on %s\",\"total_cost_usd\":0.01,\"num_turns\":1,\"usage\":{\"input_tokens\":1,\"output_tokens\":1},\"modelUsage\":{\"%s\":{\"costUSD\":0.01}}}\\n' \"$m\" \"$m\"\n").unwrap();
    // A codex that doesn't say which model it ran.
    let codex = tools.path().join("codex");
    std::fs::write(&codex, "#!/bin/sh\ncat >/dev/null\nm=default\nwhile [ $# -gt 0 ]; do [ \"$1\" = --model ] && m=\"$2\"; shift; done\nprintf '{\"type\":\"thread.started\",\"thread_id\":\"t1\"}\\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"ran on %s\"}}\\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\\n' \"$m\"\n").unwrap();
    for f in [&claude, &codex] {
        std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (_d, mut e) = engine_with_agents(&format!(
        "[paths]\nclaude = \"{}\"\ncodex = \"{}\"\n[models]\nfast = \"claude-haiku-5-5\"\n",
        claude.display(),
        codex.display()
    ));
    e.set("B2", "@claude('Say hi.', model='fast')", Actor::Human)
        .unwrap();
    e.set(
        "B3",
        "@codex('Say hi.', model='gpt-5.6-luna')",
        Actor::Human,
    )
    .unwrap();
    e.set("B4", "@claude('Say hi.')", Actor::Human).unwrap();
    e.set("C2", "= {B2.model}", Actor::Human).unwrap();
    for n in ["B2", "B3", "B4"] {
        e.trigger(n, Actor::Human).unwrap();
    }
    e.drain().await.unwrap();
    assert_eq!(
        value(&e, "B2"),
        "ran on claude-haiku-5-5",
        "the alias resolved and reached claude"
    );
    assert_eq!(
        e.cells["B2"].usage.as_ref().unwrap().model.as_deref(),
        Some("claude-haiku-5-5")
    );
    assert_eq!(
        value(&e, "C2"),
        "claude-haiku-5-5",
        "a formula reads it as .model"
    );
    assert_eq!(value(&e, "B3"), "ran on gpt-5.6-luna");
    assert_eq!(
        e.cells["B3"].usage.as_ref().unwrap().model.as_deref(),
        Some("gpt-5.6-luna"),
        "an agent that doesn't report its model is recorded with the one asked for"
    );
    assert_eq!(value(&e, "B4"), "ran on default", "no model=, no flag");
}

#[tokio::test]
async fn a_watch_of_several_globs_reruns_on_any_of_them() {
    // `# watch: a b` was one glob with a space in it: it matched nothing and
    // the cell silently never re-ran.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("dossiers/C01")).unwrap();
    std::fs::write(dir.path().join("dossiers/C01/a.md"), "one\n").unwrap();
    std::fs::write(dir.path().join("allow.txt"), "x\n").unwrap();
    let mut e = Engine::open(dir.path()).unwrap();
    let mut watch = e.store.watch().unwrap();
    e.set(
        "check",
        "$ cat dossiers/C01/a.md allow.txt | wc -l | tr -d ' ' # watch: dossiers/C01/** allow.txt",
        Actor::Cli,
    )
    .unwrap();
    e.drain().await.unwrap();
    assert_eq!(value(&e, "check"), "2");
    e.pump(&mut watch).unwrap();
    // A change to the second glob's file re-runs it.
    std::fs::write(dir.path().join("allow.txt"), "x\ny\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while value(&e, "check") != "3" {
        assert!(
            std::time::Instant::now() < deadline,
            "stuck at {:?}",
            value(&e, "check")
        );
        e.pump(&mut watch).unwrap();
        e.drain().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // A bad one among several is an error on the cell, saying which rule.
    e.set("bad", "$ true # watch: ok/** /abs/**", Actor::Cli)
        .unwrap();
    match &e.cells["bad"].state {
        CellState::Error(m) => assert!(m.contains("relative to the board"), "{m}"),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[tokio::test]
async fn undoing_a_row_delete_brings_an_agents_answer_back_current() {
    let (_d, mut e) = engine_with_agents("[agents]\nfake = \"echo answered\"\n");
    e.set("A2", "one", Actor::Human).unwrap();
    e.set("A3", "two", Actor::Human).unwrap();
    e.set("B3", "@fake('about {A3}')", Actor::Human).unwrap();
    e.set("C2", "@fake('about {A3}')", Actor::Human).unwrap();
    e.trigger("B3", Actor::Human).unwrap();
    e.trigger("C2", Actor::Human).unwrap();
    e.drain().await.unwrap();
    assert_eq!(e.cells["B3"].state, CellState::Ok);
    let removed = e.delete(None, Axis::Rows, 3, 1, Actor::Human).unwrap();
    assert!(e.cells["C2"].source.as_deref().unwrap().contains("#REF!"));
    e.undelete(None, Axis::Rows, 3, 1, &removed, Actor::Human)
        .unwrap();
    // The inputs came back as they were, so neither answer is out of date.
    assert_eq!(value(&e, "B3"), "answered");
    assert_eq!(e.cells["B3"].state, CellState::Ok);
    assert_eq!(e.cells["C2"].source.as_deref(), Some("@fake('about {A3}')"));
    assert_eq!(e.cells["C2"].state, CellState::Ok);
    // A real change to the input still marks them.
    e.set("A3", "three", Actor::Human).unwrap();
    assert_eq!(e.cells["B3"].state, CellState::Stale);
    assert_eq!(e.cells["C2"].state, CellState::Stale);
}

#[test]
fn a_move_sees_what_another_process_wrote_a_moment_ago() {
    let dir = tempfile::tempdir().unwrap();
    let mut window = Engine::open(dir.path()).unwrap();
    let mut other = Engine::open(dir.path()).unwrap();
    window.set("A2", "one", Actor::Human).unwrap();
    window.set("B3", "= len({A2})", Actor::Human).unwrap();
    // `other` never pumped its watch: its mirror has neither cell.
    assert!(!other.cells.contains_key("B3"));
    other.insert(None, Axis::Rows, 2, 1, Actor::Cli).unwrap();
    let moved = other
        .store
        .get("B4")
        .unwrap()
        .expect("the cell moved, not lost");
    assert_eq!(moved.source.as_deref(), Some("= len({A3})"));
    assert_eq!(other.store.get("A3").unwrap().unwrap().value, "one");
    assert!(other.store.get("B3").unwrap().is_none());
}

#[test]
fn a_dead_sessions_status_doesnt_block_a_move() {
    let (_d, mut e) = engine_with_agents("[agents]\nfake = \"echo answered\"\n");
    e.set("B2", "@fake('x')", Actor::Human).unwrap();
    e.set("B2_status", "ended", Actor::System).unwrap();
    assert_eq!(e.live_session(), None);
    e.insert(None, Axis::Rows, 1, 1, Actor::Human).unwrap();
    assert!(e.cells.contains_key("B3"));
    if !crate::session::available() {
        // Without tmux nothing can vouch for the session, so the status stands.
        e.set("B3_status", "running", Actor::System).unwrap();
        assert_eq!(e.live_session().as_deref(), Some("B3"));
        let err = e.insert(None, Axis::Rows, 1, 1, Actor::Human).unwrap_err();
        assert!(err.to_string().contains("live session"), "{err}");
    }
}
