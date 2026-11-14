use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use memchr::memchr_iter;
use once_cell::sync::Lazy;
use regex::Regex;

/// Result of timestamp extraction: the parsed datetime and the byte range it occupied in the line.
pub struct TimestampMatch {
    pub datetime: DateTime<Utc>,
    pub start: usize,
    pub end: usize,
}

static RE_ISO8601: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?").unwrap()
});

// Complete enclosing formats are matched before considering yearless syslog.
static RE_BRACKETED_DATE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"\[[A-Z][a-z]{2} [A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2} \d{4}\]",
        r"|\d{2}/[A-Z][a-z]{2}/\d{4}:\d{2}:\d{2}:\d{2} [+-]\d{4}",
    ))
    .unwrap()
});

static RE_EPOCH: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b1[0-9]{9}(?:\d{3})?\b").unwrap());

static RE_SYSLOG: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2}").unwrap());

/// Prefer the earliest valid dated timestamp, then epoch, then yearless syslog.
pub fn extract_timestamp(line: &str) -> Option<TimestampMatch> {
    let iso = find_timestamp(line, &RE_ISO8601, parse_iso8601);
    // The dated formats cannot overlap each other. Only search before an ISO
    // candidate for an earlier bracket/common-log date, keeping the usual
    // leading ISO case cheap without giving it priority over earlier dates.
    let prefix = &line[..iso.as_ref().map_or(line.len(), |ts| ts.start)];
    // Both bracket/common-log formats require at least 26 bytes.
    let dated = if prefix.len() < 26 {
        None
    } else {
        find_timestamp(prefix, &RE_BRACKETED_DATE, |text| {
            match text.as_bytes()[0] {
                b'[' => parse_apache_bracket(text),
                _ => parse_common_log(text),
            }
        })
    };
    dated
        .or(iso)
        .or_else(|| {
            // Most syslog lines contain only short numeric fields. Require a
            // possible epoch's ten ASCII digits before checking word boundaries.
            let bytes = line.as_bytes();
            memchr_iter(b'1', bytes)
                .any(|i| {
                    bytes[i..]
                        .get(..10)
                        .is_some_and(|digits| digits.iter().all(u8::is_ascii_digit))
                })
                .then(|| find_timestamp(line, &RE_EPOCH, parse_epoch))
                .flatten()
        })
        .or_else(|| find_timestamp(line, &RE_SYSLOG, parse_syslog))
}

fn find_timestamp(
    line: &str,
    regex: &Regex,
    parse: impl Fn(&str) -> Option<DateTime<Utc>>,
) -> Option<TimestampMatch> {
    let mut offset = 0;
    while let Some(matched) = regex.find_at(line, offset) {
        if let Some(datetime) = parse(matched.as_str()) {
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
            (
                "1705328521 INFO sent_at=2024-01-15T15:30:00Z",
                "2024-01-15T15:30:00Z",
            ),
            (
                "2024-01-15T14:22:01Z [Thu Jun 09 06:07:04 2005]",
                "2024-01-15T14:22:01Z",
            ),
            (
                "Jan 15 14:22:01 INFO 2024-01-15T15:30:00Z",
                "2024-01-15T15:30:00Z",
            ),
            ("Jan 15 14:22:01 INFO 1705328521", "1705328521"),
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
    fn test_syslog_relay_prefers_full_iso_timestamp() {
        let line =
            "Jan 15 14:22:01 web1 app[42]: 2024-01-15T14:22:01.123+0530 INFO request done in 12ms";
        let ts = extract_timestamp(line).unwrap();
        assert_eq!(&line[ts.start..ts.end], "2024-01-15T14:22:01.123+0530");
        assert_eq!(ts.datetime.to_rfc3339(), "2024-01-15T08:52:01.123+00:00");
        assert_eq!(
            strip_timestamp(line, &ts),
            "Jan 15 14:22:01 web1 app[42]: <TS> INFO request done in 12ms"
        );
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
