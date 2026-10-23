#![no_main]
//! The share format: a header read before any key is asked for, and an open that
//! never derives when it has nothing to derive from.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pandora::vault::share::info(data);
    let _ = pandora::vault::share::open(data, None, None);
    let _ = pandora::vault::share::parse_public(&String::from_utf8_lossy(data));
});
