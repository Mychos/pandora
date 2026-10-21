//! The socket the app listens on, the connections waiting on an answer, and the
//! browsers the user has let in.
//!
//! Connections are read on their own threads: a local process that connects and then
//! says nothing must not park the frame that draws the window. Everything they read
//! comes back through a channel and is answered on the frame, so the vault is only
//! ever touched from the thread that owns it.

use super::logic;
use crate::bridge::protocol::{self, Request, Response};
use crate::bridge::socket::{Listener, Stream};
use crate::features::boxes::model::Session;
use crate::features::settings::model::Prefs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// How long a browser waits for the user to answer the prompt before it is told no.
/// The host waits a little longer than this, so the host never gives up first.
pub const APPROVAL: Duration = Duration::from_secs(30);

/// How long a connection gets to say something before its thread gives up.
const READ: Duration = Duration::from_secs(10);

/// Connections read at once. A local process opening thousands would otherwise be
/// thousands of threads; past this, the rest wait in the socket's own backlog.
const INFLIGHT: usize = 32;

/// When the browser told us nothing about who is calling.
const UNKNOWN: &str = "unknown";

/// The add-on id Firefox hands over; Chromium hands the extension's own origin.
const FIREFOX: &str = "pandora@pandora.dev";

/// How a caller the browser named reads to the user, with a note when the name
/// alone would mislead. A `chrome-extension://` origin is shared by Chrome, Brave
/// and Edge and the browser never says which, so it names the family, not the
/// browser.
pub fn browser(caller: &str) -> (&'static str, Option<&str>) {
    if caller.starts_with("chrome-extension://") {
        ("Chromium browser", Some("Chrome, Brave or Edge"))
    } else if caller == FIREFOX {
        ("Firefox", None)
    } else if caller == UNKNOWN {
        ("Unnamed browser", None)
    } else {
        ("Browser", Some(caller))
    }
}

struct Incoming {
    stream: Stream,
    caller: String,
    id: Option<u64>,
    request: Result<Request, String>,
}

struct Waiting {
    stream: Stream,
    caller: String,
    id: Option<u64>,
    request: Request,
    since: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    None,
    /// A browser asked (or was answered) without changing anything: refresh the
    /// away timer and nothing more.
    Activity,
    /// A browser wrote the box — a save or an update. Worth telling the user.
    Wrote,
    Lock,
}

pub struct Bridge {
    tx: Sender<Incoming>,
    rx: Receiver<Incoming>,
    inflight: Arc<AtomicUsize>,
    waiting: Vec<Waiting>,
    /// Where the prompt's "remember" box starts.
    pub remember: bool,
}

#[derive(Clone, Default)]
struct Wake(Option<egui::Context>);

impl Wake {
    fn ping(&self) {
        if let Some(ctx) = &self.0 {
            ctx.request_repaint();
        }
    }
}

impl Default for Bridge {
    /// Not listening: tests and layout fixtures want the shape, not a socket.
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            inflight: Arc::new(AtomicUsize::new(0)),
            waiting: Vec::new(),
            remember: true,
        }
    }
}

impl Bridge {
    /// Listen on the local socket. A second app on the same vault gets `None` back
    /// from `bind` and does not listen at all, rather than racing the first for every
    /// request.
    pub fn start(ctx: egui::Context) -> Self {
        Self::listen_at(&crate::bridge::socket::path(), Some(ctx))
    }

    /// Listen at one named socket, for whoever wants it somewhere else.
    pub fn start_at(path: &std::path::Path) -> Self {
        Self::listen_at(path, None)
    }

    fn listen_at(path: &std::path::Path, ctx: Option<egui::Context>) -> Self {
        let bridge = Self::default();
        if let Some(listener) = Listener::bind_at(path).ok().flatten() {
            bridge.accept(listener, Wake(ctx));
        }
        bridge
    }

    /// The browser waiting on the prompt, if the user is being asked about one.
    pub fn asking(&self) -> Option<&str> {
        self.waiting.first().map(|waiting| waiting.caller.as_str())
    }

    /// A frame of the bridge: take what the threads read, answer it, and give up on
    /// whoever took too long. Never blocks — that is the whole reason the reads are
    /// on threads.
    pub fn pump(&mut self, mut session: Option<&mut Session>, prefs: &Prefs) -> Effect {
        let mut effect = Effect::None;
        for incoming in self.rx.try_iter().collect::<Vec<_>>() {
            let Incoming {
                stream,
                caller,
                id,
                request,
            } = incoming;
            let request = match request {
                Ok(request) => request,
                Err(message) => {
                    self.reply(stream, id, &Response::Error { message });
                    continue;
                }
            };

            // A browser the user has not approved waits, with its connection open,
            // for the prompt.
            if !prefs.browsers.contains(&caller) {
                self.waiting.push(Waiting {
                    stream,
                    caller,
                    id,
                    request,
                    since: Instant::now(),
                });
                continue;
            }

            // Locking is the app's: it clears the session, which is what actually
            // drops the keys, so the answer alone would be a lie.
            if matches!(request, Request::Lock) {
                effect = Effect::Lock;
                self.reply(stream, id, &Response::Ok);
                continue;
            }

            let response = logic::answer(request, session.as_deref_mut());
            if effect != Effect::Lock {
                // A write is worth telling the app about; a read only refreshes the
                // away timer. A frame that already wrote keeps saying so.
                if matches!(response, Response::Saved { .. }) {
                    effect = Effect::Wrote;
                } else if effect == Effect::None {
                    effect = Effect::Activity;
                }
            }
            self.reply(stream, id, &response);
        }

        self.expire_at(Instant::now());
        effect
    }

    /// Answer the prompt: let the waiting browser in, or turn it away. Returns
    /// whether the approval list grew, so the caller writes it down — the model does
    /// not touch the settings file itself.
    pub fn resolve(
        &mut self,
        allow: bool,
        remember: bool,
        session: Option<&mut Session>,
        prefs: &mut Prefs,
    ) -> bool {
        if self.waiting.is_empty() {
            return false;
        }
        let waiting = self.waiting.remove(0);

        if !allow {
            self.reply(
                waiting.stream,
                waiting.id,
                &Response::Error {
                    message: "the Pandora app refused this browser".to_owned(),
                },
            );
            return false;
        }

        let mut remembered = false;
        if remember && !prefs.browsers.contains(&waiting.caller) {
            prefs.browsers.push(waiting.caller);
            remembered = true;
        }
        let response = if matches!(waiting.request, Request::Lock) {
            Response::Ok
        } else {
            logic::answer(waiting.request, session)
        };
        self.reply(waiting.stream, waiting.id, &response);
        remembered
    }

    /// One answer, then the connection closes. A small frame into a socket buffer, so
    /// it does not park the frame.
    fn reply(&self, mut stream: Stream, id: Option<u64>, response: &Response) {
        let _ = protocol::reply(&mut stream, id, response);
    }

    /// Take connections on a thread of its own, so a browser that knocks while the
    /// window is idle still gets answered: the reader wakes the frame that will do it.
    /// Past the cap the rest wait in the socket's own backlog rather than in a thread
    /// each. If this thread cannot start there is no listener to drain.
    fn accept(&self, listener: Listener, wake: Wake) {
        let tx = self.tx.clone();
        let inflight = Arc::clone(&self.inflight);
        let _ = thread::Builder::new()
            .name("pandora-bridge-listen".to_owned())
            .spawn(move || loop {
                while inflight.load(Ordering::Relaxed) >= INFLIGHT {
                    thread::sleep(Duration::from_millis(20));
                }
                let Ok(stream) = listener.accept() else {
                    break; // the listener is gone: nothing more will arrive
                };
                inflight.fetch_add(1, Ordering::Relaxed);
                let read = read_one(stream, tx.clone(), Arc::clone(&inflight), wake.clone());
                let spawned = thread::Builder::new()
                    .name("pandora-bridge".to_owned())
                    .spawn(read);
                if spawned.is_err() {
                    inflight.fetch_sub(1, Ordering::Relaxed);
                }
            });
    }

    /// Whoever nobody answered in time is told no and let go, so the browser is not
    /// left waiting on a prompt that is gone. The clock is a parameter because it is
    /// the only time-dependent thing here, and a test cannot wait half a minute.
    fn expire_at(&mut self, now: Instant) {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|waiting| now.duration_since(waiting.since) >= APPROVAL);
        self.waiting = kept;
        for waiting in gone {
            self.reply(
                waiting.stream,
                waiting.id,
                &Response::Error {
                    message: "the Pandora app did not answer in time".to_owned(),
                },
            );
        }
    }
}

/// The reader for one connection: the whole of it is one request, so the thread ends
/// as soon as it has handed it over. It pings the frame *after* queueing, so the
/// answer is drawn even if the window was idle and egui was not painting when the
/// browser knocked.
fn read_one(
    mut stream: Stream,
    tx: Sender<Incoming>,
    inflight: Arc<AtomicUsize>,
    wake: Wake,
) -> impl FnOnce() + Send + 'static {
    move || {
        let _ = stream.set_read_timeout(Some(READ));
        let (id, caller, request) = match protocol::read(&mut stream) {
            Ok(Some(bytes)) => {
                let envelope = protocol::parse(&bytes);
                (
                    envelope.id,
                    envelope.caller.unwrap_or_else(|| UNKNOWN.to_owned()),
                    envelope.request,
                )
            }
            Ok(None) => (None, UNKNOWN.to_owned(), Err("nothing was sent".to_owned())),
            Err(e) => (None, UNKNOWN.to_owned(), Err(format!("unreadable: {e}"))),
        };
        let _ = tx.send(Incoming {
            stream,
            caller,
            id,
            request,
        });
        wake.ping();
        inflight.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests;
