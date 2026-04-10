use std::process::Command;

fn run_cli(args: &[&str]) -> (String, String, bool) {
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--"])
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
    let (stdout, _, success) = run_cli(&[
        "--json",
        "--context",
        "2",
        "tests/fixtures/sample.log",
    ]);
    assert!(success);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("invalid JSON");
    let patterns = parsed["patterns"].as_array().unwrap();
    // At least the first pattern should have example lines
    let examples = patterns[0]["example_lines"].as_array().unwrap();
    assert!(examples.len() <= 2);
    assert!(!examples.is_empty());
}

#[test]
fn test_llm_mode_suppresses_banner_by_default() {
    let (_, stderr, success) = run_cli(&["--llm", "tests/fixtures/sample.log"]);
    assert!(success);
    assert!(
        !stderr.contains("Powered by CtrlB"),
        "LLM mode should suppress banner by default, got stderr: {}",
        stderr
    );
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
fn test_source_label_human() {
    let (stdout, _, success) = run_cli(&[
        "--source-label", "pod-a",
        "tests/fixtures/sample.log",
    ]);
    assert!(success);
    assert!(
        stdout.contains("pod-a"),
        "Human output should include source label, got: {}",
        stdout
    );
}

#[test]
fn test_source_label_llm() {
    let (stdout, _, success) = run_cli(&[
        "--llm", "--source-label", "pod-a",
        "tests/fixtures/sample.log",
    ]);
    assert!(success);
    assert!(
        stdout.contains("(pod-a)"),
        "LLM output should include source label in header, got: {}",
        stdout
    );
}

#[test]
fn test_source_label_json() {
    let (stdout, _, success) = run_cli(&[
        "--json", "--source-label", "pod-a",
        "tests/fixtures/sample.log",
    ]);
    assert!(success);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("invalid JSON");
    assert_eq!(parsed["summary"]["source_label"], "pod-a");
}

#[test]
fn test_stdin_input() {
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--", "--no-banner", "-q", "-"])
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

/// Normalize a parsed JSON output for deterministic snapshots:
/// - Sort patterns by id
/// - Sort top_values within each variable by count desc, then value asc
fn normalize_json_output(value: &mut serde_json::Value) {
    if let Some(patterns) = value.get_mut("patterns").and_then(|p| p.as_array_mut()) {
        patterns.sort_by_key(|p| p["id"].as_u64().unwrap_or(0));
        for pattern in patterns.iter_mut() {
            if let Some(vars) = pattern.get_mut("variables").and_then(|v| v.as_array_mut()) {
                for var in vars.iter_mut() {
                    if let Some(tv) = var.get_mut("top_values").and_then(|t| t.as_array_mut()) {
                        tv.sort_by(|a, b| {
                            let ca = a["count"].as_u64().unwrap_or(0);
                            let cb = b["count"].as_u64().unwrap_or(0);
                            cb.cmp(&ca).then_with(|| {
                                a["value"]
                                    .as_str()
                                    .unwrap_or("")
                                    .cmp(b["value"].as_str().unwrap_or(""))
                            })
                        });
                    }
                }
            }
        }
    }
}

#[test]
fn test_json_output_snapshot() {
    let (stdout, _, success) = run_cli(&[
        "--json", "--quiet", "--no-banner", "--top", "3", "--context", "1",
        "tests/fixtures/sample.log",
    ]);
    assert!(success, "CLI should succeed");
    let mut parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("invalid JSON output");
    normalize_json_output(&mut parsed);
    insta::assert_json_snapshot!("json_output", parsed, {
        ".patterns[].example_lines" => insta::dynamic_redaction(|value, _path| {
            let count = match &value {
                insta::internals::Content::Seq(items) => items.len(),
                _ => 0,
            };
            format!("[{} examples]", count)
        }),
        ".patterns[].variables[].top_values" => insta::dynamic_redaction(|value, _path| {
            let count = match &value {
                insta::internals::Content::Seq(items) => items.len(),
                _ => 0,
            };
            format!("[{} values]", count)
        }),
    });
}

#[test]
fn test_llm_output_snapshot() {
    let (stdout, _, success) = run_cli(&[
        "--llm", "--quiet", "--no-banner", "--top", "3",
        "tests/fixtures/sample.log",
    ]);
    assert!(success, "CLI should succeed");
    // Redact non-deterministic parts: variable value lines (pipe-separated groups
    // with percentages) and example lines (reservoir-sampled).
    let redacted: String = stdout
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.contains('|') && trimmed.contains("%)") {
                let indent = &line[..line.len() - trimmed.len()];
                let group_count = trimmed.split(" | ").count();
                format!("{}[{} variable groups redacted]", indent, group_count)
            } else if trimmed.starts_with("e.g. ") {
                let indent = &line[..line.len() - trimmed.len()];
                format!("{}[example redacted]", indent)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("llm_output", redacted);
}

#[test]
fn test_human_output_snapshot() {
    let (stdout, _, success) = run_cli(&[
        "--no-banner", "--no-color", "--quiet", "--top", "3",
        "tests/fixtures/sample.log",
    ]);
    assert!(success, "CLI should succeed");
    // Redact non-deterministic parts and sort pattern blocks for stability.
    // 1. Redact variable value lists (top-k with ties is non-deterministic)
    // 2. Sort pattern blocks by template (patterns with equal counts swap order)
    // 3. Renumber patterns after sorting
    let redacted_lines: Vec<String> = stdout
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(colon_pos) = trimmed.find(':') {
                let label = &trimmed[..colon_pos];
                let is_var_line = matches!(
                    label,
                    "HexID" | "IPv4" | "Duration" | "Integer" | "Float"
                        | "String" | "Enum" | "UUID"
                );
                if is_var_line {
                    let indent = &line[..line.len() - trimmed.len()];
                    let rest = trimmed[colon_pos + 1..].trim();
                    if rest.starts_with("mean=") {
                        return line.to_string();
                    }
                    let value_count = rest.split(", ").count();
                    return format!(
                        "{}{}:    [{} values redacted]",
                        indent, label, value_count
                    );
                }
            }
            line.to_string()
        })
        .collect();

    // Split into pattern blocks and sort by template for deterministic ordering
    let text = redacted_lines.join("\n");
    let mut blocks: Vec<String> = Vec::new();
    let mut current_block = String::new();
    for line in text.lines() {
        if line.starts_with("Pattern #") && !current_block.is_empty() {
            blocks.push(current_block.trim_end().to_string());
            current_block = String::new();
        }
        current_block.push_str(line);
        current_block.push('\n');
    }
    if !current_block.trim().is_empty() {
        blocks.push(current_block.trim_end().to_string());
    }
    // Sort blocks by their template line (second line of each block)
    blocks.sort_by(|a, b| {
        let tmpl_a = a.lines().nth(1).unwrap_or("");
        let tmpl_b = b.lines().nth(1).unwrap_or("");
        tmpl_a.cmp(tmpl_b)
    });
    // Renumber pattern blocks after sorting
    let mut renumbered = String::new();
    for (i, block) in blocks.iter().enumerate() {
        let mut first = true;
        for line in block.lines() {
            if first && line.starts_with("Pattern #") {
                // Replace "Pattern #N" with renumbered version
                if line.split_once(']').or_else(|| line.split_once(')')).is_some() {
                    let after_hash = &line["Pattern #".len()..];
                    let num_end = after_hash.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_hash.len());
                    renumbered.push_str(&format!(
                        "Pattern #{}{}",
                        i + 1,
                        &after_hash[num_end..]
                    ));
                } else {
                    renumbered.push_str(line);
                }
                first = false;
            } else {
                renumbered.push_str(line);
            }
            renumbered.push('\n');
        }
        renumbered.push('\n');
    }
    let redacted = renumbered.trim_end().to_string() + "\n";
    insta::assert_snapshot!("human_output", redacted);
}

#[test]
fn test_sim_threshold_flag() {
    // Very low threshold should merge aggressively (fewer patterns)
    let (stdout_low, _, success_low) = run_cli(&[
        "--json", "--sim-threshold", "0.2",
        "tests/fixtures/sample.log",
    ]);
    assert!(success_low);
    let parsed_low: serde_json::Value = serde_json::from_str(&stdout_low).expect("invalid JSON");
    let count_low = parsed_low["summary"]["pattern_count"].as_u64().unwrap();

    // High threshold should produce more patterns
    let (stdout_high, _, success_high) = run_cli(&[
        "--json", "--sim-threshold", "0.8",
        "tests/fixtures/sample.log",
    ]);
    assert!(success_high);
    let parsed_high: serde_json::Value =
        serde_json::from_str(&stdout_high).expect("invalid JSON");
    let count_high = parsed_high["summary"]["pattern_count"].as_u64().unwrap();

    assert!(
        count_high >= count_low,
        "Higher threshold should produce >= patterns: high={} low={}",
        count_high,
        count_low
    );
}
