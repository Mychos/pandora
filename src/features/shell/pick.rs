//! One file dialog at a time, off the frame thread: while the system picker is
//! open the window keeps painting instead of going "not responding".

use crate::app::App;
use crate::shared::i18n::t;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

/// Redraw pace while a picker is out; the answer arrives on its own thread.
const STEP: Duration = Duration::from_millis(80);

/// What runs with the answer once the picker closes.
type Then = Box<dyn FnOnce(&mut App, Vec<PathBuf>)>;

pub struct Ask {
    rx: Receiver<Vec<PathBuf>>,
    then: Then,
}

impl App {
    /// Open a system picker on its own thread; `then` lands its answer on a later
    /// frame. A second ask while one is out is dropped — one modal at a time.
    pub fn ask(
        &mut self,
        ctx: &egui::Context,
        dialog: impl FnOnce() -> Vec<PathBuf> + Send + 'static,
        then: impl FnOnce(&mut App, Vec<PathBuf>) + 'static,
    ) {
        launch(&mut self.ask, ctx, dialog, then);
    }

    /// Land a finished picker; the ask is taken before `then` runs, so a landing
    /// that opens another dialog is not wiped on return.
    pub fn pump_pick(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.ask.as_ref() else {
            return;
        };
        match ask.rx.try_recv() {
            Ok(paths) => {
                if let Some(ask) = self.ask.take() {
                    (ask.then)(self, paths);
                }
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(STEP),
            Err(TryRecvError::Disconnected) => {
                self.ask = None;
                self.shell.fail(t("the file dialog was interrupted"));
            }
        }
    }
}

/// The ask without an app to hold it: a widget that only has the slot can still put
/// a dialog out, and its answer lands in the context's temp data.
pub fn launch(
    slot: &mut Option<Ask>,
    ctx: &egui::Context,
    dialog: impl FnOnce() -> Vec<PathBuf> + Send + 'static,
    then: impl FnOnce(&mut App, Vec<PathBuf>) + 'static,
) {
    if slot.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(dialog());
    });
    *slot = Some(Ask {
        rx,
        then: Box::new(then),
    });
    ctx.request_repaint();
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use crate::features::shell::model::StatusKind;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    /// Pump until the ask is gone, so a worker thread has time to answer.
    fn settle(app: &mut App, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.ask.is_some() && Instant::now() < deadline {
            app.pump_pick(ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.ask.is_none(), "the ask never came back");
    }

    #[test]
    fn an_ask_lands_on_a_later_frame_and_only_once() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        let landed = Arc::new(AtomicUsize::new(0));
        let ignored = Arc::new(AtomicUsize::new(0));
        let (go, wait) = mpsc::channel::<()>();
        let (first, second) = (landed.clone(), ignored.clone());
        app.ask(
            &ctx,
            move || {
                let _ = wait.recv();
                vec![PathBuf::from("copy.csv")]
            },
            move |_, paths| {
                assert_eq!(paths, vec![PathBuf::from("copy.csv")]);
                first.fetch_add(1, Ordering::SeqCst);
            },
        );
        app.ask(
            &ctx,
            || vec![PathBuf::from("other.csv")],
            move |_, _| {
                second.fetch_add(1, Ordering::SeqCst);
            },
        );
        app.pump_pick(&ctx);
        assert_eq!(
            landed.load(Ordering::SeqCst),
            0,
            "an ask does not land on the frame that opened it"
        );
        go.send(()).unwrap();
        settle(&mut app, &ctx);
        assert_eq!(landed.load(Ordering::SeqCst), 1, "the ask lands once");
        assert_eq!(
            ignored.load(Ordering::SeqCst),
            0,
            "a second ask while one is out is dropped"
        );
        app.pump_pick(&ctx);
        assert_eq!(
            landed.load(Ordering::SeqCst),
            1,
            "a landed ask is not run again"
        );
    }

    #[test]
    fn a_worker_that_dies_clears_the_ask_with_a_warning() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.ask(
            &ctx,
            || panic!("the picker crashed"),
            |_, _| panic!("a dead picker still lands"),
        );
        settle(&mut app, &ctx);
        let status = app.shell.status.as_ref().expect("a warning is shown");
        assert!(status.kind == StatusKind::Error, "the warning is an error");
        assert!(
            status.text.contains("interrupted"),
            "the warning says the dialog was interrupted"
        );
    }
}
