use super::*;
use crate::features::settings::model::GITHUB;
use crate::shared::ui::shot;
use std::fs;

#[test]
fn avatar_textures_live_with_the_sync_state() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let png = include_bytes!("../../../../assets/icon.png").to_vec();
    let base = ctx.tex_manager().read().num_allocated();
    let mut textures = std::collections::HashMap::new();

    {
        let first = avatar(&ctx, "octocat", Some(&png), &mut textures).expect("decodes");
        let again = avatar(&ctx, "octocat", Some(&png), &mut textures).expect("reuses");
        assert_eq!(first.id(), again.id(), "one texture per owner");
        assert_eq!(ctx.tex_manager().read().num_allocated(), base + 1);
    }

    assert_eq!(textures.len(), 1);
    textures.clear();
    assert_eq!(
        ctx.tex_manager().read().num_allocated(),
        base,
        "the refresh drops the old avatar texture"
    );
}

fn frame(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> Vec<egui::epaint::ClippedShape> {
    shot::frame(ctx, egui::vec2(900.0, 700.0), events, |ui| run(ui, app))
}

fn click(app: &mut App, ctx: &egui::Context, text: &str) {
    let shapes = frame(app, ctx, Vec::new());
    let at = shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(s.shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text:?} is on screen"));
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(app, ctx, vec![egui::Event::PointerMoved(at), button(true)]);
    frame(app, ctx, vec![button(false)]);
}

#[test]
fn temp_page_shots() {
    for (w, h) in [
        (393.0, 851.0),
        (393.0, 690.0),
        (360.0, 640.0),
        (780.0, 360.0),
        (900.0, 700.0),
    ] {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.token = zeroize::Zeroizing::new("t".to_owned());
        app.sync.repo_loaded = true;
        app.sync.repos = (0..3)
            .map(|i| crate::features::sync::github::Repo {
                full_name: format!("uriach/repo-{i}"),
                owner: "uriach".to_owned(),
                private: false,
            })
            .collect();
        // A working copy nobody else names: two tests that share one folder in the
        // shared data dir delete each other's files between a `create_dir_all` and
        // the write that follows it.
        let carbon = Source {
            kind: GITHUB.to_owned(),
            target: "uriach/carbon-shots".to_owned(),
        };
        app.prefs.sources = vec![carbon.clone()];
        let dir = store::source_dir(&carbon.kind, &carbon.target);
        fs::create_dir_all(&dir).unwrap();
        for file in ["bank.pandora", "personal.pandora", "work.pandora"] {
            fs::write(dir.join(file), b"locked").unwrap();
        }
        let name = format!("page-{}x{}", w as u32, h as u32);
        crate::shared::ui::shot::png(&ctx, &name, w as u32, h as u32, None, |ui| {
            run(ui, &mut app)
        });
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_working_copy_opens_without_a_session() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);

    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/pandora-ui-test".to_owned(),
    };
    app.prefs.sources = vec![source.clone()];
    let dir = store::source_dir(&source.kind, &source.target);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("personal.pandora");
    fs::write(&file, b"locked").unwrap();

    click(&mut app, &ctx, "personal");
    assert_eq!(app.screen, Screen::Unlock, "the copy opens off the disk");
    assert_eq!(app.auth.unlock_path, file.display().to_string());
    assert!(
        app.sync.repo_job.is_none(),
        "no request goes out without a token"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_repo_that_did_not_answer_says_why_instead_of_looking_empty() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);

    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/unanswered".to_owned(),
    };
    app.prefs.sources = vec![source.clone()];
    app.sync.failed.insert(
        crate::features::sync::logic::source_key(&source),
        crate::features::sync::github::EXPIRED.to_owned(),
    );

    let words = texts(&frame(&mut app, &ctx, Vec::new()));
    assert!(
        words.iter().any(|w| w.contains("no longer valid")),
        "the reason is on screen: {words:?}"
    );
    assert!(
        !words.contains(&"this repo has no boxes yet — create one below".to_owned()),
        "a read nobody answered is not an empty folder: {words:?}"
    );

    // Nothing answered and the disk holds nothing either: with no session
    // the place is the sign-in hint, and not an empty repo.
    app.sync.failed.clear();
    let words = texts(&frame(&mut app, &ctx, Vec::new()));
    assert!(
        words
            .iter()
            .any(|w| w.contains("sign in to see the whole repo")),
        "the sign-in hint gets its turn: {words:?}"
    );
}

#[test]
fn the_machine_view_lists_local_files_and_leaves_working_copies_alone() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Machine);

    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/machine-ui-test".to_owned(),
    };
    app.prefs.sources = vec![source.clone()];
    let inside = store::source_dir(&source.kind, &source.target).join("repo-box.pandora");
    let dir = std::env::temp_dir().join("pandora-machine-local");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let outside = dir.join("plain.pandora");
    fs::write(&outside, b"locked").unwrap();

    app.auth.recents = vec![
        crate::features::auth::model::Recent { path: inside },
        crate::features::auth::model::Recent {
            path: outside.clone(),
        },
    ];

    let words = texts(&frame(&mut app, &ctx, Vec::new()));
    // The row is the file's name: the box's own name never leaves the box.
    assert!(
        words.contains(&"plain".to_owned()),
        "the local file is listed: {words:?}"
    );
    assert!(
        !words.contains(&"repo-box".to_owned()),
        "a working copy is not a local file: {words:?}"
    );

    click(&mut app, &ctx, "plain");
    assert_eq!(app.screen, Screen::Unlock, "the row opens the box");
    assert_eq!(app.auth.unlock_path, outside.display().to_string());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_local_list_keeps_the_bar_gutter_beside_its_buttons() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Machine);
    app.auth.recents.push(crate::features::auth::model::Recent {
        path: PathBuf::from("/tmp/local-box.pandora"),
    });

    let shapes = frame(&mut app, &ctx, Vec::new());
    let row_name = rect_of(&shapes, "local-box");
    let row = surface_of(&shapes, row_name);
    let button = |text: &str| {
        let label = rect_of(&shapes, text);
        shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.fill == theme::palette().elev && rect.rect.contains(label.center()) =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap_or_else(|| panic!("{text:?} has a button surface"))
    };
    let buttons = button("Open a box file…").union(button("Create a new box"));

    assert!(
        (row.left() - buttons.left()).abs() < 1.0,
        "left edges differ: {row:?} {buttons:?}"
    );
    assert!(
        (buttons.right() - row.right() - theme::BAR).abs() < 1.0,
        "the row pays the bar's gutter on its right: {row:?} {buttons:?}"
    );
}

#[test]
fn the_code_and_its_copy_sit_on_one_line() {
    for (w, h) in [(393.0, 851.0), (900.0, 700.0)] {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = login_app(&ctx);
        let shapes = shot::frame(&ctx, egui::vec2(w, h), Vec::new(), |ui| run(ui, &mut app));

        let code = rect_of(&shapes, DEVICE);
        let copy = mark_beside(&shapes, code);
        let off = code.center().y - copy.center().y;
        assert!(
            off.abs() <= 1.0,
            "the code sits {off:.1} px off its copy at {w}x{h}: {code:?} against {copy:?}"
        );
    }
}

#[test]
fn the_sign_in_view_says_where_the_code_goes() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let size = egui::vec2(393.0, 851.0);

    let mut app = login_app(&ctx);
    let shapes = shot::frame(&ctx, size, Vec::new(), |ui| run(ui, &mut app));
    let words = texts(&shapes);
    for said in [
        "enter this code at github.com/login/device",
        DEVICE,
        "Open GitHub",
        "Cancel",
    ] {
        assert!(
            words.iter().any(|w| w == said),
            "{said:?} is on screen: {words:?}"
        );
    }

    // Before the code lands the same field holds the wait: a spinner, the
    // line saying what is being asked for, and nothing to act on yet.
    let mut app = login_app(&ctx);
    app.auth.login.as_mut().expect("a sign-in is on").code = None;
    let shapes = shot::frame(&ctx, size, Vec::new(), |ui| run(ui, &mut app));
    let words = texts(&shapes);
    assert!(
        words.iter().any(|w| w == "asking GitHub for a code…"),
        "the wait says what it waits for: {words:?}"
    );
    assert!(
        !words.iter().any(|w| w == "Open GitHub"),
        "there is nothing to open before there is a page: {words:?}"
    );
    assert!(!words.iter().any(|w| w == DEVICE), "no code yet: {words:?}");
    assert!(
            shapes.iter().any(|s| matches!(
                &s.shape,
                egui::Shape::Path(p)
                    if matches!(p.stroke.color, egui::epaint::ColorMode::Solid(c) if c == theme::palette().muted)
            )),
            "the field holds a spinner while it waits"
        );
}

fn rect_of(shapes: &[egui::epaint::ClippedShape], text: &str) -> egui::Rect {
    shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(s.shape.visual_bounding_rect()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text:?} is on screen"))
}

fn surface_of(shapes: &[egui::epaint::ClippedShape], name: egui::Rect) -> egui::Rect {
    shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill == theme::palette().panel
                    && (r.rect.height() - ROW).abs() < 1.0
                    && r.rect.contains(name.center()) =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the row paints a surface under {name:?}"))
}

const DEVICE: &str = "WDJB-MJHT";
const PAGE: &str = "https://github.com/login/device";

fn login_app(ctx: &egui::Context) -> App {
    let mut app = App::blank(ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);
    let (_worker, rx) = std::sync::mpsc::channel();
    app.auth.login = Some(crate::features::auth::model::Login {
        code: Some((DEVICE.to_owned(), PAGE.to_owned())),
        rx,
    });
    app
}

fn mark_beside(shapes: &[egui::epaint::ClippedShape], value: egui::Rect) -> egui::Rect {
    shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.brush.is_some()
                    && r.rect.left() > value.right()
                    && (r.rect.center().y - value.center().y).abs() < 60.0 =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the value carries a copy beside it: {value:?}"))
}

#[test]
fn a_repo_row_carries_its_mark_on_the_left() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);

    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/row-layout".to_owned(),
    };
    app.prefs.sources = vec![source.clone()];
    let dir = store::source_dir(&source.kind, &source.target);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("personal.pandora");
    fs::write(&file, b"locked").unwrap();

    let shapes = frame(&mut app, &ctx, Vec::new());
    let name = rect_of(&shapes, "personal");
    let surface = surface_of(&shapes, name);
    // The mark is an image: a rect filled with a texture brush.
    let mark = shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.brush.is_some() => (r.rect.width() <= 20.0
                && surface.contains(r.rect.center())
                && r.rect.right() < name.left())
            .then_some(r.rect),
            _ => None,
        })
        .expect("the row carries a mark, and it comes before the name");

    assert!(
        mark.left() - surface.left() < 14.0,
        "the mark opens the row: {mark:?} in {surface:?}"
    );
    assert!(
        name.left() - mark.right() < 16.0,
        "the mark sits against the name: {mark:?} vs {name:?}"
    );
    // Centred, a 400pt row would start the name some 170pt in.
    assert!(
        name.left() - surface.left() < 80.0,
        "the name follows the mark instead of floating mid-row: {name:?} in {surface:?}"
    );
    assert!(
        name.left() < surface.center().x,
        "the row reads from the left: {name:?} in {surface:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_first_read_says_it_is_reading_and_leaves_the_copies_in_place() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);
    app.sync.token = zeroize::Zeroizing::new("t".to_owned());

    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/reading".to_owned(),
    };
    app.prefs.sources = vec![source.clone()];
    let dir = store::source_dir(&source.kind, &source.target);
    fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("kept.pandora");
    fs::write(&copy, b"locked").unwrap();

    // The read has gone out and has not come back.
    let (_tx, rx) = std::sync::mpsc::channel();
    app.sync.repo_job = Some(crate::app::RepoJob {
        source: Some(source),
        announce: false,
        rx,
    });

    let words = texts(&frame(&mut app, &ctx, Vec::new()));
    assert!(
        words.contains(&"kept".to_owned()),
        "the copy on disk is on screen while the repo answers: {words:?}"
    );
    assert!(
        !words
            .iter()
            .any(|w| w.starts_with("this repo has no boxes")),
        "an unread repo is not called empty: {words:?}"
    );
    assert!(
        !words.contains(&"reading the repo…".to_owned()),
        "with copies on screen the wait covers nothing: {words:?}"
    );

    // With nothing on disk the wait is said, instead of a card that looks settled.
    fs::remove_file(&copy).unwrap();
    let words = texts(&frame(&mut app, &ctx, Vec::new()));
    assert!(
        words.contains(&"reading the repo…".to_owned()),
        "the wait says what it waits for: {words:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_new_box_form_floats_and_closes() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);

    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/pandora-ui-test".to_owned(),
    };
    app.prefs.sources = vec![source.clone()];
    app.sync.new_in = Some(source);

    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frame(&mut app, &ctx, Vec::new());
    }
    let words: Vec<String> = shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    assert!(
        words.iter().any(|w| w.eq_ignore_ascii_case("new box")),
        "the modal is up: {words:?}"
    );
    assert!(
        words.iter().any(|w| w.contains("octocat/pandora-ui-test")),
        "it says where the box lands"
    );
    assert!(words.iter().any(|w| w == "box name"), "{words:?}");

    // No Cancel button: the dialog's own X, the backdrop and Esc close it.
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(app.sync.new_in.is_none(), "Esc drops the form");
}

#[test]
fn the_home_prefix_reads_as_a_tilde() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    assert_eq!(short_path(&home.join("boxes/work.box")), "~/boxes/work.box");
    assert_eq!(short_path(Path::new("/etc/x")), "/etc/x");
}

#[test]
fn a_working_copy_sits_in_the_recents_beside_a_local_file() {
    let ctx = egui::Context::default();
    let mut app = App::blank(&ctx);
    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/boxes".to_owned(),
    };
    let inside = store::source_dir(&source.kind, &source.target).join("work.pandora");
    let outside = PathBuf::from("/tmp/plain.pandora");
    app.prefs.sources.push(source.clone());

    assert_eq!(app.source_of(&inside), Some(source));
    assert_eq!(app.source_of(&outside), None);

    app.auth.recents = vec![
        crate::features::auth::model::Recent { path: inside },
        crate::features::auth::model::Recent { path: outside },
    ];
    let listed: Vec<_> = app.auth.recents.iter().map(|r| r.label()).collect();
    assert_eq!(
        listed,
        ["work", "plain"],
        "the file's own name, not the box's"
    );
}

#[test]
fn a_copy_carries_the_github_mark_before_its_source_is_known() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/recents-ui-test".to_owned(),
    };
    let inside = store::source_dir(&source.kind, &source.target).join("work.pandora");
    let outside = PathBuf::from("/tmp/plain.pandora");
    app.auth.recents = vec![
        crate::features::auth::model::Recent {
            path: inside.clone(),
        },
        crate::features::auth::model::Recent { path: outside },
    ];

    // The folder says the copy is a repo's even with no source in the prefs.
    let listed = rows(&app);
    assert_eq!(listed[0].2, Some(theme::Brand::Github));
    assert_eq!(listed[1].2, None, "a local file carries no repo's mark");
    assert_eq!(
        listed[0].3,
        short_path(&inside),
        "an unknown repo is named by its folder"
    );

    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Machine);
    let words = texts(&frame(&mut app, &ctx, Vec::new()));
    assert!(
        !words.contains(&"work".to_owned()),
        "a copy belongs to the GitHub view: {words:?}"
    );

    app.prefs.sources.push(source);
    assert_eq!(
        rows(&app)[0].3,
        "octocat/recents-ui-test",
        "a known repo names its rows"
    );
}

fn picker_rect(ctx: &egui::Context) -> egui::Rect {
    ctx.memory(|mem| mem.area_rect(egui::Id::new("github-repo")))
        .expect("the picker is up")
}

#[test]
fn a_modal_keeps_off_the_screen_edges() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.sync.new_in = Some(Source {
        kind: GITHUB.to_owned(),
        target: "octocat/boxes".to_owned(),
    });

    // A screen narrower than the width the dialog asks for.
    let (w, h) = (360.0, 780.0);
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = shot::frame(&ctx, egui::vec2(w, h), Vec::new(), |ui| run(ui, &mut app));
    }

    // The heading is laid inside the frame's own margin, past the room the
    // card keeps: 16 off the screen, 20 of frame margin.
    let gap = crate::shared::ui::chrome::GAP;
    let heading = shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "NEW BOX" => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        })
        .expect("the dialog's heading is painted");
    assert!(
        heading.left() >= gap + 20.0 - 1.0,
        "the card keeps {gap} and its own margin off the screen: {heading:?}",
    );

    // The modal paints its backdrop over the whole window: it is what the
    // modal's area is seen through, so the room the card keeps must not be
    // taken off it.
    let backdrop = shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill.r() == 0
                    && r.fill.g() == 0
                    && r.fill.b() == 0
                    && r.fill.a() > 0
                    && r.rect.width() >= w =>
            {
                Some((r.rect, s.clip_rect))
            }
            _ => None,
        })
        .expect("the modal paints its backdrop");
    assert!(
        backdrop.0.left() == 0.0
            && backdrop.0.right() == w
            && backdrop.1.left() == 0.0
            && backdrop.1.right() == w,
        "the backdrop covers the window edge to edge, not {:?} seen through {:?}",
        backdrop.0,
        backdrop.1,
    );
}

#[test]
fn the_repo_picker_holds_its_size_while_loading() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);
    app.sync.token = zeroize::Zeroizing::new("t".to_owned());
    app.sync.adding = true;

    // The read has gone out and has not come back.
    let (_tx, rx) = std::sync::mpsc::channel();
    app.sync.repo_job = Some(crate::app::RepoJob {
        source: None,
        announce: true,
        rx,
    });
    // The dialog lays its own heading out lazily, so the first frames are not
    // yet its final shape; both states get the same number of them.
    for _ in 0..6 {
        frame(&mut app, &ctx, Vec::new());
    }
    let loading = picker_rect(&ctx).height();
    let searching = search_rect(&ctx).top();

    // The repos land: a second owner adds a chip, and neither the window nor
    // the field under the chips so much as twitches.
    app.sync.repo_job = None;
    app.sync.repo_loaded = true;
    app.sync.repos = vec![
        crate::features::sync::github::Repo {
            full_name: "octocat/one".to_owned(),
            owner: "octocat".to_owned(),
            private: false,
        },
        crate::features::sync::github::Repo {
            full_name: "sponsor/two".to_owned(),
            owner: "sponsor".to_owned(),
            private: false,
        },
    ];
    for _ in 0..6 {
        frame(&mut app, &ctx, Vec::new());
    }
    let landed = picker_rect(&ctx).height();
    let search = search_rect(&ctx).top();

    assert!(
        (landed - loading).abs() < 0.5,
        "the window is already its final size: {loading} while loading, {landed} once they land"
    );
    assert!(
        (search - searching).abs() < 0.5,
        "the select is above the search field and holds its room:\
             {searching} while loading, {search} once the repos land"
    );
}

#[test]
fn a_public_repo_is_listed_but_cannot_be_picked() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.sync.repos = vec![
        crate::features::sync::github::Repo {
            full_name: "octocat/closed".to_owned(),
            owner: "octocat".to_owned(),
            private: true,
        },
        crate::features::sync::github::Repo {
            full_name: "octocat/open".to_owned(),
            owner: "octocat".to_owned(),
            private: false,
        },
    ];

    let size = egui::vec2(360.0, 300.0);
    let mut chosen: Option<String> = None;
    let mut draw = |events: Vec<egui::Event>, chosen: &mut Option<String>| {
        shot::frame(&ctx, size, events, |ui| {
            listing(ui, &mut app, 200.0, chosen);
        })
    };

    let shapes = draw(Vec::new(), &mut chosen);
    let words = shot::words(&shapes);
    assert!(words.iter().any(|w| w == "private"), "{words:?}");
    assert!(
        words
            .iter()
            .any(|w| w == t("public — choose a private repo")),
        "a public repo says why it is out: {words:?}"
    );

    // Press each row in turn: only the private one answers.
    for (name, pickable) in [("octocat/open", false), ("octocat/closed", true)] {
        let at = rect_of(&shapes, name).center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        draw(
            vec![egui::Event::PointerMoved(at), button(true)],
            &mut chosen,
        );
        draw(vec![button(false)], &mut chosen);
        assert_eq!(
            chosen.is_some(),
            pickable,
            "{name} answered the press: {chosen:?}"
        );
        chosen = None;
    }
}

#[test]
fn the_repo_picker_fits_the_body_it_is_given() {
    for (w, h) in [(360.0, 640.0), (360.0, 420.0), (800.0, 360.0)] {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.token = zeroize::Zeroizing::new("t".to_owned());
        app.sync.adding = true;
        app.sync.repo_loaded = true;
        app.sync.repos = vec![
            crate::features::sync::github::Repo {
                full_name: "octocat/one".to_owned(),
                owner: "octocat".to_owned(),
                private: false,
            },
            crate::features::sync::github::Repo {
                full_name: "sponsor/two".to_owned(),
                owner: "sponsor".to_owned(),
                private: false,
            },
        ];
        let mut shapes = Vec::new();
        for _ in 0..8 {
            shapes = shot::frame(&ctx, egui::vec2(w, h), Vec::new(), |ui| run(ui, &mut app));
        }

        // The dialog's own frame: the `radius(12)` panel the card is drawn in.
        let mut frames: Vec<egui::Rect> = Vec::new();
        fn walk(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Rect(r) if r.corner_radius == theme::radius(12) => out.push(r.rect),
                egui::Shape::Vec(v) => {
                    for inner in v {
                        walk(inner, out);
                    }
                }
                _ => {}
            }
        }
        for s in &shapes {
            walk(&s.shape, &mut frames);
        }
        let frame = *frames.first().expect("the picker paints its frame");

        let body = crate::shared::ui::chrome::body(&ctx);
        assert!(
            frame.min.y >= body.min.y - 0.5 && frame.max.y <= body.max.y + 0.5,
            "the picker is off the body at {w}x{h}: frame {frame:?} vs body {body:?}"
        );
        assert!(
            frame.min.x >= body.min.x - 0.5 && frame.max.x <= body.max.x + 0.5,
            "the picker is off the body at {w}x{h}: frame {frame:?} vs body {body:?}"
        );
    }
}

#[test]
fn a_phones_repo_list_reaches_the_action_row() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);
    app.sync.token = zeroize::Zeroizing::new("t".to_owned());
    app.sync.adding = true;
    app.sync.repo_loaded = true;
    app.sync.repos = (0..12)
        .map(|i| crate::features::sync::github::Repo {
            full_name: format!("uriach/repo-{i}"),
            owner: "uriach".to_owned(),
            private: false,
        })
        .collect();

    let mut shapes = Vec::new();
    for _ in 0..4 {
        shapes = shot::frame(&ctx, egui::vec2(393.0, 851.0), Vec::new(), |ui| {
            run(ui, &mut app)
        });
    }

    // The first row's painted text rides the scroll area's own clip, which is
    // the viewport the list was given.
    let viewport = shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "uriach/repo-0" => Some(s.clip_rect),
            _ => None,
        })
        .expect("the first row is painted");
    let card = picker_rect(&ctx);
    let under = card.max.y - viewport.max.y;
    let target = crate::shared::ui::responsive::target(&ctx);
    // Under the list there are only the air above the action row, the row
    // itself and the frame's own bottom margin.
    let tail = 12.0 + ctx.style_of(ctx.theme()).spacing.item_spacing.y + target + 18.0;
    assert!(
        under <= tail + 2.0,
        "the list gives up {under} under itself, more than the {tail} of its tail:\
             rows stop at {viewport:?} in a card ending at {card:?}"
    );
    assert!(
        viewport.height() > card.height() / 2.0,
        "the scroll is the card, not a band in it: {viewport:?} in {card:?}"
    );
}

fn search_rect(ctx: &egui::Context) -> egui::Rect {
    ctx.read_response(egui::Id::new("repo-search"))
        .expect("the search field is up")
        .rect
}

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_org_select_filters_the_list() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);
    app.sync.adding = true;
    app.sync.repo_loaded = true;
    app.sync.repos = vec![
        crate::features::sync::github::Repo {
            full_name: "octocat/one".to_owned(),
            owner: "octocat".to_owned(),
            private: false,
        },
        crate::features::sync::github::Repo {
            full_name: "uriach/two".to_owned(),
            owner: "uriach".to_owned(),
            private: false,
        },
        crate::features::sync::github::Repo {
            full_name: "uriach/three".to_owned(),
            owner: "uriach".to_owned(),
            private: false,
        },
    ];

    // The dialog needs a frame to place itself before it paints.
    for _ in 0..2 {
        frame(&mut app, &ctx, Vec::new());
    }
    let shot = frame(&mut app, &ctx, Vec::new());
    assert!(
        rect_of(&shot, ALL).bottom() < search_rect(&ctx).top(),
        "the orgs pick the list, the search narrows what is left"
    );
    assert!(
        texts(&shot).contains(&ALL.to_owned()),
        "the select says what is picking the list"
    );

    click(&mut app, &ctx, ALL);
    let open = texts(&frame(&mut app, &ctx, Vec::new()));
    assert!(
        open.contains(&"uriach".to_owned()),
        "the orgs are one entry each, and one org is one entry"
    );

    click(&mut app, &ctx, "uriach");
    assert_eq!(
        app.sync.repo_search, "uriach/",
        "picking an org types it into the field"
    );

    let narrowed = texts(&frame(&mut app, &ctx, Vec::new()));
    assert!(narrowed.contains(&"uriach/two".to_owned()));
    assert!(
        !narrowed.contains(&"octocat/one".to_owned()),
        "the other org is filtered out"
    );
    assert!(
        narrowed.contains(&"uriach".to_owned()),
        "and the select reads back what it is holding"
    );

    // Back through the select: the button wears the org now, so `All repos`
    // is only in the list it opens.
    click(&mut app, &ctx, "uriach");
    click(&mut app, &ctx, ALL);
    assert_eq!(
        app.sync.repo_search, "",
        "`All repos` lets the rest back in"
    );
    let widened = texts(&frame(&mut app, &ctx, Vec::new()));
    assert!(widened.contains(&"octocat/one".to_owned()));
}

fn painted(shapes: &[egui::epaint::ClippedShape], fill: egui::Color32) -> egui::Rect {
    shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
            _ => None,
        })
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or_else(|| panic!("a rect painted in {fill:?}"))
}

#[test]
fn the_github_view_keeps_its_tail_inside_its_card() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);
    app.sync.token = zeroize::Zeroizing::new("t".to_owned());
    let carbon = Source {
        kind: GITHUB.to_owned(),
        target: "uriach/carbon".to_owned(),
    };
    app.prefs.sources = vec![
        carbon.clone(),
        Source {
            kind: GITHUB.to_owned(),
            target: "Jota-Music/jota".to_owned(),
        },
    ];
    let dir = store::source_dir(&carbon.kind, &carbon.target);
    fs::create_dir_all(&dir).unwrap();
    for file in ["bank.pandora", "personal.pandora", "work.pandora"] {
        fs::write(dir.join(file), b"locked").unwrap();
    }

    let mut shapes = Vec::new();
    for _ in 0..4 {
        shapes = frame(&mut app, &ctx, Vec::new());
    }
    let card = painted(&shapes, theme::palette().panel);
    let create = painted(&shapes, theme::ACCENT);
    assert!(
        create.height() < 48.0,
        "the way to create a box is the one accent block: {create:?}"
    );
    assert!(
        card.max.y <= 700.0 - BOTTOM,
        "the card holds the window it is drawn in, and leaves the floor alone: {card:?}"
    );
    assert!(
        card.contains_rect(create),
        "the way to create a box sits on the card, not under it: {card:?} {create:?}"
    );
    assert!(
        card.contains_rect(rect_of(&shapes, "Keep me signed in")),
        "and so does the toggle under it"
    );
    assert!(
        rect_of(&shapes, "uriach/carbon").bottom() < rect_of(&shapes, "bank").top(),
        "the select picks the list, above the boxes it holds"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_wrapped_empty_hint_keeps_the_repo_tail_on_the_card() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);
    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/a-rather-long-repo-name".to_owned(),
    };
    app.prefs.sources = vec![source.clone()];

    // A phone's width, and a repo read with nothing in it.
    let narrow = egui::vec2(380.0, 700.0);
    app.sync.token = zeroize::Zeroizing::new("t".to_owned());
    app.sync.listing.insert(
        crate::features::sync::logic::source_key(&source),
        Vec::new(),
    );
    let mut shapes = Vec::new();
    for _ in 0..4 {
        shapes = shot::frame(&ctx, narrow, Vec::new(), |ui| run(ui, &mut app));
    }
    let words = texts(&shapes);
    assert!(
        words
            .iter()
            .any(|w| w == "this repo has no boxes yet — create one below"),
        "the empty line is up: {words:?}"
    );
    let card = painted(&shapes, theme::palette().panel);
    assert!(
        card.max.y <= narrow.y - BOTTOM,
        "the card keeps the floor gap after the line wraps: {card:?}"
    );
    assert!(
        card.contains_rect(painted(&shapes, theme::ACCENT)),
        "the way to create a box stays on the card: {card:?}"
    );
    assert!(
        card.contains_rect(rect_of(&shapes, "Keep me signed in")),
        "and so does the toggle under it: {card:?}"
    );

    // Without a session the line is the longest one there is: it names the
    // copies on this machine and points at the sign-in above.
    app.sync.token = zeroize::Zeroizing::new(String::new());
    app.sync.listing.clear();
    let mut shapes = Vec::new();
    for _ in 0..4 {
        shapes = shot::frame(&ctx, narrow, Vec::new(), |ui| run(ui, &mut app));
    }
    let words = texts(&shapes);
    assert!(
        words
            .iter()
            .any(|w| w == "only the copies saved on this computer — sign in to see the whole repo"),
        "the longest line is up: {words:?}"
    );
    let card = painted(&shapes, theme::palette().panel);
    assert!(
        card.max.y <= narrow.y - BOTTOM,
        "the card keeps the floor gap with the longest line: {card:?}"
    );
    assert!(
        card.contains_rect(rect_of(&shapes, "Sign in with GitHub")),
        "and the way in stays on it: {card:?}"
    );

    // A window too short for the card's own floor gap — a small desktop, not
    // the phone this is written for. The band gives up its room, and the way
    // in and the toggle still ride the card instead of falling off it.
    let short = egui::vec2(380.0, 620.0);
    let mut shapes = Vec::new();
    for _ in 0..4 {
        shapes = shot::frame(&ctx, short, Vec::new(), |ui| run(ui, &mut app));
    }
    let card = painted(&shapes, theme::palette().panel);
    assert!(
        card.contains_rect(rect_of(&shapes, "Sign in with GitHub")),
        "the way in holds the card even when the floor gap does not: {card:?}"
    );
    assert!(
        card.contains_rect(rect_of(&shapes, "Keep me signed in")),
        "and the toggle under it does too: {card:?}"
    );
}

#[test]
fn a_long_repo_name_stays_inside_the_github_card() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.side = Some(Side::Github);
    let source = Source {
        kind: GITHUB.to_owned(),
        target: "octocat/a-very-long-repository-name-that-does-not-fit".to_owned(),
    };
    // Signed, so both marks are live, but already read, so no request goes
    // out over the network while the row is laid out.
    app.sync.token = zeroize::Zeroizing::new("t".to_owned());
    app.sync.read_at.insert(
        crate::features::sync::logic::source_key(&source),
        Instant::now(),
    );
    app.prefs.sources = vec![source];

    let mut shapes = Vec::new();
    for _ in 0..4 {
        shapes = shot::frame(&ctx, egui::vec2(393.0, 851.0), Vec::new(), |ui| {
            run(ui, &mut app)
        });
    }

    let card = painted(&shapes, theme::palette().panel);
    // The select and the two marks that act on the repo share one line, and
    // that line is the one this is about: the select's own label finds it.
    let row = shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with("octocat/") => {
                Some(s.shape.visual_bounding_rect().center().y)
            }
            _ => None,
        })
        .expect("the repo's name is in the select");
    let controls: Vec<egui::Rect> = shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill == theme::palette().elev
                    && (r.rect.center().y - row).abs() < responsive::target(&ctx) / 2.0 =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        controls.len(),
        3,
        "the select and its two marks are all on the line: {controls:?}"
    );
    for control in controls {
        assert!(
            card.contains_rect(control),
            "a control leaves the card: {control:?} in {card:?}"
        );
    }
}

#[test]
fn the_recents_card_keeps_the_floor_gap_of_the_local_view() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.screen = Screen::Sources;
    app.auth.recents = (0..12)
        .map(|i| crate::features::auth::model::Recent {
            path: PathBuf::from(format!("/tmp/floor{i}.pandora")),
        })
        .collect();

    let card = |app: &mut App, side: Side| {
        app.auth.side = Some(side);
        let mut shapes = Vec::new();
        for _ in 0..2 {
            shapes = frame(app, &ctx, Vec::new());
        }
        painted(&shapes, theme::palette().panel)
    };
    let recents = card(&mut app, Side::Recents);
    let local = card(&mut app, Side::Machine);

    assert!(
        (recents.max.y - local.max.y).abs() < 1.0,
        "recents and local leave a different floor gap: {recents:?} {local:?}"
    );
}

#[test]
fn the_folder_section_names_the_file_and_offers_both_moves() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.open("Folder section");
    let dir = tempfile::tempdir().expect("a temp dir");
    app.boxes.session.as_mut().expect("a box").remote =
        Some(crate::features::settings::model::Remote {
            kind: crate::features::settings::model::FOLDER.to_owned(),
            repo: String::new(),
            file: String::new(),
            sha: String::new(),
            folder: dir.path().display().to_string(),
            digest: "0123456789abcdef0123456789abcdef".to_owned(),
            stamp: 1_700_000_000_000,
            read_only: false,
            blocked: false,
        });

    let shapes = shot::frame(&ctx, egui::vec2(900.0, 700.0), Vec::new(), |ui| {
        folder(ui, &mut app)
    });
    let drawn = shot::words(&shapes);
    for want in [
        "FOLDER MIRROR",
        "vault.pandora",
        "Choose a folder…",
        "Sync now",
    ] {
        assert!(
            drawn.iter().any(|word| word == want),
            "{want:?} is missing: {drawn:?}"
        );
    }
}

#[test]
fn the_asking_field_keeps_its_right_padding_at_any_width() {
    for (w, h) in [(360.0, 640.0), (393.0, 851.0), (900.0, 700.0)] {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = login_app(&ctx);
        app.auth.login.as_mut().expect("a sign-in is on").code = None;
        let shapes = shot::frame(&ctx, egui::vec2(w, h), Vec::new(), |ui| run(ui, &mut app));

        let line = rect_of(&shapes, "asking GitHub for a code…");
        // The surface the wait sits on, not the card around it: the smallest one.
        let field = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.fill == theme::palette().elev
                        && (r.rect.center().y - line.center().y).abs() < 2.0
                        && r.rect.width() > line.width() =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap_or_else(|| panic!("the wait has a field around it at {w}x{h}"));

        assert!(
            field.right() - line.right() >= 40.0,
            "the dots keep clear of the field's edge at {w}x{h}: {line:?} in {field:?}"
        );
    }
}
