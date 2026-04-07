use std::collections::HashMap;

use serde::Serialize;

use crate::anomaly::AnomalyFlags;
use crate::stats::PatternStore;
use crate::types::PatternID;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warn,
    Debug,
    Info,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warn => "WARN",
            Severity::Debug => "DEBUG",
            Severity::Info => "INFO",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PatternScore {
    pub pattern_id: PatternID,
    pub score: f64,
    pub severity: Severity,
    pub keyword_weight: f64,
}

/// Build the keyword -> weight map. All lookups are uppercase.
fn keyword_weights() -> HashMap<&'static str, f64> {
    let mut map = HashMap::new();

    for kw in [
        "ERROR", "FATAL", "PANIC", "CRIT", "CRITICAL", "SEGFAULT", "SIGKILL", "SIGSEGV", "OOM",
        "CRASH", "ABORT", "CORRUPT",
    ] {
        map.insert(kw, 10.0);
    }

    for kw in [
        "WARN", "WARNING", "FAIL", "FAILED", "FAILURE", "TIMEOUT", "TIMED_OUT", "REFUSED",
        "DENIED", "REJECTED", "EXCEPTION", "DEADLOCK", "OVERFLOW", "EXHAUSTED", "UNAVAILABLE",
        "UNREACHABLE", "UNAUTHORIZED", "FORBIDDEN", "RETRY", "RETRYING", "BACKOFF",
    ] {
        map.insert(kw, 5.0);
    }

    for kw in ["DEBUG", "TRACE"] {
        map.insert(kw, 0.1);
    }

    map
}

/// Tokenize a template string on whitespace and delimiter characters.
/// This ensures `[ERROR]`, `level=ERROR`, `ERROR:` etc. produce an `ERROR` token.
fn tokenize_template(template: &str) -> Vec<String> {
    let delimiters = |c: char| -> bool {
        c.is_whitespace()
            || matches!(
                c,
                '[' | ']'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '='
                    | ':'
                    | ';'
                    | ','
                    | '.'
                    | '"'
                    | '\''
                    | '<'
                    | '>'
            )
    };

    template
        .split(delimiters)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_uppercase())
        .collect()
}

/// Max prefix length to scan for severity keywords. Log levels appear in
/// the first ~50-80 chars; scanning further causes false positives when
/// structured data (JSON, Ruby hashes) contains field names like "error".
const KEYWORD_PREFIX_LEN: usize = 100;

/// Match template tokens against keyword weights. Only scans the first
/// `KEYWORD_PREFIX_LEN` characters to avoid matching data field names
/// deep in the template body. Returns the max weight found, or 1.0 if
/// no keywords match.
fn match_keyword_weight(template: &str, weights: &HashMap<&str, f64>) -> f64 {
    let mut end = template.len().min(KEYWORD_PREFIX_LEN);
    while !template.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &template[..end];
    let tokens = tokenize_template(prefix);
    let mut max_weight = 1.0_f64;
    let mut found = false;

    for token in &tokens {
        if let Some(&w) = weights.get(token.as_str()) {
            if !found || w > max_weight {
                max_weight = w;
                found = true;
            }
        }
    }

    if found { max_weight } else { 1.0 }
}

fn severity_from_weight(weight: f64) -> Severity {
    if weight >= 10.0 {
        Severity::Error
    } else if weight >= 5.0 {
        Severity::Warn
    } else if weight <= 0.1 {
        Severity::Debug
    } else {
        Severity::Info
    }
}

pub fn compute_scores(
    store: &PatternStore,
    anomalies: &[AnomalyFlags],
) -> HashMap<PatternID, PatternScore> {
    let weights = keyword_weights();

    // Build anomaly lookup: pattern_id -> max anomaly severity
    let mut anomaly_severity: HashMap<PatternID, f64> = HashMap::new();
    for flags in anomalies {
        let max_sev = flags
            .anomalies
            .iter()
            .map(|a| a.severity())
            .fold(0.0_f64, f64::max);
        anomaly_severity.insert(flags.pattern_id, max_sev);
    }

    let mut scores = HashMap::new();

    for (&pattern_id, pattern) in &store.patterns {
        let keyword_weight = match_keyword_weight(&pattern.template, &weights);
        let anomaly_boost = 1.0 + anomaly_severity.get(&pattern_id).copied().unwrap_or(0.0);
        let score = pattern.count as f64 * keyword_weight * anomaly_boost;
        let severity = severity_from_weight(keyword_weight);

        scores.insert(
            pattern_id,
            PatternScore {
                pattern_id,
                score,
                severity,
                keyword_weight,
            },
        );
    }

    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_template() {
        let tokens = tokenize_template("[ERROR] something happened: <*>");
        assert!(tokens.contains(&"ERROR".to_string()));
        assert!(!tokens.contains(&"[ERROR]".to_string()));
    }

    #[test]
    fn test_keyword_matching() {
        let weights = keyword_weights();
        assert_eq!(match_keyword_weight("[ERROR] foo bar", &weights), 10.0);
        assert_eq!(match_keyword_weight("level=WARN something", &weights), 5.0);
        assert_eq!(match_keyword_weight("DEBUG: checking value", &weights), 0.1);
        assert_eq!(
            match_keyword_weight("INFO normal log message", &weights),
            1.0
        );
    }

    #[test]
    fn test_no_partial_match() {
        let weights = keyword_weights();
        // "warning_count" — tokenized on _, produces "WARNING" and "COUNT"
        // But our delimiter list doesn't include '_', so "WARNING_COUNT" stays as one token.
        assert_eq!(
            match_keyword_weight("warning_count is 5", &weights),
            1.0
        );
    }

    #[test]
    fn test_prefix_only_scanning() {
        let weights = keyword_weights();
        // ERROR past 100 chars should NOT match
        let mut long_template = "INFO normal log line ".repeat(6); // ~126 chars
        long_template.push_str("[ERROR] this is deep in the body");
        assert_eq!(match_keyword_weight(&long_template, &weights), 1.0);

        // ERROR in prefix SHOULD match
        assert_eq!(
            match_keyword_weight("[ERROR] something failed early", &weights),
            10.0
        );

        // Realistic false positive: "error" as a JSON field name at ~175 chars
        let sqq_template = r#"I, [<TS> #<*>] INFO -- : SQS Polling Runner {:mail=>[], :container_tracking=>[{"notificationType"=>"Update", {n1} "success"=><*>, "error"=>nil,"#;
        assert_eq!(match_keyword_weight(sqq_template, &weights), 1.0);
    }

    #[test]
    fn test_severity_classification() {
        assert_eq!(severity_from_weight(10.0), Severity::Error);
        assert_eq!(severity_from_weight(5.0), Severity::Warn);
        assert_eq!(severity_from_weight(0.1), Severity::Debug);
        assert_eq!(severity_from_weight(1.0), Severity::Info);
    }
}
