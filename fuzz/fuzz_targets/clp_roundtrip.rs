#![no_main]

use libfuzzer_sys::fuzz_target;
use ctrlb_decompose::extraction::clp::core::{
    EightByteEncodedVariable, FourByteEncodedVariable,
    encode_message, decode_message,
};

fuzz_target!(|data: &str| {
    // Test 8-byte round-trip
    let (logtype, encoded_vars, dictionary_vars) =
        encode_message::<EightByteEncodedVariable>(data);
    let decoded = decode_message::<EightByteEncodedVariable>(
        &logtype, &encoded_vars, &dictionary_vars,
    );
    assert_eq!(decoded, data, "8-byte round-trip failed");

    // Test 4-byte round-trip
    let (logtype, encoded_vars, dictionary_vars) =
        encode_message::<FourByteEncodedVariable>(data);
    let decoded = decode_message::<FourByteEncodedVariable>(
        &logtype, &encoded_vars, &dictionary_vars,
    );
    assert_eq!(decoded, data, "4-byte round-trip failed");
});
