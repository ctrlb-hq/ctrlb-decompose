use crate::types::VarType;

/// Infer a semantic label for a variable slot based on the template context and type.
pub fn infer_label(template: &str, slot_index: usize, var_type: VarType) -> String {
    let wildcard = "<*>";
    let mut current_slot = 0;
    let mut search_start = 0;

    while let Some(pos) = template[search_start..].find(wildcard) {
        let abs_pos = search_start + pos;
        if current_slot == slot_index {
            let before = &template[..abs_pos];
            let after = &template[abs_pos + wildcard.len()..];
            return label_from_context(before, after, var_type, slot_index);
        }
        current_slot += 1;
        search_start = abs_pos + wildcard.len();
    }

    // Fallback if slot_index exceeds <*> count
    type_default(var_type, slot_index)
}

fn label_from_context(before: &str, after: &str, var_type: VarType, slot_index: usize) -> String {
    // Check for key=<*> pattern: text immediately before ends with "="
    let before_trimmed = before.trim_end();
    if before_trimmed.ends_with('=') {
        let key_region = &before_trimmed[..before_trimmed.len() - 1];
        if let Some(key) = key_region
            .split(|c: char| c.is_whitespace() || c == '/' || c == '[' || c == '(')
            .last()
        {
            let clean = key.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if !clean.is_empty() {
                return sanitize_label(clean);
            }
        }
    }

    // Get the word immediately before the <*>
    let prev_word = before.split_whitespace().last().unwrap_or("");
    let prev_clean = prev_word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');

    // Duration keywords
    if matches!(
        prev_clean.to_lowercase().as_str(),
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
    let prev_lower = prev_clean.to_lowercase();
    for kw in keywords {
        if prev_lower.contains(kw) {
            return kw.to_string();
        }
    }

    // Check following text for time unit suffixes
    let next_word = after.split_whitespace().next().unwrap_or("");
    if matches!(
        next_word.to_lowercase().as_str(),
        "ms" | "seconds" | "s" | "minutes" | "hours"
    ) {
        return "duration".to_string();
    }

    type_default(var_type, slot_index)
}

fn type_default(var_type: VarType, slot_index: usize) -> String {
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
        assert_eq!(
            infer_label("connecting to <*>", 0, VarType::IPv4),
            "ip"
        );
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
        assert_eq!(
            infer_label("trace <*> started", 0, VarType::UUID),
            "uuid"
        );
    }

    #[test]
    fn test_embedded_wildcard_key_value() {
        // "pool_size=<*>/<*> queue_depth=<*>" — 3 slots
        assert_eq!(
            infer_label("Pool saturated pool_size=<*>/<*> queue_depth=<*>", 0, VarType::Integer),
            "pool_size"
        );
        assert_eq!(
            infer_label("Pool saturated pool_size=<*>/<*> queue_depth=<*>", 2, VarType::Integer),
            "queue_depth"
        );
    }

    #[test]
    fn test_embedded_wildcard_brackets() {
        // slot 0=[<*>], slot 1=<*> after "job", slot 2=<*> after "in", slot 3=records=<*>
        assert_eq!(
            infer_label("<TS> INFO [<*>] Batch job <*> completed in <*> records=<*>", 2, VarType::Duration),
            "duration"
        );
        assert_eq!(
            infer_label("<TS> INFO [<*>] Batch job <*> completed in <*> records=<*>", 3, VarType::Integer),
            "records"
        );
    }

    #[test]
    fn test_embedded_wildcard_not_mislabeled() {
        let label = infer_label(
            "<TS> INFO [<*>] Batch job <*> completed in <*> records=<*>",
            1,
            VarType::String,
        );
        assert_ne!(label, "duration_ms", "Batch job name should not be labeled duration_ms");
        assert_ne!(label, "duration", "Batch job name should not be labeled duration");
    }
}
