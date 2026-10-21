use super::*;
use crate::app::Field;
use crate::bridge::serve;
use crate::features::boxes::model::Tab;
use crate::features::entries::model::{Axis, View};
use crate::vault::model::{Login, Vault};
use crate::vault::store::Credentials;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const SECRET: &str = "apple";

/// How long a test waits on the browser's own thread. The answer comes off a
/// Unix socket read by a thread of its own, so the wait is wall clock: counting
/// frames makes the test's patience depend on how busy the machine is, and this
/// suite runs hundreds of tests at once.
const PATIENCE: Duration = Duration::from_secs(10);
const BROWSER: &str = "chrome-extension://faked/";

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
        last_edit: None,
        last_mtime: None,
        conflict: false,
        ignore_mtime: None,
        remote: None,
        baseline: None,
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
fn settle(bridge: &mut Bridge, session: &mut Session, prefs: &Prefs) -> Effect {
    let mut effect = Effect::None;
    for _ in 0..100 {
        let next = bridge.pump(Some(session), prefs);
        if next != Effect::None {
            effect = next;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    effect
}

#[test]
fn a_locked_app_answers_locked() {
    let mut none = None;
    for request in [
        Request::Entries,
        Request::Reveal { id: "x".to_owned() },
        Request::Match {
            host: Some("example.com".to_owned()),
            url: None,
        },
    ] {
        assert!(
            matches!(
                logic::answer(request, none.as_mut()),
                Response::Error { ref message } if message == "locked"
            ),
            "a box the app does not have open is locked"
        );
    }
    // Status answers rather than errors — "locked" is an answer to that one.
    assert!(matches!(
        logic::answer(Request::Status, none.as_mut()),
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
    let mut session = session(&[("GitHub", "octocat", "github.com")]);
    let id = session.vault.logins[0].id.clone();

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
        },
    ];
    for request in requests {
        let json = serde_json::to_string(&logic::answer(request, Some(&mut session))).unwrap();
        assert!(!json.contains(SECRET), "{json}");
    }

    let json =
        serde_json::to_string(&logic::answer(Request::Reveal { id }, Some(&mut session))).unwrap();
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
    let mut session = session(&[]);

    let json = serde_json::to_string(&logic::answer(
        Request::Update {
            id: "no such entry".to_owned(),
            username: Some(SENT.to_owned()),
            password: Some(SENT.to_owned()),
        },
        Some(&mut session),
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
    let mut session = session(&[]);
    assert!(matches!(
        logic::answer(
            Request::Unlock {
                path: "/tmp/x.pandora".to_owned(),
                password: "pw".to_owned(),
            },
            Some(&mut session)
        ),
        Response::Error { .. }
    ));
    assert!(matches!(
        logic::answer(Request::ListBoxes, Some(&mut session)),
        Response::Error { .. }
    ));
}

#[test]
fn a_save_from_the_browser_lands_in_the_open_box() {
    let mut session = session(&[]);
    let reply = logic::answer(
        Request::Create {
            name: "Example".to_owned(),
            username: "eve".to_owned(),
            password: "typed".to_owned(),
            url: "https://example.com/login".to_owned(),
        },
        Some(&mut session),
    );
    assert!(matches!(reply, Response::Saved { .. }));
    assert!(session.dirty, "the app's own autosave is what writes it");
    assert_eq!(session.vault.logins.len(), 1);
    assert_eq!(
        serve::matches(&session.vault, "https://example.com/session").len(),
        1,
        "and it claims the page it came from"
    );
}

#[test]
fn an_update_keeps_what_it_was_not_sent() {
    let mut session = session(&[("GitHub", "octocat", "github.com")]);
    let id = session.vault.logins[0].id.clone();

    logic::answer(
        Request::Update {
            id: id.clone(),
            username: None,
            password: Some("rotated".to_owned()),
        },
        Some(&mut session),
    );
    let item = &session.vault.logins[0];
    assert_eq!(item.username, "octocat", "not sent, so it stays");
    assert_eq!(item.password.as_str(), "rotated");

    assert!(matches!(
        logic::answer(
            Request::Update {
                id: "nope".to_owned(),
                username: None,
                password: None,
            },
            Some(&mut session)
        ),
        Response::Error { .. }
    ));
}

#[test]
fn an_unapproved_browser_waits_for_the_prompt() {
    let (dir, socket) = tmp();
    let mut bridge = Bridge::start_at(&socket);
    let mut session = session(&[("GitHub", "octocat", "github.com")]);
    let mut prefs = Prefs::default();

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":1,"caller":"{BROWSER}","type":"status"}}"#),
    );
    assert_eq!(settle(&mut bridge, &mut session, &prefs), Effect::None);

    assert_eq!(bridge.asking(), Some(BROWSER));
    assert!(
        reply.try_recv().is_err(),
        "nothing is answered until the user says so"
    );

    assert!(
        bridge.resolve(true, true, Some(&mut session), &mut prefs),
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
    let mut session = session(&[]);
    let mut prefs = Prefs::default();

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":2,"caller":"{BROWSER}","type":"entries"}}"#),
    );
    assert_eq!(settle(&mut bridge, &mut session, &prefs), Effect::None);

    assert!(!bridge.resolve(false, true, Some(&mut session), &mut prefs));
    assert!(prefs.browsers.is_empty(), "a refusal is not remembered");
    let answered = reply.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(answered.contains("refused"), "{answered}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_approved_browser_is_answered_without_a_prompt() {
    let (dir, socket) = tmp();
    let mut bridge = Bridge::start_at(&socket);
    let mut session = session(&[("GitHub", "octocat", "github.com")]);
    let prefs = Prefs {
        browsers: vec![BROWSER.to_owned()],
        ..Prefs::default()
    };

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":3,"caller":"{BROWSER}","type":"match","url":"https://github.com/x"}}"#),
    );
    assert_eq!(
        settle(&mut bridge, &mut session, &prefs),
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
    let mut session = session(&[]);
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
        settle(&mut bridge, &mut session, &prefs),
        Effect::Wrote,
        "a browser save is the effect the app reports"
    );
    assert!(session.dirty, "and it landed in the open box");
    let _ = reply.recv_timeout(Duration::from_secs(5)).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_prompt_nobody_answers_expires() {
    let (dir, socket) = tmp();
    let mut bridge = Bridge::start_at(&socket);
    let mut session = session(&[]);
    let prefs = Prefs::default();

    let reply = ask(
        &socket,
        &format!(r#"{{"_id":4,"caller":"{BROWSER}","type":"entries"}}"#),
    );
    settle(&mut bridge, &mut session, &prefs);
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
