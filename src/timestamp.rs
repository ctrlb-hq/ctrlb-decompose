use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;

/// Result of timestamp extraction: the parsed datetime and the byte range it occupied in the line.
pub struct TimestampMatch {
    pub datetime: DateTime<Utc>,
    pub start: usize,
    pub end: usize,
}

// Apache/bracket format: [Thu Jun 09 06:07:04 2005]
static RE_APACHE_BRACKET: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[[A-Z][a-z]{2} [A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2} \d{4}\]").unwrap()
});

static RE_ISO8601: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})?").unwrap()
});

static RE_COMMON_LOG: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\d{2}/[A-Z][a-z]{2}/\d{4}:\d{2}:\d{2}:\d{2} [+-]\d{4}").unwrap());

static RE_SYSLOG: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2}").unwrap());

static RE_EPOCH: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(1[0-9]{9}(\d{3})?)\b").unwrap());

/// Extract the first timestamp found in a log line, returning the datetime and byte range.
pub fn extract_timestamp(line: &str) -> Option<TimestampMatch> {
    // Fast path: Timestamps almost always appear within the first 64 bytes.
    // Searching the prefix first bounds regex DFA/NFA state transitions.
    if line.len() > 64 {
        let prefix_len = line.floor_char_boundary(64);
        if let Some(m) = extract_timestamp_in_str(&line[..prefix_len]) {
            return Some(m);
        }
    }
    extract_timestamp_in_str(line)
}

fn extract_timestamp_in_str(slice: &str) -> Option<TimestampMatch> {
    // Apache bracket format: [Thu Jun 09 06:07:04 2005]
    // Must be before syslog since syslog would partially match the inner part
    if let Some(m) = RE_APACHE_BRACKET.find(slice)
        && let Some(dt) = parse_apache_bracket(m.as_str())
    {
        return Some(TimestampMatch {
            datetime: dt,
            start: m.start(),
            end: m.end(),
        });
    }

    // ISO8601 / RFC3339
    if let Some(m) = RE_ISO8601.find(slice)
        && let Some(dt) = parse_iso8601(m.as_str())
    {
        return Some(TimestampMatch {
            datetime: dt,
            start: m.start(),
            end: m.end(),
        });
    }

    // Common log format: 15/Jan/2024:14:22:01 +0000
    if let Some(m) = RE_COMMON_LOG.find(slice)
        && let Some(dt) = parse_common_log(m.as_str())
    {
        return Some(TimestampMatch {
            datetime: dt,
            start: m.start(),
            end: m.end(),
        });
    }

    // Syslog: Jan 15 14:22:01
    if let Some(m) = RE_SYSLOG.find(slice)
        && let Some(dt) = parse_syslog(m.as_str())
    {
        return Some(TimestampMatch {
            datetime: dt,
            start: m.start(),
            end: m.end(),
        });
    }

    // Epoch seconds (10 digits) or millis (13 digits)
    if let Some(caps) = RE_EPOCH.captures(slice)
        && let Some(m) = caps.get(1)
        && let Some(dt) = parse_epoch(m.as_str())
    {
        return Some(TimestampMatch {
            datetime: dt,
            start: m.start(),
            end: m.end(),
        });
    }

    None
}

fn parse_apache_bracket(s: &str) -> Option<DateTime<Utc>> {
    // s is like "[Thu Jun 09 06:07:04 2005]"
    let inner = &s[1..s.len() - 1]; // strip brackets
    NaiveDateTime::parse_from_str(inner, "%a %b %d %H:%M:%S %Y")
        .ok()
        .map(|dt| dt.and_utc())
}

fn parse_iso8601(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in &[
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M:%S%:z",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc());
        }
    }
    None
}

fn parse_common_log(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_str(s, "%d/%b/%Y:%H:%M:%S %z")
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_syslog(s: &str) -> Option<DateTime<Utc>> {
    let year = Utc::now().year();
    let with_year = format!("{} {}", year, s);
    NaiveDateTime::parse_from_str(&with_year, "%Y %b %e %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc())
}

fn parse_epoch(s: &str) -> Option<DateTime<Utc>> {
    let n: i64 = s.parse().ok()?;
    if s.len() == 13 {
        DateTime::from_timestamp_millis(n)
    } else if s.len() == 10 {
        DateTime::from_timestamp(n, 0)
    } else {
        None
    }
}

/// Strip the timestamp region from a line, replacing it with `<TS>`.
/// Returns the modified line. If no timestamp was found, returns the original.
pub fn strip_timestamp(line: &str, ts_match: &TimestampMatch) -> String {
    let mut result = String::with_capacity(line.len());
    result.push_str(&line[..ts_match.start]);
    result.push_str("<TS>");
    result.push_str(&line[ts_match.end..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iso8601_with_z() {
        let line = "2024-01-15T14:22:01.123Z INFO request completed";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(
            ts.datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2024-01-15 14:22:01"
        );
    }

    #[test]
    fn test_iso8601_with_timezone() {
        let line = "2024-01-15T14:22:01+05:30 INFO request completed";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(
            ts.datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2024-01-15 08:52:01"
        );
    }

    #[test]
    fn test_rfc3339_variant_space() {
        let line = "2024-01-15 14:22:01.456 INFO request completed";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(
            ts.datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2024-01-15 14:22:01"
        );
    }

    #[test]
    fn test_common_log_format() {
        let line = r#"127.0.0.1 - - [15/Jan/2024:14:22:01 +0000] "GET / HTTP/1.1" 200"#;
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(
            ts.datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2024-01-15 14:22:01"
        );
    }

    #[test]
    fn test_syslog_format() {
        let line = "Jan 15 14:22:01 myhost sshd[1234]: Accepted publickey";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(
            ts.datetime.format("%m-%d %H:%M:%S").to_string(),
            "01-15 14:22:01"
        );
    }

    #[test]
    fn test_epoch_seconds() {
        let line = "timestamp=1705328521 level=info msg=hello";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(ts.datetime.format("%Y-%m-%d").to_string(), "2024-01-15");
    }

    #[test]
    fn test_epoch_millis() {
        let line = "ts=1705328521123 level=info";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(ts.datetime.format("%Y-%m-%d").to_string(), "2024-01-15");
    }

    #[test]
    fn test_no_timestamp() {
        let line = "just a plain log line with no timestamp";
        assert!(extract_timestamp(line).is_none());
    }

    #[test]
    fn test_multiple_timestamps_takes_first() {
        let line = "2024-01-15T14:22:01Z processed at 2024-01-15T15:00:00Z";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(ts.datetime.format("%H:%M:%S").to_string(), "14:22:01");
    }

    #[test]
    fn test_apache_bracket_format() {
        let line = "[Thu Jun 09 06:07:04 2005] [notice] LDAP: Built with OpenLDAP LDAP SDK";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(
            ts.datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2005-06-09 06:07:04"
        );
        assert_eq!(ts.start, 0);
        assert_eq!(ts.end, 26); // "[Thu Jun 09 06:07:04 2005]"
    }

    #[test]
    fn test_strip_timestamp_apache() {
        let line = "[Thu Jun 09 06:07:04 2005] [notice] LDAP: Built";
        let ts = extract_timestamp(line).unwrap();
        let stripped = strip_timestamp(line, &ts);
        assert_eq!(stripped, "<TS> [notice] LDAP: Built");
    }

    #[test]
    fn test_strip_timestamp_iso() {
        let line = "2024-01-15T14:22:01.123Z INFO request completed";
        let ts = extract_timestamp(line).unwrap();
        let stripped = strip_timestamp(line, &ts);
        assert_eq!(stripped, "<TS> INFO request completed");
    }

    #[test]
    fn test_strip_timestamp_syslog() {
        let line = "Jan 15 14:22:01 myhost sshd[1234]: Accepted";
        let ts = extract_timestamp(line).unwrap();
        let stripped = strip_timestamp(line, &ts);
        assert_eq!(stripped, "<TS> myhost sshd[1234]: Accepted");
    }
}
