use std::collections::HashMap;

use memchr::memchr2_iter;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Notice,
    Warn,
    Warning,
    Error,
    Fatal,
    Panic,
    Crit,
    Critical,
    Alert,
    Emerg,
    Emergency,
}

impl LogLevel {
    // Keep the level roots indexed by the enum's contiguous discriminants.
    pub(crate) const COUNT: usize = Self::Emergency as usize + 1;

    pub fn weight(self) -> f64 {
        match self {
            Self::Trace | Self::Debug => 0.1,
            Self::Info | Self::Notice => 1.0,
            Self::Warn | Self::Warning => 5.0,
            _ => 10.0,
        }
    }
}

/// Detect a level in a timestamp-stripped line. Bare levels must be in the
/// first five tokens and use uppercase or an explicit enclosing delimiter.
#[inline]
pub fn detect_log_level(line: &str) -> Option<LogLevel> {
    // A leading uppercase level (also immediately after <TS>) cannot have an
    // earlier key. Avoid searching its message body in this common case.
    let leading = line.strip_prefix("<TS> ").unwrap_or(line);
    if leading
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_uppercase)
        && let Some(level) = leading
            .split_whitespace()
            .next()
            .and_then(|token| parse_header_level(token.as_bytes()))
    {
        return Some(level);
    }

    // Search at the key offsets directly, without regex captures or case folding
    // the entire line (including non-ASCII message bodies).
    let bytes = line.as_bytes();
    let keyed = memchr2_iter(b'l', b'L', bytes)
        .find_map(|i| parse_keyed_level(bytes, i).map(|level| (i, level)));
    let before_key = &bytes[..keyed.map_or(bytes.len(), |(offset, _)| offset)];
    let keyed = memchr2_iter(b's', b'S', before_key)
        .find_map(|i| parse_keyed_level(bytes, i).map(|level| (i, level)))
        .or(keyed);

    // A header can only override a keyed value if it occurs earlier. In
    // particular, don't scan a JSON/logfmt message body for header tokens.
    let prefix = line[..keyed.map_or(line.len(), |(offset, _)| offset)].trim_end();
    // A single printable ASCII token with no header opener cannot contain a
    // bare level. This covers timestamp fields before a logfmt/JSON level key.
    if prefix
        .as_bytes()
        .first()
        .is_some_and(|b| !matches!(b, b'A'..=b'Z' | b'[' | b'<' | b'('))
        && prefix.bytes().all(|b| (b'!'..=b'~').contains(&b))
    {
        return keyed.map(|(_, level)| level);
    }
    for token in prefix.split_whitespace().take(5) {
        if let Some(level) = parse_header_level(token.as_bytes()) {
            return Some(level);
        }
    }
    keyed.map(|(_, level)| level)
}

fn parse_header_level(token: &[u8]) -> Option<LogLevel> {
    let close = match token.first()? {
        b'[' => b']',
        b'<' => b'>',
        b'(' => b')',
        b'A'..=b'Z' => {
            let word = token.split(|&b| b == b':').next()?;
            if !word.iter().all(u8::is_ascii_uppercase) {
                return None;
            }
            return parse_log_level(word);
        }
        _ => return None,
    };
    let end = token.iter().position(|&b| b == close)?;
    if !token[end + 1..].iter().all(|b| b";,:.".contains(b)) {
        return None;
    }
    parse_log_level(token[1..end].rsplit(|&b| b == b':').next()?)
}

#[inline]
fn parse_keyed_level(bytes: &[u8], offset: usize) -> Option<LogLevel> {
    let mut start = offset;
    if start > 0 && matches!(bytes[start - 1], b'\'' | b'"') {
        start -= 1;
    }
    if start > 0 && !bytes[start - 1].is_ascii_whitespace() && !b",{[".contains(&bytes[start - 1]) {
        return None;
    }
    let rest = &bytes[offset..];
    let key_len = [b"level".as_slice(), b"lvl", b"severity"]
        .into_iter()
        .find_map(|key| {
            rest.get(..key.len())
                .filter(|s| s.eq_ignore_ascii_case(key))
                .map(|_| key.len())
        })?;
    let mut rest = &rest[key_len..];
    if matches!(rest.first(), Some(b'\'' | b'"')) {
        rest = &rest[1..];
    }
    rest = rest.trim_ascii_start();
    if !matches!(rest.first(), Some(b':' | b'=')) {
        return None;
    }
    rest = rest[1..].trim_ascii_start();
    if matches!(rest.first(), Some(b'\'' | b'"')) {
        rest = &rest[1..];
    }
    let end = rest
        .iter()
        .position(|b| !b.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    // Reject partial values such as error_handler, error2 and erroré.
    if rest
        .get(end)
        .is_some_and(|b| b.is_ascii_digit() || *b == b'_' || !b.is_ascii())
    {
        return None;
    }
    parse_log_level(&rest[..end])
}

#[inline]
fn parse_log_level(level: &[u8]) -> Option<LogLevel> {
    // Compare the common four-byte levels directly, avoiding a case-conversion
    // buffer on the INFO/WARN path used by most structured logs.
    if level.len() == 4 {
        return if level.eq_ignore_ascii_case(b"INFO") {
            Some(LogLevel::Info)
        } else if level.eq_ignore_ascii_case(b"WARN") {
            Some(LogLevel::Warn)
        } else if level.eq_ignore_ascii_case(b"CRIT") {
            Some(LogLevel::Crit)
        } else {
            None
        };
    }
    // All level spellings are ASCII and at most nine bytes long.
    let mut uppercase = [0; 9];
    if !(4..=uppercase.len()).contains(&level.len()) {
        return None;
    }
    for (out, byte) in uppercase.iter_mut().zip(level) {
        *out = byte.to_ascii_uppercase();
    }
    match &uppercase[..level.len()] {
        b"TRACE" => Some(LogLevel::Trace),
        b"DEBUG" => Some(LogLevel::Debug),
        b"NOTICE" => Some(LogLevel::Notice),
        b"WARNING" => Some(LogLevel::Warning),
        b"ERROR" => Some(LogLevel::Error),
        b"FATAL" => Some(LogLevel::Fatal),
        b"PANIC" => Some(LogLevel::Panic),
        b"CRITICAL" => Some(LogLevel::Critical),
        b"ALERT" => Some(LogLevel::Alert),
        b"EMERG" => Some(LogLevel::Emerg),
        b"EMERGENCY" => Some(LogLevel::Emergency),
        _ => None,
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
#[cfg(test)]
fn match_keyword_weight(template: &str, weights: &HashMap<&str, f64>) -> f64 {
    match_keyword_weight_with_level(template, weights, None)
}

fn match_keyword_weight_with_level(
    template: &str,
    weights: &HashMap<&str, f64>,
    level: Option<LogLevel>,
) -> f64 {
    let mut end = template.len().min(KEYWORD_PREFIX_LEN);
    while !template.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &template[..end];
    let tokens = tokenize_template(prefix);
    let mut max_weight = level.map_or(1.0, LogLevel::weight);
    let mut found = level.is_some();

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
        let keyword_weight =
            match_keyword_weight_with_level(&pattern.template, &weights, pattern.detected_level);
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
    fn test_detect_raw_log_levels() {
        for (raw, expected) in [
            ("[ERROR] request failed", Some(LogLevel::Error)),
            ("<ERROR>; request failed", Some(LogLevel::Error)),
            ("level=\"warn\" operation=read", Some(LogLevel::Warn)),
            (r#"{"lvl": "debug"}"#, Some(LogLevel::Debug)),
            (r#"{"severity": "FATAL"}"#, Some(LogLevel::Fatal)),
            ("payload level=INFO", Some(LogLevel::Info)),
            ("level=error_handler", None),
            (r#"{"error":false}"#, None),
            ("ordinary message", None),
        ] {
            assert_eq!(detect_log_level(raw), expected, "{raw}");
        }
        assert_eq!(
            detect_log_level(&format!("{} ERROR", "x".repeat(101))),
            Some(LogLevel::Error)
        );
        assert_eq!(
            detect_log_level(&format!("{} severity=ERROR", "x".repeat(101))),
            Some(LogLevel::Error)
        );
        let weights = keyword_weights();
        assert_eq!(
            match_keyword_weight_with_level("level=<*> failed", &weights, Some(LogLevel::Info)),
            5.0
        );
        assert_eq!(
            match_keyword_weight_with_level("level=<*> normal", &weights, Some(LogLevel::Debug)),
            0.1
        );
    }

    #[test]
    fn test_header_level_forms_and_token_limit() {
        for (line, expected) in [
            ("<TS> ERROR a b c d", Some(LogLevel::Error)),
            ("I, [<TS> #1]  ERROR -- : msg", Some(LogLevel::Error)),
            ("<TS> [core:error] [pid 1] msg", Some(LogLevel::Error)),
            (
                "2024/01/15 14:22:01 [error] 123#0: msg",
                Some(LogLevel::Error),
            ),
            ("<TS> web1 app[42]: ERROR msg", Some(LogLevel::Error)),
            ("ERROR:root:msg", Some(LogLevel::Error)),
            ("(warn) msg", Some(LogLevel::Warn)),
            ("a b c d INFO msg", Some(LogLevel::Info)),
            ("a\u{2003}b c d INFO msg", Some(LogLevel::Info)),
            ("a\u{2003}b c d e INFO msg", None),
            ("a\u{000b}b c d INFO msg", Some(LogLevel::Info)),
            ("a b c d e ERROR msg", None),
            ("a b c d e [ERROR] msg", None),
            ("info msg", None),
            ("Error msg", None),
            ("user info updated id=1", None),
            ("Setting log level to debug for module a1", None),
            ("[error_handler] msg", None),
            ("[core:error]suffix msg", None),
            ("level=warn ERROR msg", Some(LogLevel::Warn)),
            ("ERROR msg severity=info", Some(LogLevel::Error)),
            ("[core:error] msg level=warn", Some(LogLevel::Error)),
        ] {
            assert_eq!(detect_log_level(line), expected, "{line}");
        }
        for word in ["ALERT", "EMERG", "EMERGENCY", "NOTICE", "CRIT"] {
            let level = parse_log_level(word.as_bytes()).unwrap();
            assert_eq!(detect_log_level(&format!("{word} msg")), Some(level));
            assert_eq!(
                detect_log_level(&format!("[{}] msg", word.to_lowercase())),
                Some(level)
            );
            assert_eq!(
                detect_log_level(&format!("{} msg", word.to_lowercase())),
                None
            );
        }
    }

    #[test]
    fn test_ascii_keyed_levels_with_unicode_messages() {
        for (line, expected) in [
            ("message λ level=warn", Some(LogLevel::Warn)),
            (r#"{"LEVEL" : "eRrOr", "msg":"λ"}"#, Some(LogLevel::Error)),
            ("['LvL' = 'trace'] λ", Some(LogLevel::Trace)),
            ("severity=warn level=error", Some(LogLevel::Warn)),
            ("level=error severity=warn", Some(LogLevel::Error)),
            ("level=invalid severity=warn", Some(LogLevel::Warn)),
            ("level info severity = emerg", Some(LogLevel::Emerg)),
            ("ſeverity=error", None),
            ("élevel=error", None),
            ("log.level=error", None),
            ("level=error2", None),
            ("level=erroré", None),
            ("level=error_handler", None),
            ("level=", None),
        ] {
            assert_eq!(detect_log_level(line), expected, "{line}");
        }
    }

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
