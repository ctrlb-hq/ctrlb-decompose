use crate::extraction::clp::core::{
    EightByteEncodedVariable, VariablePlaceholder, decode_message,
};
use crate::extraction::clp::encoding::EncodingContext;
use crate::extraction::drain3::{Config, Drain, TypedVariable, classify_variable};
use crate::types::PatternID;

/// Result of processing a log line through the CLP → Drain3 pipeline.
pub struct PipelineParsedLog {
    pub pattern_id: PatternID,
    pub display_template: String,
    pub variables: Vec<TypedVariable>,
    pub count: u64,
}

/// CLP → Drain3 pipeline.
///
/// CLP normalizes each line by replacing variables (UUIDs, numbers, hex IDs)
/// with placeholder characters, producing a stable "logtype" string.
/// Drain3 then clusters these logtypes to discover structural patterns.
pub struct ClpDrainPipeline {
    clp_ctx: EncodingContext<EightByteEncodedVariable>,
    drain: Drain,
}

impl ClpDrainPipeline {
    pub fn new(drain_config: Config) -> Self {
        Self {
            clp_ctx: EncodingContext::new(2048, 128),
            drain: Drain::new(drain_config),
        }
    }

    /// Process a single log line (already timestamp-stripped) through the pipeline.
    pub fn process_line(&mut self, stripped: &str) -> PipelineParsedLog {
        // Step 1: CLP encode — normalize variables into placeholders
        let (logtype, encoded_vars, dictionary_vars) = {
            let (lt, ev, dv) = self.clp_ctx.encode_message(stripped);
            (lt.to_string(), ev.to_vec(), dv.to_vec())
        };

        // Step 2: Feed CLP logtype to Drain3 for structural clustering
        let parsed = self.drain.extract_template_and_vars(&logtype);

        // Step 3: Merge CLP variables + Drain3 wildcards into unified output
        let drain_template = &parsed.template;
        let drain_tokens: Vec<&str> = drain_template.split_whitespace().collect();
        let logtype_tokens: Vec<&str> = logtype.split_whitespace().collect();

        let mut display_parts: Vec<String> = Vec::new();
        let mut variables: Vec<TypedVariable> = Vec::new();

        // Cursors into CLP variable arrays
        let mut encoded_cursor: usize = 0;
        let mut dict_cursor: usize = 0;

        // Also track a global cursor across the full logtype to handle
        // CLP variables consumed by tokens we iterate over.
        // We need a second pair of cursors for the content (logtype) side
        // to advance past CLP vars in `<*>` positions.
        let mut content_encoded_cursor: usize = 0;
        let mut content_dict_cursor: usize = 0;

        for (i, drain_tok) in drain_tokens.iter().enumerate() {
            if *drain_tok == "<*>" {
                // Drain3 wildcard: the entire logtype token at this position varied.
                // Decode the original text from the CLP logtype token + vars.
                if i < logtype_tokens.len() {
                    let lt_tok = logtype_tokens[i];
                    let raw = decode_clp_fragment(
                        lt_tok,
                        &encoded_vars,
                        &mut content_encoded_cursor,
                        &dictionary_vars,
                        &mut content_dict_cursor,
                    );
                    variables.push(TypedVariable {
                        var_type: classify_variable(&raw),
                        raw,
                    });
                }
                display_parts.push("<*>".to_string());
            } else {
                // Fixed token in the Drain3 template. It may contain CLP placeholders.
                let lt_tok = if i < logtype_tokens.len() {
                    logtype_tokens[i]
                } else {
                    drain_tok
                };

                // Advance content cursors for this token (mirrors what we do with display cursors)
                advance_clp_cursors(lt_tok, &mut content_encoded_cursor, &mut content_dict_cursor);

                // Build display token: replace each CLP placeholder with <*> and extract vars
                let (display_tok, tok_vars) = expand_clp_placeholders(
                    drain_tok,
                    &encoded_vars,
                    &mut encoded_cursor,
                    &dictionary_vars,
                    &mut dict_cursor,
                );
                display_parts.push(display_tok);
                variables.extend(tok_vars);
            }
        }

        let display_template = display_parts.join(" ");

        PipelineParsedLog {
            pattern_id: parsed.pattern_id,
            display_template,
            variables,
            count: parsed.count,
        }
    }
}

/// Decode a CLP logtype fragment back to original text, advancing var cursors.
fn decode_clp_fragment(
    logtype_fragment: &str,
    encoded_vars: &[EightByteEncodedVariable],
    encoded_cursor: &mut usize,
    dictionary_vars: &[String],
    dict_cursor: &mut usize,
) -> String {
    // Count how many encoded and dict vars are in this fragment
    let (n_encoded, n_dict) = count_clp_vars(logtype_fragment);

    // Slice the relevant vars for decode_message
    let enc_end = (*encoded_cursor + n_encoded).min(encoded_vars.len());
    let dict_end = (*dict_cursor + n_dict).min(dictionary_vars.len());

    let enc_slice = &encoded_vars[*encoded_cursor..enc_end];
    let dict_slice = &dictionary_vars[*dict_cursor..dict_end];

    let decoded = decode_message::<EightByteEncodedVariable>(logtype_fragment, enc_slice, dict_slice);

    *encoded_cursor += n_encoded;
    *dict_cursor += n_dict;

    decoded
}

/// Count the number of encoded (int+float) and dictionary placeholders in a logtype fragment.
fn count_clp_vars(logtype: &str) -> (usize, usize) {
    let mut n_encoded = 0;
    let mut n_dict = 0;
    let escape_char = VariablePlaceholder::Escape as u8 as char;
    let int_char = VariablePlaceholder::Integer as u8 as char;
    let float_char = VariablePlaceholder::Float as u8 as char;
    let dict_char = VariablePlaceholder::Dictionary as u8 as char;

    let mut chars = logtype.chars();
    while let Some(c) = chars.next() {
        if c == escape_char {
            // Skip the next char (it's escaped)
            chars.next();
        } else if c == int_char || c == float_char {
            n_encoded += 1;
        } else if c == dict_char {
            n_dict += 1;
        }
    }

    (n_encoded, n_dict)
}

/// Advance CLP variable cursors past all placeholders in a logtype token.
fn advance_clp_cursors(
    logtype_token: &str,
    encoded_cursor: &mut usize,
    dict_cursor: &mut usize,
) {
    let (n_encoded, n_dict) = count_clp_vars(logtype_token);
    *encoded_cursor += n_encoded;
    *dict_cursor += n_dict;
}

/// Replace CLP placeholder chars in a Drain3 template token with `<*>`,
/// extracting typed variables for each placeholder.
fn expand_clp_placeholders(
    drain_token: &str,
    encoded_vars: &[EightByteEncodedVariable],
    encoded_cursor: &mut usize,
    dictionary_vars: &[String],
    dict_cursor: &mut usize,
) -> (String, Vec<TypedVariable>) {
    let escape_char = VariablePlaceholder::Escape as u8 as char;
    let int_char = VariablePlaceholder::Integer as u8 as char;
    let float_char = VariablePlaceholder::Float as u8 as char;
    let dict_char = VariablePlaceholder::Dictionary as u8 as char;

    let mut display = String::with_capacity(drain_token.len());
    let mut vars = Vec::new();

    let mut chars = drain_token.chars();
    while let Some(c) = chars.next() {
        if c == escape_char {
            // Escaped char — push the next char literally
            if let Some(next) = chars.next() {
                display.push(next);
            }
        } else if c == int_char {
            // Integer variable
            let raw = if *encoded_cursor < encoded_vars.len() {
                let val = encoded_vars[*encoded_cursor];
                *encoded_cursor += 1;
                // Decode integer: the bits are the value
                (val as u64).to_string()
            } else {
                *encoded_cursor += 1;
                "<*>".to_string()
            };
            vars.push(TypedVariable {
                var_type: classify_variable(&raw),
                raw,
            });
            display.push_str("<*>");
        } else if c == float_char {
            // Float variable — use decode_message approach
            let raw = if *encoded_cursor < encoded_vars.len() {
                // Build a mini logtype with just this placeholder to decode
                let mini_logtype = format!("{}", float_char);
                let decoded = decode_message::<EightByteEncodedVariable>(
                    &mini_logtype,
                    &encoded_vars[*encoded_cursor..*encoded_cursor + 1],
                    &[],
                );
                *encoded_cursor += 1;
                decoded
            } else {
                *encoded_cursor += 1;
                "<*>".to_string()
            };
            vars.push(TypedVariable {
                var_type: classify_variable(&raw),
                raw,
            });
            display.push_str("<*>");
        } else if c == dict_char {
            // Dictionary variable
            let raw = if *dict_cursor < dictionary_vars.len() {
                let val = dictionary_vars[*dict_cursor].clone();
                *dict_cursor += 1;
                val
            } else {
                *dict_cursor += 1;
                "<*>".to_string()
            };
            vars.push(TypedVariable {
                var_type: classify_variable(&raw),
                raw,
            });
            display.push_str("<*>");
        } else {
            display.push(c);
        }
    }

    (display, vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_basic() {
        let mut pipeline = ClpDrainPipeline::new(Config::default());

        let line1 = "Request from 10.0.1.15 completed in 45ms status=200";
        let line2 = "Request from 192.168.1.1 completed in 100ms status=500";

        let r1 = pipeline.process_line(line1);
        let r2 = pipeline.process_line(line2);

        // Both lines should match the same pattern
        assert_eq!(r1.pattern_id, r2.pattern_id);
        // Template should have <*> where the variables are
        assert!(r1.display_template.contains("<*>"));
        // Should have extracted variables
        assert!(!r2.variables.is_empty());
    }

    #[test]
    fn test_pipeline_uuid_extraction() {
        let mut pipeline = ClpDrainPipeline::new(Config::default());

        let line1 = "[ts1] [6a18594f-0174-48ae-baa6-b7d072081800] (INFO) [Invista] fetchViewData {\"view_id\":\"879cc438-d86b-4f5b-bb53-2fe1b2a7cd9d\",\"page\":1}";
        let line2 = "[ts2] [bf918d48-6193-49ae-86ad-7d4fdff7a252] (INFO) [Invista] fetchViewData {\"view_id\":\"879cc438-d86b-4f5b-bb53-2fe1b2a7cd9d\",\"page\":1}";

        let r1 = pipeline.process_line(line1);
        let r2 = pipeline.process_line(line2);

        // Should be the same pattern since CLP normalizes UUIDs
        assert_eq!(r1.pattern_id, r2.pattern_id);

        // The UUIDs should be extracted as variables
        let uuid_vars: Vec<_> = r2.variables.iter()
            .filter(|v| v.raw.contains('-') && v.raw.len() > 30)
            .collect();
        assert!(uuid_vars.len() >= 2, "Should extract UUIDs as variables, got: {:?}", r2.variables);
    }

    #[test]
    fn test_count_clp_vars() {
        let int_char = VariablePlaceholder::Integer as u8 as char;
        let dict_char = VariablePlaceholder::Dictionary as u8 as char;
        let float_char = VariablePlaceholder::Float as u8 as char;

        let s = format!("hello{}world{}foo{}", int_char, dict_char, float_char);
        let (n_enc, n_dict) = count_clp_vars(&s);
        assert_eq!(n_enc, 2); // int + float
        assert_eq!(n_dict, 1);
    }

    #[test]
    fn test_pipeline_no_vars() {
        let mut pipeline = ClpDrainPipeline::new(Config::default());

        let line = "INFO Starting application";
        let result = pipeline.process_line(line);

        assert_eq!(result.display_template, "INFO Starting application");
        assert!(result.variables.is_empty());
    }
}
