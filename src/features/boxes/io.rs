//! Background I/O for the vault file: modified_time, load and save never run on
//! the UI thread, so a vault that lives on a slow (network) filesystem cannot
//! freeze the interface.

use crate::vault::model::Vault;
use crate::vault::store;
use crate::vault::store::Credentials;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

pub enum Request {
    Meta {
        path: PathBuf,
    },
    Load {
        path: PathBuf,
        creds: Credentials,
    },
    Save {
        path: PathBuf,
        vault: Box<Vault>,
        creds: Credentials,
    },
}

pub enum Reply {
    Meta {
        path: PathBuf,
        mtime: Option<SystemTime>,
        slow: bool,
    },
    Load {
        path: PathBuf,
        result: Result<Box<Vault>, String>,
    },
    Save {
        path: PathBuf,
        result: Result<(), String>,
        vault: Box<Vault>,
        mtime: Option<SystemTime>,
    },
}

pub struct Io {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    _thread: JoinHandle<()>,
}

#[allow(clippy::new_without_default)]
impl Io {
    pub fn new() -> Self {
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
        let thread = thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                match req {
                    Request::Meta { path } => {
                        let start = std::time::Instant::now();
                        let mtime = store::modified_time(&path);
                        let slow = start.elapsed() > std::time::Duration::from_millis(50);
                        let _ = reply_tx.send(Reply::Meta { path, mtime, slow });
                    }
                    Request::Load { path, creds } => {
                        let result = store::load(&path, &creds).map(Box::new);
                        let _ = reply_tx.send(Reply::Load { path, result });
                    }
                    Request::Save { path, vault, creds } => {
                        let mut vault = *vault;
                        let result = store::save(&path, &mut vault, &creds);
                        let mtime = if result.is_ok() {
                            store::modified_time(&path)
                        } else {
                            None
                        };
                        let _ = reply_tx.send(Reply::Save {
                            path,
                            result,
                            vault: Box::new(vault),
                            mtime,
                        });
                    }
                }
            }
        });
        Self {
            tx: req_tx,
            rx: reply_rx,
            _thread: thread,
        }
    }

    pub fn request(&self, req: Request) {
        let _ = self.tx.send(req);
    }

    pub fn drain(&self) -> Vec<Reply> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            out.push(r);
        }
        out
    }
}
