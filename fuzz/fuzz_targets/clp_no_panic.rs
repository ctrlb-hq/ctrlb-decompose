#![no_main]

use libfuzzer_sys::fuzz_target;
use ctrlb_decompose::extraction::clp::core::{
    EightByteEncodedVariable,
    get_bounds_of_next_var, escape_and_append_const_to_logtype,
    encode_float_string, encode_integer_string,
};

fuzz_target!(|data: &str| {
    // get_bounds_of_next_var should never panic
    let mut begin = 0;
    let mut end = 0;
    while let Some((b, e)) = get_bounds_of_next_var(data, begin, end) {
        assert!(data.is_char_boundary(b));
        assert!(data.is_char_boundary(e));
        assert!(b < e);
        begin = e;
        end = e;
    }

    // escape_and_append_const_to_logtype should never panic
    let mut logtype = String::new();
    escape_and_append_const_to_logtype(data, &mut logtype);

    // encode_float_string and encode_integer_string should never panic
    let _ = encode_float_string::<EightByteEncodedVariable>(data);
    let _ = encode_integer_string::<EightByteEncodedVariable>(data);
});
