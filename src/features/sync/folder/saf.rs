//! The folder mirror over Android's Storage Access Framework: a tree the user
//! picked once, written through the platform instead of through a path — the
//! folder may be Drive's or a card's, and only `DocumentsContract` knows which.
//!
//! The class is `dev.pandora.Pandora`, and the calls are:
//!
//! * `treeList(String tree) : String?` — the names in the tree, one per line.
//! * `treeRead(String arg) : byte[]?` — the bytes filed under the name in `arg`.
//! * `treeWrite(String arg, byte[] bytes) : String?` — writes in place, answering
//!   `null`, `moved` when the stamp is not the expected one, or the platform's
//!   own sentence saying why it would not.
//! * `treeStamp(String arg) : String?` — epoch milliseconds, or `null`.
//! * `treeDelete(String arg) : String?` — drops that name.
//! * `treeWritable(String tree) : String?` — `"1"`, `"0"`, or `null` when unknown.

use crate::features::sync::folder::{Folder, Guard, Write};
use crate::shared::android;
use crate::shared::i18n::{t, tf};

/// A tree the user picked in the system's picker. The uri is kept on this side:
/// the saved remote is the one place that knows which folder a mirror is.
pub struct Safe {
    uri: String,
}

impl Safe {
    pub fn new(uri: impl Into<String>) -> Self {
        Self { uri: uri.into() }
    }

    /// The tree and a name as the one string a platform call carries. A content
    /// uri is escaped, so the newline separates and never occurs in either.
    fn arg(&self, name: &str) -> String {
        format!("{}\n{name}", self.uri)
    }
}

impl Folder for Safe {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        android::bytes_with("treeRead", &self.arg(name))
    }

    fn stamp(&self, name: &str) -> Result<Option<u64>, String> {
        let Some(millis) = android::text_with("treeStamp", &self.arg(name))? else {
            return Ok(None);
        };
        Ok(millis.parse().ok())
    }

    fn write(&self, name: &str, bytes: &[u8], guard: Guard) -> Result<Write, String> {
        let expected = match guard {
            Guard::Missing => "-1".to_owned(),
            Guard::Stamp(stamp) => stamp.to_string(),
            Guard::Unguarded => "-2".to_owned(),
        };
        let arg = format!("{}\n{name}\n{expected}", self.uri);
        match android::send_bytes_with("treeWrite", &arg, bytes)? {
            Some(why) if why == "moved" => Ok(Write::Moved),
            Some(why) => Err(why),
            None => Ok(Write::Filed),
        }
    }

    fn writable(&self) -> Result<bool, String> {
        Ok(android::text_with("treeWritable", &self.uri)?.as_deref() != Some("0"))
    }

    fn list(&self) -> Result<Vec<String>, String> {
        let listed = android::text_with("treeList", &self.uri)?
            .ok_or_else(|| t("the folder could not be listed").to_owned())?;
        Ok(listed
            .lines()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        match android::text_with("treeDelete", &self.arg(name))? {
            Some(why) if why.is_empty() => Ok(()),
            Some(why) => Err(tf(
                "the folder could not drop {name}: {why}",
                &[("name", &name), ("why", &why)],
            )),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A debug probe, not a test: it asks a real provider what it answers and
    /// asserts nothing, so a plain `cargo test` stays green. `PANDORA_SAF_TREE`
    /// names the tree (the uri the picker hands back); `PANDORA_SAF_PROBE_WRITE=1`
    /// also writes and drops a throwaway file, which is the only way to see a
    /// provider refuse creation, overwriting or deletion on a phone.
    ///
    /// Run it on a device by hand:
    ///
    /// ```text
    /// PANDORA_SAF_TREE=content://... PANDORA_SAF_PROBE_WRITE=1 \
    ///     cargo test --target aarch64-linux-android -- --ignored probe_saf --nocapture
    /// ```
    #[test]
    #[ignore = "debug probe: talks to a real SAF provider"]
    fn probe_saf() {
        let Ok(uri) = std::env::var("PANDORA_SAF_TREE") else {
            println!("set PANDORA_SAF_TREE=content://... to a tree the picker returned");
            return;
        };
        let safe = Safe::new(uri);
        println!("writable: {:?}", safe.writable());
        println!("list: {:?}", safe.list());
        println!(
            "stamp: {:?}",
            safe.stamp(crate::features::sync::folder::NAME)
        );
        println!(
            "read: {:?}",
            safe.read(crate::features::sync::folder::NAME)
                .map(|bytes| bytes.map(|bytes| bytes.len()))
        );

        if std::env::var("PANDORA_SAF_PROBE_WRITE").as_deref() != Ok("1") {
            println!("set PANDORA_SAF_PROBE_WRITE=1 to write and drop a throwaway file");
            return;
        }
        let name = "pandora-saf-probe.pandora";
        let _ = safe.remove(name);
        println!(
            "write: {:?}",
            safe.write(name, b"pandora saf probe", Guard::Missing)
        );
        println!("stamp after: {:?}", safe.stamp(name));
        println!("read after: {:?}", safe.read(name));
        println!(
            "moved: {:?}",
            safe.write(name, b"not that", Guard::Stamp(1))
        );
        println!("remove: {:?}", safe.remove(name));
    }
}
