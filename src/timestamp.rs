use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;

/// Result of timestamp extraction: the parsed datetime and the byte range it occupied in the line.
pub struct TimestampMatch {
    pub datetime: DateTime<Utc>,
    pub start: usize,
    pub end: usize,
}

// Alternation selects the earliest position. Complete formats precede their
// inner matches, so an Apache bracket timestamp wins over its syslog portion.
static RE_TIMESTAMP: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"\[[A-Z][a-z]{2} [A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2} \d{4}\]",
        r"|\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?",
        r"|\d{2}/[A-Z][a-z]{2}/\d{4}:\d{2}:\d{2}:\d{2} [+-]\d{4}",
        r"|[A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2}",
        r"|\b1[0-9]{9}(?:\d{3})?\b",
    ))
    .unwrap()
});

/// Extract the first timestamp found in a log line, returning the datetime and byte range.
pub fn extract_timestamp(line: &str) -> Option<TimestampMatch> {
    let mut offset = 0;
    while let Some(matched) = RE_TIMESTAMP.find_at(line, offset) {
        let text = matched.as_str();
        // The formats have distinct opening characters or date separators;
        // identifying them here avoids capture tracking during the search.
        let datetime = match text.as_bytes()[0] {
            b'[' => parse_apache_bracket(text),
            b'A'..=b'Z' => parse_syslog(text),
            _ if text.contains('/') => parse_common_log(text),
            _ if text.contains('-') => parse_iso8601(text),
            _ => parse_epoch(text),
        };
        if let Some(datetime) = datetime {
            return Some(TimestampMatch {
                datetime,
                start: matched.start(),
                end: matched.end(),
            });
        }
        // A failed enclosing candidate may still contain a valid inner one.
        offset = matched.start() + line[matched.start()..].chars().next()?.len_utf8();
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
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%d %H:%M:%S%.f%z"] {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    for fmt in &[
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%dT%H:%M:%SZ",
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
    fn test_timestamp_compact_offsets() {
        for (raw, expected) in [
            ("2024-01-15T14:22:01+0530", "08:52:01"),
            ("2024-01-15T14:22:01.123-0230", "16:52:01"),
            ("2024-01-15 14:22:01+0530", "08:52:01"),
        ] {
            let ts = extract_timestamp(raw).unwrap();
            assert_eq!(ts.datetime.format("%H:%M:%S").to_string(), expected);
        }
    }

    #[test]
    fn test_timestamp_position_and_enclosing_format() {
        for (line, expected) in [
            (
                "[15/Jan/2024:14:22:01 +0000] INFO sent_at=2024-01-15T15:30:00Z",
                "15/Jan/2024:14:22:01 +0000",
            ),
            ("1705328521 INFO sent_at=2024-01-15T15:30:00Z", "1705328521"),
            (
                "2024-01-15T14:22:01Z [Thu Jun 09 06:07:04 2005]",
                "2024-01-15T14:22:01Z",
            ),
            (
                "Jan 15 14:22:01 INFO 2024-01-15T15:30:00Z",
                "Jan 15 14:22:01",
            ),
            ("[Thu Jun 09 06:07:04 2005]", "[Thu Jun 09 06:07:04 2005]"),
            (
                "2024-99-99T00:00:00Z then 2024-01-15T14:22:01Z",
                "2024-01-15T14:22:01Z",
            ),
        ] {
            let ts = extract_timestamp(line).unwrap();
            assert_eq!(&line[ts.start..ts.end], expected);
        }
    }

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
        assert_eq!(ts.datetime.format("%m-%d %H:%M:%S").to_string(), "01-15 14:22:01");
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
