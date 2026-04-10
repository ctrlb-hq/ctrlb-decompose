#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use ctrlb_decompose::extraction::drain3::{Config, Drain};

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    lines: Vec<String>,
}

fuzz_target!(|input: FuzzInput| {
    if input.lines.is_empty() || input.lines.len() > 100 {
        return;
    }

    let config = Config::default();
    let mut drain = Drain::new(config);

    for line in &input.lines {
        if line.len() > 500 {
            continue;
        }
        let parsed = drain.extract_template_and_vars(line);
        assert!(parsed.pattern_id > 0);
        assert!(parsed.count >= 1);
    }
});
