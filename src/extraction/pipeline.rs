use crate::extraction::clp::core::{EightByteEncodedVariable, VariablePlaceholder, decode_message};
use crate::extraction::clp::encoding::EncodingContext;
use crate::extraction::drain3::{Config, Drain, TypedVariable, classify_variable};
use crate::types::PatternID;

/// Result of processing a log line through the CLP → Drain3 pipeline.
pub struct PipelineParsedLog {
    pub pattern_id: PatternID,
    pub display_template: String,
    pub variables: Vec<TypedVariable>,
    /// Actual variable counts for tokens that have slots, in token order.
    /// Literal `<*>` text in a fixed token does not contribute a slot.
    pub token_slot_counts: Vec<(usize, usize)>,
    /// Decoded fixed tokens with embedded CLP variables, indexed by token position.
    /// These feed bounded summaries in case Drain later widens the whole token.
    pub embedded_tokens: Vec<(usize, String)>,
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

        let (display_template, variables, embedded_tokens, token_slot_counts) =
            extract_template_variables(&parsed.template, &logtype, &encoded_vars, &dictionary_vars);

        PipelineParsedLog {
            pattern_id: parsed.pattern_id,
            display_template,
            variables,
            token_slot_counts,
            embedded_tokens,
            count: parsed.count,
        }
    }
}

/// Extract against a Drain template. Only a whole-token
/// wildcard replaces a CLP token; fixed tokens use the original CLP placeholders.
fn extract_template_variables(
    template: &str,
    logtype: &str,
    encoded_vars: &[EightByteEncodedVariable],
    dictionary_vars: &[String],
) -> (
    String,
    Vec<TypedVariable>,
    Vec<(usize, String)>,
    Vec<(usize, usize)>,
) {
    let template_tokens: Vec<&str> = template.split_whitespace().collect();
    let logtype_tokens: Vec<&str> = logtype.split_whitespace().collect();
    debug_assert_eq!(template_tokens.len(), logtype_tokens.len());

    let mut display_parts = Vec::new();
    let mut variables = Vec::new();
    let mut embedded_tokens = Vec::new();
    let mut token_slot_counts = Vec::new();

    // Both wildcard decoding and fixed-token expansion consume the same CLP
    // streams in logtype order. A wildcard must advance both shared cursors.
    let mut encoded_cursor = 0;
    let mut dict_cursor = 0;

    for (position, (template_token, logtype_token)) in
        template_tokens.iter().zip(&logtype_tokens).enumerate()
    {
        let slots_before = variables.len();
        if *template_token == "<*>" {
            let raw = decode_clp_fragment(
                logtype_token,
                encoded_vars,
                &mut encoded_cursor,
                dictionary_vars,
                &mut dict_cursor,
            );
            variables.push(TypedVariable {
                var_type: classify_variable(&raw),
                raw,
            });
            display_parts.push("<*>".to_string());
        } else {
            let mut token_encoded_cursor = encoded_cursor;
            let mut token_dict_cursor = dict_cursor;
            let (display_token, token_vars) = expand_clp_placeholders(
                logtype_token,
                encoded_vars,
                &mut encoded_cursor,
                dictionary_vars,
                &mut dict_cursor,
            );
            if !token_vars.is_empty() && display_token != "<*>" {
                embedded_tokens.push((
                    position,
                    decode_clp_fragment(
                        logtype_token,
                        encoded_vars,
                        &mut token_encoded_cursor,
                        dictionary_vars,
                        &mut token_dict_cursor,
                    ),
                ));
            }
            display_parts.push(display_token);
            variables.extend(token_vars);
        }
        let slot_count = variables.len() - slots_before;
        if slot_count != 0 {
            token_slot_counts.push((position, slot_count));
        }
    }

    debug_assert_eq!(encoded_cursor, encoded_vars.len());
    debug_assert_eq!(dict_cursor, dictionary_vars.len());
    (
        display_parts.join(" "),
        variables,
        embedded_tokens,
        token_slot_counts,
    )
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

    let decoded =
        decode_message::<EightByteEncodedVariable>(logtype_fragment, enc_slice, dict_slice);

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
    use crate::types::VarType;

    #[test]
    fn test_pipeline_path_widening_keeps_cursors_in_sync() {
        let mut pipeline = ClpDrainPipeline::new(Config::default());
        let first = pipeline.process_line("request path=/foo/1 elapsed=10");
        assert_eq!(first.display_template, "request path=/foo/<*> elapsed=<*>");
        assert_eq!(
            first
                .variables
                .iter()
                .map(|v| v.raw.as_str())
                .collect::<Vec<_>>(),
            ["1", "10"]
        );

        let second = pipeline.process_line("request path=/bar/2 elapsed=20");
        assert_eq!(second.pattern_id, first.pattern_id);
        assert_eq!(second.display_template, "request <*> elapsed=<*>");
        assert_eq!(
            second
                .variables
                .iter()
                .map(|v| v.raw.as_str())
                .collect::<Vec<_>>(),
            ["path=/bar/2", "20"]
        );
    }

    #[test]
    fn test_pipeline_interleaved_wildcards_and_placeholders() {
        let mut pipeline = ClpDrainPipeline::new(Config::default());
        // Seed a cluster with wildcards at the beginning, middle and end. Each
        // fixed token has integer, dictionary and float placeholders.
        pipeline
            .drain
            .train("<*> metrics=\x11,\x12,\x13 <*> more=\x11,\x12,\x13 <*>");

        for (line, expected) in [
            (
                "route(\x11,\x12,\x13,\\,/foo/1,0xabc1,1.25) metrics=10,deadbeef,2.50 phase(/v2/22,feedface,3.75) more=30,0xface,4.50 tail(/v3/33,ab12,5.75)",
                [
                    "route(\x11,\x12,\x13,\\,/foo/1,0xabc1,1.25)",
                    "10",
                    "deadbeef",
                    "2.50",
                    "phase(/v2/22,feedface,3.75)",
                    "30",
                    "0xface",
                    "4.50",
                    "tail(/v3/33,ab12,5.75)",
                ],
            ),
            (
                "lane(/bar/2,0xdef2,6.25) metrics=20,cafebabe,7.50 stage(/v4/44,deadbeef,8.75) more=40,0xbeef,9.50 end(/v5/55,cd34,10.75)",
                [
                    "lane(/bar/2,0xdef2,6.25)",
                    "20",
                    "cafebabe",
                    "7.50",
                    "stage(/v4/44,deadbeef,8.75)",
                    "40",
                    "0xbeef",
                    "9.50",
                    "end(/v5/55,cd34,10.75)",
                ],
            ),
            (
                "start metrics=50,facefeed,11.50 middle more=60,0xabcd,12.50 finish",
                [
                    "start", "50", "facefeed", "11.50", "middle", "60", "0xabcd", "12.50", "finish",
                ],
            ),
        ] {
            let parsed = pipeline.process_line(line);
            assert_eq!(parsed.pattern_id, 1);
            assert_eq!(
                parsed.display_template,
                "<*> metrics=<*>,<*>,<*> <*> more=<*>,<*>,<*> <*>"
            );
            let expected_types = [
                VarType::String,
                VarType::Integer,
                VarType::HexID,
                VarType::Float,
                VarType::String,
                VarType::Integer,
                VarType::HexID,
                VarType::Float,
                VarType::String,
            ];
            let expected_vars: Vec<_> = expected
                .into_iter()
                .zip(expected_types)
                .map(|(raw, var_type)| TypedVariable {
                    raw: raw.into(),
                    var_type,
                })
                .collect();
            assert_eq!(parsed.variables, expected_vars, "line: {line}");
        }
    }

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
        let uuid_vars: Vec<_> = r2
            .variables
            .iter()
            .filter(|v| v.raw.contains('-') && v.raw.len() > 30)
            .collect();
        assert!(
            uuid_vars.len() >= 2,
            "Should extract UUIDs as variables, got: {:?}",
            r2.variables
        );
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
