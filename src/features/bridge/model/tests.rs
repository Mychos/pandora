use super::*;
use crate::app::{Field, Session};
use crate::bridge::serve;
use crate::features::boxes::model::Tab;
use crate::features::entries::model::{Axis, View};
use crate::vault::model::{Login, Vault};
use crate::vault::store::Credentials;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// One answer off the boxes the test opened, with no box picked: the box on screen
/// speaks, exactly as it does before the browser chooses one.
fn answer(request: Request, boxes: &mut Boxes) -> Response {
    logic::answer(request, boxes, &mut None)
}

/// The box on screen, for a test that reads one back after an answer.
fn shown(boxes: &Boxes) -> &Vault {
    &boxes.session.as_ref().expect("a box is open").vault
}

const SECRET: &str = "apple";

/// How long a test waits on the browser's own thread. The answer comes off a
/// Unix socket read by a thread of its own, so the wait is wall clock: counting
/// frames makes the test's patience depend on how busy the machine is, and this
/// suite runs hundreds of tests at once.
const PATIENCE: Duration = Duration::from_secs(10);
const BROWSER: &str = "chrome-extension://faked/";

/// One open box, the way the app hands it to the bridge.
fn open(items: &[(&str, &str, &str)]) -> Boxes {
    Boxes {
        session: Some(session(items)),
        ..Default::default()
    }
}

fn session(items: &[(&str, &str, &str)]) -> Session {
    let mut vault = Vault::new("Box");
    for (name, username, url) in items {
        let mut item = Login::new(1);
        item.name = (*name).to_owned();
        item.username = (*username).to_owned();
        item.password = Zeroizing::new(SECRET.to_owned());
        item.url = (*url).to_owned();
        vault.logins.push(item);
    }
    Session {
        path: PathBuf::from("/tmp/pandora-bridge-test.pandora"),
        creds: Credentials::rekey("test", None).unwrap(),
        vault,
        parked: false,
        parked_seq: 0,
        dirty: false,
        immediate: false,
        last_edit: None,
        file_print: None,
        last_mtime: None,
        conflict: false,
        remote_vault: false,
        remote: None,
        baseline: None,
        saved: None,
        push_at: None,
        behind: false,
        selected: None,
        search: String::new(),
        last_delete: None,
        show_trash: false,
        reveal: HashSet::<(String, Field)>::new(),
        show_new: false,
        show_qr: false,
        scan: None,
        confirm: None,
        tab: Tab::Passwords,
        folder: None,
        collapsed: HashSet::new(),
        view: View::List,
        axis: Axis::Target,
        at: Vec::new(),
        editing_folder: None,
        new_folder: None,
        editing_entry: None,
        moving_entry: None,
        meta_pending: false,
        load_pending: false,
        save_pending: false,
        edit_version: 0,
        save_version: 0,
    }
}

fn tmp() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("pandora-bridge-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("bridge.sock");
    (dir, socket)
}

/// The list of linked browsers has to name them honestly: a Chromium origin
/// cannot say Chrome from Brave, and Firefox names its add-on.
#[test]
fn a_caller_reads_as_the_browser_family_that_named_it() {
    assert_eq!(
        browser("chrome-extension://faked/"),
        ("Chromium browser", Some("Chrome, Brave or Edge"))
    );
    assert_eq!(browser(FIREFOX), ("Firefox", None));
    assert_eq!(browser(UNKNOWN), ("Unnamed browser", None));
    assert_eq!(
        browser("something-else"),
        ("Browser", Some("something-else"))
    );
}

/// A stand-in browser: one request, then whatever comes back.
fn ask(socket: &Path, message: &str) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let socket = socket.to_path_buf();
    let message = message.to_owned();
    std::thread::spawn(move || {
        let mut stream = crate::bridge::socket::connect_at(&socket).unwrap();
        protocol::write(&mut stream, message.as_bytes()).unwrap();
        let reply = protocol::read(&mut stream).unwrap().unwrap_or_default();
        let _ = tx.send(String::from_utf8_lossy(&reply).into_owned());
    });
    rx
}

/// Pump until the bridge has taken the connection, or give up.
fn settle(bridge: &mut Bridge, boxes: &mut Boxes, prefs: &Prefs) -> Effect {
    let mut effect = Effect::None;
    for _ in 0..100 {
        let next = bridge.pump(boxes, prefs);
        if next != Effect::None {
            effect = next;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    effect
}

#[test]
fn a_locked_app_answers_locked() {
    let mut boxes = Boxes::default();
    for request in [
        Request::Entries,
        Request::Reveal {
            id: "x".to_owned(),
            box_path: None,
        },
        Request::Match {
            host: Some("example.com".to_owned()),
            url: None,
        },
    ] {
        assert!(
            matches!(
                answer(request, &mut boxes),
                Response::Error { ref message } if message == "locked"
            ),
            "a box the app does not have open is locked"
        );
    }
    // Status answers rather than errors — "locked" is an answer to that one.
    assert!(matches!(
        answer(Request::Status, &mut boxes),
        Response::Status {
            unlocked: false,
            ..
        }
    ));
}

/// A listing, a match, a save and a status never carry a secret — passwords and
/// one-time codes alike. What leaves is either derived for a pick (`reveal`,
/// `code`) or made up on the spot (`generate`), and of those only `reveal` hands
/// back something the box stores.
#[test]
fn no_answer_but_a_pick_carries_a_secret() {
    let mut boxes = open(&[("GitHub", "octocat", "github.com")]);
    let id = shown(&boxes).logins[0].id.clone();

    let requests = [
        Request::Status,
        Request::Entries,
        Request::Codes,
        Request::Refresh,
        Request::Generate { length: None },
        Request::Match {
            host: Some("github.com".to_owned()),
            url: None,
        },
        Request::Update {
            id: id.clone(),
            username: None,
            password: None,
            box_path: None,
        },
    ];
    for request in requests {
        let json = serde_json::to_string(&answer(request, &mut boxes)).unwrap();
        assert!(!json.contains(SECRET), "{json}");
    }

    let json =
        serde_json::to_string(&answer(Request::Reveal { id, box_path: None }, &mut boxes)).unwrap();
    assert!(
        json.contains(SECRET),
        "reveal is the way out for a password"
    );
}

/// An answer never repeats what the caller put in it — the one place that
/// could is the fallback for a request neither peer serves, and it is a fixed
/// sentence. What a caller sends can be a password.
#[test]
fn an_answer_never_repeats_what_the_caller_sent() {
    const SENT: &str = "hunter2-never-in-a-reply";
    let mut boxes = open(&[]);

    let json = serde_json::to_string(&answer(
        Request::Update {
            id: "no such entry".to_owned(),
            username: Some(SENT.to_owned()),
            password: Some(SENT.to_owned()),
            box_path: None,
        },
        &mut boxes,
    ))
    .unwrap();
    assert!(!json.contains(SENT), "the request came back out: {json}");

    // And the fallback cannot interpolate anything, whoever adds a request
    // variant next and forgets to answer it.
    let fallback =
        crate::shared::i18n::t("the extension asked for something Pandora does not serve");
    assert!(
        !fallback.contains('{') && !fallback.contains(":?"),
        "the unsupported-request answer has no place for the request: {fallback}"
    );
}

#[test]
fn the_app_refuses_the_requests_its_window_owns() {
    let mut boxes = open(&[]);
    assert!(matches!(
        answer(
            Request::Unlock {
                path: "/tmp/x.pandora".to_owned(),
                password: "pw".to_owned(),
            },
            &mut boxes
        ),
        Response::Error { .. }
    ));
    // What the app does answer is which boxes it has open, so the browser can pick
    // one: opening a box is still the window's job.
    let listed = answer(Request::ListBoxes, &mut boxes);
    assert!(
        matches!(
            &listed,
            Response::Boxes { boxes, picked: Some(path) }
                if boxes.len() == 1 && path == "/tmp/pandora-bridge-test.pandora"
        ),
        "the one open box is listed, and it is the one the browser reads: {listed:?}"
    );
}

#[test]
fn a_save_from_the_browser_lands_in_the_open_box() {
    let mut boxes = open(&[]);
    let reply = answer(
        Request::Create {
            name: "Example".to_owned(),
            username: "eve".to_owned(),
            password: "typed".to_owned(),
            url: "https://example.com/login".to_owned(),
        },
        &mut boxes,
    );
    assert!(matches!(reply, Response::Saved { .. }));
    assert!(
        boxes.session.as_ref().unwrap().dirty,
        "the app's own autosave is what writes it"
    );
    assert_eq!(shown(&boxes).logins.len(), 1);
    assert_eq!(
        serve::matches(shown(&boxes), "https://example.com/session").len(),
        1,
        "and it claims the page it came from"
    );
}

#[test]
fn an_update_keeps_what_it_was_not_sent() {
    let mut boxes = open(&[("GitHub", "octocat", "github.com")]);
    let id = shown(&boxes).logins[0].id.clone();

    answer(
        Request::Update {
            id: id.clone(),
            username: None,
            password: Some("rotated".to_owned()),
            box_path: None,
        },
        &mut boxes,
    );
    let item = &shown(&boxes).logins[0];
    assert_eq!(item.username, "octocat", "not sent, so it stays");
    assert_eq!(item.password.as_str(), "rotated");

    assert!(matches!(
        answer(
            Request::Update {
                id: "nope".to_owned(),
                username: None,
                password: None,
                box_path: None,
            },
            &mut boxes
        ),
        Response::Error { .. }
    ));
}

#[test]
fn an_unapproved_browser_waits_for_the_prompt() {
    let (dir, socket) = tmp();
    let mut bridge = Bridge::start_at(&socket);
    let mut boxes = open(&[("GitHub", "octocat", "github.com")]);
    let mut prefs = Prefs::default();

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":1,"caller":"{BROWSER}","type":"status"}}"#),
    );
    assert_eq!(settle(&mut bridge, &mut boxes, &prefs), Effect::None);

    assert_eq!(bridge.asking(), Some(BROWSER));
    assert!(
        reply.try_recv().is_err(),
        "nothing is answered until the user says so"
    );

    assert!(
        bridge.resolve(true, true, &mut boxes, &mut prefs),
        "the caller is told to write the approval down"
    );
    assert!(prefs.browsers.contains(&BROWSER.to_owned()));
    let answered = reply.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(answered.contains("\"unlocked\":true"), "{answered}");
    assert!(answered.contains("\"_id\":1"), "the pairing id rides back");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_refused_browser_is_told_no_and_remembers_nothing() {
    let (dir, socket) = tmp();
    let mut bridge = Bridge::start_at(&socket);
    let mut boxes = open(&[]);
    let mut prefs = Prefs::default();

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":2,"caller":"{BROWSER}","type":"entries"}}"#),
    );
    assert_eq!(settle(&mut bridge, &mut boxes, &prefs), Effect::None);

    assert!(!bridge.resolve(false, true, &mut boxes, &mut prefs));
    assert!(prefs.browsers.is_empty(), "a refusal is not remembered");
    let answered = reply.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(answered.contains("refused"), "{answered}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_approved_browser_is_answered_without_a_prompt() {
    let (dir, socket) = tmp();
    let mut bridge = Bridge::start_at(&socket);
    let mut boxes = open(&[("GitHub", "octocat", "github.com")]);
    let prefs = Prefs {
        browsers: vec![BROWSER.to_owned()],
        ..Prefs::default()
    };

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":3,"caller":"{BROWSER}","type":"match","url":"https://github.com/x"}}"#),
    );
    assert_eq!(
        settle(&mut bridge, &mut boxes, &prefs),
        Effect::Activity,
        "approved browser activity refreshes the away timer"
    );

    assert_eq!(bridge.asking(), None, "nobody was asked");
    let answered = reply.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(answered.contains("octocat"), "{answered}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A browser can knock while the window is idle and egui is drawing nothing. The
/// reader has to wake the frame, or the answer never comes and the panel spins on
/// "Loading…" — which is what a locked box looked like.
#[test]
fn a_browser_knock_wakes_the_idle_frame() {
    let ctx = egui::Context::default();
    let (dir, socket) = tmp();
    let _bridge = Bridge::listen_at(&socket, Some(ctx.clone()));

    // A browser knocks and goes. The reader takes the empty connection — the
    // request itself does not matter, only that the frame is woken for it.
    drop(crate::bridge::socket::connect_at(&socket).unwrap());

    for _ in 0..100 {
        if ctx.has_requested_repaint() {
            let _ = std::fs::remove_dir_all(dir);
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("a browser knock must wake the frame that answers it");
}

/// A save from the panel changes the box, so the frame says so: the app turns
/// this into a notice, where a read is only activity.
#[test]
fn a_write_from_the_browser_is_worth_telling_the_app() {
    let (dir, socket) = tmp();
    let mut bridge = Bridge::start_at(&socket);
    let mut boxes = open(&[]);
    let prefs = Prefs {
        browsers: vec![BROWSER.to_owned()],
        ..Prefs::default()
    };

    let reply = ask(
        &socket,
        &format!(
            r#"{{"_id":6,"caller":"{BROWSER}","type":"create","name":"Example","username":"eve","password":"typed","url":"https://example.com/login"}}"#
        ),
    );
    assert_eq!(
        settle(&mut bridge, &mut boxes, &prefs),
        Effect::Wrote,
        "a browser save is the effect the app reports"
    );
    assert!(
        boxes.session.as_ref().unwrap().dirty,
        "and it landed in the open box"
    );
    let _ = reply.recv_timeout(Duration::from_secs(5)).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_prompt_nobody_answers_expires() {
    let (dir, socket) = tmp();
    let mut bridge = Bridge::start_at(&socket);
    let mut boxes = open(&[]);
    let prefs = Prefs::default();

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":4,"caller":"{BROWSER}","type":"entries"}}"#),
    );
    settle(&mut bridge, &mut boxes, &prefs);
    assert_eq!(bridge.asking(), Some(BROWSER));

    bridge.expire_at(Instant::now() + APPROVAL);
    assert_eq!(bridge.asking(), None, "the prompt is gone");
    let answered = reply.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(answered.contains("in time"), "{answered}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The wiring: the app's own frame has to call the bridge, and the answer has to
/// come off the box that is open on screen. This is the closest thing to the app
/// under a real window that a headless test can do.
#[test]
fn the_app_answers_a_browser_through_its_own_loop() {
    use crate::app::App;
    use crate::shared::theme;
    use crate::shared::ui::shot;

    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    // The app has a box open — and it is the one the browser is answered from.
    app.open("Relayed");
    app.prefs = Prefs {
        browsers: vec![BROWSER.to_owned()],
        ..Prefs::default()
    };
    app.shell.activity = Some(Instant::now() - Duration::from_secs(60));
    let (dir, socket) = tmp();
    app.bridge = Bridge::start_at(&socket);

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":9,"caller":"{BROWSER}","type":"status"}}"#),
    );
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        let mut input = shot::input(egui::vec2(1000.0, 700.0), Vec::new());
        input.focused = false;
        let mut output = ctx.run_ui(input, |ui| app.run_ui(ui));
        output.textures_delta.clear();
        if let Ok(answered) = reply.try_recv() {
            assert!(answered.contains("\"unlocked\":true"), "{answered}");
            assert!(answered.contains("\"name\":\"Relayed\""), "{answered}");
            assert!(
                app.shell.activity.unwrap().elapsed() < Duration::from_secs(1),
                "browser activity refreshes the inactivity timer"
            );
            let _ = std::fs::remove_dir_all(dir);
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the app never answered through its own loop in {PATIENCE:?}");
}

/// The prompt is the one place a browser is let in, so what it draws matters: both
/// answers, and the affirmative on the right, where the app's other dialogs put it.
#[test]
fn the_prompt_draws_both_answers() {
    use crate::app::App;
    use crate::shared::theme;
    use crate::shared::ui::shot;

    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.open("Box");
    let (dir, socket) = tmp();
    app.bridge = Bridge::start_at(&socket);

    let _asked = ask(
        &socket,
        &format!(r#"{{"_id":1,"caller":"{BROWSER}","type":"status"}}"#),
    );
    let deadline = Instant::now() + PATIENCE;
    while app.bridge.asking().is_none() && Instant::now() < deadline {
        shot::frame(&ctx, egui::vec2(900.0, 640.0), Vec::new(), |ui| {
            app.run_ui(ui)
        });
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(app.bridge.asking(), Some(BROWSER), "the prompt is up");

    let shapes = shot::frame(&ctx, egui::vec2(900.0, 640.0), Vec::new(), |ui| {
        app.run_ui(ui)
    });
    let at = |text: &str| {
        shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(s.shape.visual_bounding_rect().center().x)
            }
            _ => None,
        })
    };
    let allow = at("Allow").expect("the affirmative is drawn");
    let deny = at("Deny").expect("the safe answer is drawn");
    assert!(allow > deny, "Allow sits to the right of Deny");
    let _ = std::fs::remove_dir_all(dir);
}

/// A page's fill spans every box the app has open, and each row says which box it
/// came from: a page has no idea which box holds what.
#[test]
fn a_fill_spans_every_open_box_and_says_which() {
    let mut boxes = open(&[("Work", "me", "example.com")]);
    let mut second = session(&[("Bank", "me", "example.com")]);
    second.path = PathBuf::from("/tmp/pandora-bridge-second.pandora");
    second.vault.name = "Second".to_owned();
    boxes.tabs.push(second);

    let reply = answer(
        Request::Match {
            host: Some("example.com".to_owned()),
            url: None,
        },
        &mut boxes,
    );
    let Response::Entries { entries } = reply else {
        panic!("a match lists entries");
    };
    assert_eq!(
        entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["Work", "Bank"],
        "both boxes answered, the one on screen first"
    );
    assert_eq!(entries[0].box_name.as_deref(), Some("Box"));
    assert_eq!(entries[1].box_name.as_deref(), Some("Second"));
    assert_eq!(
        entries[1].box_path.as_deref(),
        Some("/tmp/pandora-bridge-second.pandora")
    );
}

/// A row listed from another box reads that box: the entry's id alone would land on
/// whichever box happens to answer next.
#[test]
fn a_row_from_another_box_reads_that_box() {
    let mut boxes = open(&[("Work", "me", "example.com")]);
    let mut second = session(&[("Bank", "me", "example.com")]);
    second.path = PathBuf::from("/tmp/pandora-bridge-other.pandora");
    let id = second.vault.logins[0].id.clone();
    // The other box's own secret: only the box the row came from decides which one.
    second.vault.logins[0].password = Zeroizing::new("other-secret".to_owned());
    boxes.tabs.push(second);

    let reply = answer(
        Request::Reveal {
            id: id.clone(),
            box_path: Some("/tmp/pandora-bridge-other.pandora".to_owned()),
        },
        &mut boxes,
    );
    assert!(
        matches!(reply, Response::Secret { ref password, .. } if password == "other-secret"),
        "the row's own box answered"
    );

    let reply = answer(
        Request::Reveal {
            id: shown(&boxes).logins[0].id.clone(),
            box_path: None,
        },
        &mut boxes,
    );
    assert!(
        matches!(reply, Response::Secret { ref password, .. } if password == SECRET),
        "with no box named, the one on screen answers"
    );
}

/// The browser reads from the box it picked; a tab closed under the pick falls back
/// to the one on screen rather than answering nothing.
#[test]
fn the_browser_reads_the_box_it_picked() {
    let mut boxes = open(&[]);
    let mut second = session(&[]);
    second.path = PathBuf::from("/tmp/pandora-bridge-picked.pandora");
    second.vault.name = "Picked".to_owned();
    boxes.tabs.push(second);
    let mut picked = None;

    assert!(matches!(
        logic::answer(
            Request::Select {
                path: "/tmp/pandora-bridge-picked.pandora".to_owned()
            },
            &mut boxes,
            &mut picked
        ),
        Response::Ok
    ));
    assert!(
        matches!(
            logic::answer(Request::Refresh, &mut boxes, &mut picked),
            Response::Unlocked { ref name, .. } if name == "Picked"
        ),
        "the picked box answers, not the one on screen"
    );
    assert!(matches!(
        logic::answer(Request::ListBoxes, &mut boxes, &mut picked),
        Response::Boxes {
            picked: Some(ref path),
            ..
        } if path == "/tmp/pandora-bridge-picked.pandora"
    ));

    boxes.tabs.clear();
    assert!(
        matches!(
            logic::answer(Request::Refresh, &mut boxes, &mut picked),
            Response::Unlocked { ref name, .. } if name == "Box"
        ),
        "a pick whose box closed reads the one on screen"
    );
    assert!(
        matches!(
            logic::answer(Request::ListBoxes, &mut boxes, &mut picked),
            Response::Boxes {
                picked: Some(ref path),
                ..
            } if path == "/tmp/pandora-bridge-test.pandora"
        ),
        "the listing names the box on screen once the pick's box is gone"
    );
    assert!(matches!(
        logic::answer(
            Request::Select {
                path: "/tmp/never-open.pandora".to_owned()
            },
            &mut boxes,
            &mut picked
        ),
        Response::Error { .. }
    ));
}

/// A parked tab whose box cannot be read says why: a placeholder vault is never the
/// answer to a browser.
#[test]
fn a_parked_tab_that_cannot_be_read_is_an_error() {
    let mut boxes = open(&[]);
    let mut second = session(&[]);
    second.path = PathBuf::from("/tmp/pandora-bridge-missing.pandora");
    second.parked = true;
    boxes.tabs.push(second);
    let mut picked = Some(PathBuf::from("/tmp/pandora-bridge-missing.pandora"));

    assert!(matches!(
        logic::answer(Request::Refresh, &mut boxes, &mut picked),
        Response::Error { .. }
    ));
}
