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
