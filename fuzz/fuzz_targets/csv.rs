#![no_main]
//! The CSV reader: quotes, line endings and short rows from a file nobody checked.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = pandora::shared::utils::csv::parse(text);
    }
});
