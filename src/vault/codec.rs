use base64::{engine::general_purpose::STANDARD, Engine as _};

pub fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// Base64 as a `Display`, so a serializer writes it without an intermediate `String`.
pub fn display(bytes: &[u8]) -> impl std::fmt::Display + '_ {
    base64::display::Base64Display::new(bytes, &STANDARD)
}

pub fn decode(s: &str) -> Result<Vec<u8>, String> {
    STANDARD.decode(s).map_err(|e| format!("bad base64: {e}"))
}
