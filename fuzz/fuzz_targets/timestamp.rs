#![no_main]

use libfuzzer_sys::fuzz_target;
use ctrlb_decompose::timestamp::{extract_timestamp, strip_timestamp};

fuzz_target!(|data: &str| {
    // extract_timestamp should never panic
    if let Some(result) = extract_timestamp(data) {
        assert!(result.start <= result.end);
        assert!(result.end <= data.len());
        assert!(data.is_char_boundary(result.start));
        assert!(data.is_char_boundary(result.end));

        // strip_timestamp should never panic when given a valid match
        let stripped = strip_timestamp(data, &result);
        assert!(stripped.len() <= data.len() + 4); // original + "<TS>" minus removed range
    }
});
