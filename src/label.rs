use crate::types::VarType;

/// Infer a semantic label for a variable slot based on the template context and type.
pub fn infer_label(template: &str, slot_index: usize, var_type: VarType) -> String {
    let tokens: Vec<&str> = template.split_whitespace().collect();

    // Find the nth wildcard position
    let mut wildcard_count = 0;
    let mut token_idx = None;
    for (i, token) in tokens.iter().enumerate() {
        if *token == "<*>" {
            if wildcard_count == slot_index {
                token_idx = Some(i);
                break;
            }
            wildcard_count += 1;
        }
    }

    if let Some(idx) = token_idx {
        // Check preceding token for contextual hints
        if idx > 0 {
            let prev = tokens[idx - 1].to_lowercase();

            // key=<*> pattern
            if let Some(key) = prev.strip_suffix('=') {
                return sanitize_label(key);
            }

            // Duration keywords
            if matches!(
                prev.as_str(),
                "in" | "after" | "took" | "elapsed" | "waited"
            ) {
                if var_type == VarType::Duration {
                    return "duration".to_string();
                }
                return "duration_ms".to_string();
            }

            // Common keywords
            let keywords = [
                "status", "code", "port", "host", "user", "path", "method", "size", "bytes",
                "count", "level", "thread", "pid", "latency", "timeout", "error", "retry",
                "attempt",
            ];
            for kw in keywords {
                if prev.contains(kw) {
                    return kw.to_string();
                }
            }
        }

        // Check following token for time unit suffixes
        if idx + 1 < tokens.len() {
            let next = tokens[idx + 1].to_lowercase();
            if matches!(next.as_str(), "ms" | "seconds" | "s" | "minutes" | "hours") {
                return "duration".to_string();
            }
        }
    }

    // Type-based defaults
    match var_type {
        VarType::IPv4 | VarType::IPv6 => "ip".to_string(),
        VarType::UUID => "uuid".to_string(),
        VarType::HexID => "id".to_string(),
        VarType::Duration => "duration".to_string(),
        VarType::Timestamp => "timestamp".to_string(),
        VarType::Integer => format!("n{}", slot_index + 1),
        VarType::Float => format!("f{}", slot_index + 1),
        VarType::Enum => format!("enum{}", slot_index + 1),
        VarType::String => format!("var{}", slot_index + 1),
    }
}

fn sanitize_label(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_value_pattern() {
        assert_eq!(
            infer_label("INFO request status= <*>", 0, VarType::Integer),
            "status"
        );
    }

    #[test]
    fn test_duration_keyword() {
        assert_eq!(
            infer_label("completed in <*>", 0, VarType::Duration),
            "duration"
        );
    }

    #[test]
    fn test_type_default_ipv4() {
        assert_eq!(infer_label("connecting to <*>", 0, VarType::IPv4), "ip");
    }

    #[test]
    fn test_type_default_integer() {
        assert_eq!(
            infer_label("something <*> happened", 0, VarType::Integer),
            "n1"
        );
    }

    #[test]
    fn test_type_default_uuid() {
        assert_eq!(infer_label("trace <*> started", 0, VarType::UUID), "uuid");
    }
}
