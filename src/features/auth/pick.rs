//! Android's way in for a box file that lives somewhere else on the phone.
//!
//! There is no dialog to open: the activity hands out a `content://` URI, which
//! is not a path the vault could reopen tomorrow, so the picker copies the file
//! into the app's own directory and answers with that path. The box is then a
//! normal local file — the copy that gets unlocked, edited and saved.
//!
//! The copy carries the document it came from. Otherwise nothing on this phone can
//! tell that the desk wrote a newer box three weeks ago, and the old copy opens
//! looking exactly like the newest one.

use crate::features::auth::model::Origin;
use crate::shared::android;
use std::path::PathBuf;

pub fn import() -> Result<(PathBuf, Option<Origin>), String> {
    match android::text_with("pick", "*/*")? {
        Some(path) => {
            let origin = android::picked_origin()?.map(|(uri, stamp)| Origin { uri, stamp });
            Ok((PathBuf::from(path), origin))
        }
        None => Err("no box file chosen".to_owned()),
    }
}
