#![no_main]
//! The three importers, and the vault payload they land in.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pandora::shared::utils::import::detect(data);
    let _ = pandora::shared::utils::import::read(data);
    // The payload's own shape: a box from another device or another version.
    let _ = serde_json::from_slice::<pandora::vault::model::Vault>(data);
});
