use base64::{engine::general_purpose::STANDARD, Engine as _};

pub fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn decode(s: &str) -> Result<Vec<u8>, String> {
    STANDARD.decode(s).map_err(|e| format!("bad base64: {e}"))
}
