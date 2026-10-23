#![no_main]
//! Base64 is the only thing between a stranger's text and the box's fields.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = pandora::vault::codec::decode(text);
    }
    // The encoder takes any bytes, and its output has to read back.
    if let Ok(round) = pandora::vault::codec::decode(&pandora::vault::codec::display(data).to_string()) {
        assert_eq!(round, data, "base64 round-trips");
    }
});
