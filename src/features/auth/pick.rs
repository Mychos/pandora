//! Android's way in for a box file that lives somewhere else on the phone.
//!
//! There is no dialog to open: the activity hands out a `content://` URI, which
//! is not a path the vault could reopen tomorrow, so the picker copies the file
//! into the app's own directory and answers with that path. The box is then a
//! normal local file — the copy that gets unlocked, edited and saved.

use crate::shared::android;
use std::path::PathBuf;

pub fn import() -> Result<PathBuf, String> {
    match android::text_with("pick", "*/*")? {
        Some(path) => Ok(PathBuf::from(path)),
        None => Err("no box file chosen".to_owned()),
    }
}
