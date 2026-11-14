use std::process::Command;

fn run_cli(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_ctrlb-decompose"))
        .args(args)
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

#[test]
fn test_human_output_sample_log() {
    let (stdout, _, success) = run_cli(&["--no-banner", "tests/fixtures/sample.log"]);
    assert!(success);
    assert!(stdout.contains("Pattern #1"));
    assert!(stdout.contains("occurrences"));
    assert!(stdout.contains("Variables:"));
}

#[test]
fn test_llm_output_sample_log() {
    let (stdout, _, success) = run_cli(&["--llm", "tests/fixtures/sample.log"]);
    assert!(success);
    assert!(stdout.contains("## Log Analysis:"));
    assert!(stdout.contains("### Patterns"));
}

#[test]
fn test_json_output_valid() {
    let (stdout, _, success) = run_cli(&["--json", "tests/fixtures/sample.log"]);
    assert!(success);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("invalid JSON output");
    assert_eq!(parsed["version"], "0.1.0");
    assert!(parsed["summary"]["total_lines"].as_u64().unwrap() > 0);
    assert!(parsed["patterns"].as_array().unwrap().len() > 0);
}

#[test]
fn test_top_flag_limits_patterns() {
    let (stdout, _, success) = run_cli(&["--json", "--top", "1", "tests/fixtures/sample.log"]);
    assert!(success);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("invalid JSON");
    assert_eq!(parsed["patterns"].as_array().unwrap().len(), 1);
}

#[test]
fn test_context_includes_examples() {
    let (stdout, _, success) = run_cli(&["--json", "--context", "2", "tests/fixtures/sample.log"]);
    assert!(success);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("invalid JSON");
    let patterns = parsed["patterns"].as_array().unwrap();
    // At least the first pattern should have example lines
    let examples = patterns[0]["example_lines"].as_array().unwrap();
    assert!(examples.len() <= 2);
    assert!(!examples.is_empty());
}

#[test]
fn test_quiet_suppresses_stderr() {
    let (_, stderr, success) = run_cli(&["-q", "tests/fixtures/sample.log"]);
    assert!(success);
    assert!(
        !stderr.contains("Processed"),
        "stderr should be suppressed with -q"
    );
}

#[test]
fn test_stdin_input() {
    let output = Command::new(env!("CARGO_BIN_EXE_ctrlb-decompose"))
        .args(["--no-banner", "-q", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                writeln!(stdin, "INFO request completed in 45ms")?;
                writeln!(stdin, "INFO request completed in 32ms")?;
                writeln!(stdin, "ERROR timeout after 5000ms")?;
            }
            child.wait_with_output()
        })
        .expect("failed to run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("Pattern #"));
}

fn analyze_json(input: &str) -> serde_json::Value {
    serde_json::from_str(&analyze_output(
        input,
        ctrlb_decompose::types::OutputMode::Json,
    ))
    .unwrap()
}

fn analyze_output(input: &str, output_mode: ctrlb_decompose::types::OutputMode) -> String {
    use ctrlb_decompose::{format::format_output, process_log_text, types::FormatOptions};
    let opts = FormatOptions {
        top: 20,
        context: 2,
        no_color: true,
        no_banner: true,
        output_mode,
    };
    let analysis = process_log_text(input, &opts);
    format_output(&analysis.store, &opts, &analysis.scores)
}

fn assert_slots(pattern: &serde_json::Value, expected: &[(&str, &[&str])]) {
    let slots = pattern["variables"].as_array().unwrap();
    assert_eq!(slots.len(), expected.len());
    for (index, (slot, (var_type, values))) in slots.iter().zip(expected).enumerate() {
        assert_eq!(slot["slot"], index);
        assert_eq!(slot["type"], *var_type);
        let mut actual: Vec<_> = slot["top_values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| {
                assert_eq!(value["count"], 1);
                value["value"].as_str().unwrap()
            })
            .collect();
        actual.sort_unstable();
        let mut expected_values = values.to_vec();
        expected_values.sort_unstable();
        assert_eq!(actual, expected_values, "slot {index}");
        if *var_type == "Integer" || *var_type == "Float" {
            let numbers: Vec<f64> = values.iter().map(|value| value.parse().unwrap()).collect();
            let numeric = &slot["numeric"];
            let mean = numbers.iter().sum::<f64>() / numbers.len() as f64;
            assert_eq!(numeric["mean"], (mean * 10.0).round() / 10.0);
            assert_eq!(
                numeric["min"],
                numbers.iter().copied().reduce(f64::min).unwrap()
            );
            assert_eq!(
                numeric["max"],
                numbers.iter().copied().reduce(f64::max).unwrap()
            );
        } else {
            assert!(slot.get("numeric").is_none(), "slot {index}: {slot}");
        }
    }
}

#[test]
fn test_path_and_elapsed_slots_through_library() {
    let output = analyze_json("request path=/foo/1 elapsed=10\nrequest path=/bar/2 elapsed=20\n");
    assert_eq!(output["summary"]["pattern_count"], 1);
    let pattern = &output["patterns"][0];
    assert_eq!(pattern["template"], "request <*> elapsed=<*>");
    assert_eq!(pattern["count"], 2);
    assert_slots(
        pattern,
        &[
            ("String", &["path=/foo/1", "path=/bar/2"]),
            ("Integer", &["10", "20"]),
        ],
    );
}

#[test]
fn test_new_wildcard_does_not_shift_existing_slots() {
    let output = analyze_json("INFO start accepted count=10\nINFO start rejected count=20\n");
    assert_eq!(output["summary"]["pattern_count"], 1);
    let pattern = &output["patterns"][0];
    assert_eq!(pattern["template"], "INFO start <*> count=<*>");
    assert_eq!(pattern["count"], 2);
    assert_slots(
        pattern,
        &[
            ("String", &["accepted", "rejected"]),
            ("Integer", &["10", "20"]),
        ],
    );
    assert_eq!(pattern["variables"][1]["label"], "count");
}

#[test]
fn test_widening_collapses_multiple_slots_and_adds_new_slots() {
    let output = analyze_json(concat!(
        "INFO route=/foo/1/2 action accepted elapsed=10 end\n",
        "INFO route=/bar/3/4 action rejected elapsed=20 tail\n",
    ));
    assert_eq!(output["summary"]["pattern_count"], 1);
    let pattern = &output["patterns"][0];
    assert_eq!(pattern["template"], "INFO <*> action <*> elapsed=<*> <*>");
    assert_eq!(pattern["count"], 2);
    assert_slots(
        pattern,
        &[
            ("String", &["route=/foo/1/2", "route=/bar/3/4"]),
            ("String", &["accepted", "rejected"]),
            ("Integer", &["10", "20"]),
            ("String", &["end", "tail"]),
        ],
    );
}

#[test]
fn test_embedded_wildcard_labels_through_library() {
    let input = "INFO method=GET completed in 42\n";
    let output = analyze_json(input);
    let pattern = &output["patterns"][0];
    assert_eq!(pattern["template"], "INFO method=<*> completed in <*>");
    assert_slots(pattern, &[("String", &["GET"]), ("Integer", &["42"])]);
    assert_eq!(pattern["variables"][0]["label"], "method");
    assert_eq!(pattern["variables"][1]["label"], "duration_ms");

    let llm = analyze_output(input, ctrlb_decompose::types::OutputMode::Llm);
    assert!(
        llm.contains("INFO method={method} completed in {duration_ms}"),
        "{llm}"
    );
    assert!(llm.contains("method: GET(100%)"), "{llm}");
    assert!(llm.contains("duration_ms: mean=42"), "{llm}");
}

#[test]
fn test_multiple_embedded_wildcards_do_not_shift_later_labels() {
    let input = "INFO metrics=10,20 completed in 42\n";
    let output = analyze_json(input);
    let variables = &output["patterns"][0]["variables"];
    assert_eq!(variables[0]["label"], "metrics");
    assert_eq!(variables[2]["label"], "duration_ms");
    let llm = analyze_output(input, ctrlb_decompose::types::OutputMode::Llm);
    assert!(
        llm.contains("INFO metrics={metrics},{n2} completed in {duration_ms}"),
        "{llm}"
    );
}

#[test]
fn test_interleaved_wildcard_slots_through_library() {
    // Literal wildcard tokens seed a cluster without changing its slot layout.
    // Subsequent lines exercise decoding inside all three wildcard positions.
    let output = analyze_json(concat!(
        "<*> metrics=10,deadbeef,2.50 <*> more=30,0xface,4.50 <*>\n",
        "route(/foo/1,0xabc1,1.25) metrics=20,cafebabe,7.50 phase(/v4/44,deadbeef,8.75) more=40,0xbeef,9.50 end(/v5/55,cd34,10.75)\n",
    ));
    assert_eq!(output["summary"]["pattern_count"], 1);
    let pattern = &output["patterns"][0];
    assert_eq!(
        pattern["template"],
        "<*> metrics=<*>,<*>,<*> <*> more=<*>,<*>,<*> <*>"
    );
    assert_eq!(pattern["count"], 2);
    assert_slots(
        pattern,
        &[
            ("String", &["<*>", "route(/foo/1,0xabc1,1.25)"]),
            ("Integer", &["10", "20"]),
            ("HexID", &["deadbeef", "cafebabe"]),
            ("Float", &["2.50", "7.50"]),
            ("String", &["<*>", "phase(/v4/44,deadbeef,8.75)"]),
            ("Integer", &["30", "40"]),
            ("HexID", &["0xface", "0xbeef"]),
            ("Float", &["4.50", "9.50"]),
            ("String", &["<*>", "end(/v5/55,cd34,10.75)"]),
        ],
    );
}

#[test]
fn test_path_slots_at_line_boundaries_through_library() {
    // Drain routes by the first tokens, so seed boundary wildcards explicitly.
    let output = analyze_json(concat!(
        "<*> INFO metrics=0,facefeed,1.25 done <*>\n",
        "/foo/1 INFO metrics=10,deadbeef,2.50 done /tail/3.25/ab12\n",
        "/bar/2 INFO metrics=20,cafebabe,7.50 done /end/4.75/cd34\n",
    ));
    assert_eq!(output["summary"]["pattern_count"], 1);
    let pattern = &output["patterns"][0];
    assert_eq!(pattern["template"], "<*> INFO metrics=<*>,<*>,<*> done <*>");
    assert_eq!(pattern["count"], 3);
    assert_slots(
        pattern,
        &[
            ("String", &["<*>", "/foo/1", "/bar/2"]),
            ("Integer", &["0", "10", "20"]),
            ("HexID", &["facefeed", "deadbeef", "cafebabe"]),
            ("Float", &["1.25", "2.50", "7.50"]),
            ("String", &["<*>", "/tail/3.25/ab12", "/end/4.75/cd34"]),
        ],
    );
}
