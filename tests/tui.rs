//! TUI behaviour through ratatui's test backend with real key events, and the
//! non-blocking guarantee against a deliberately slow fake tool.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use common::TestEnv;
use dotwarden::hide::HideRules;
use dotwarden::paths::{Dirs, Env};
use dotwarden::store::Store;
use dotwarden::toolchain::Toolchain;
use dotwarden::tui::app::{App, Load, Panel};
use dotwarden::tui::event::Event;
use dotwarden::tui::{run_loop, ui};

fn key(c: KeyCode) -> Event {
    Event::Input(TermEvent::Key(KeyEvent {
        code: c,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }))
}

fn ch(c: char) -> Event {
    key(KeyCode::Char(c))
}

fn text(buf: &Buffer) -> String {
    let area = buf.area;
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A HOME with a few files, no Dotwarden repository, and tools that do not exist.
fn fixture() -> (tempfile::TempDir, Arc<Store>, HideRules) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    for d in [".config/nvim", ".cache/junk", "Projects", "Library/Caches"] {
        fs::create_dir_all(home.join(d)).unwrap();
    }
    fs::write(home.join(".zshrc"), "export EDITOR=nvim\n").unwrap();
    fs::write(home.join(".zsh_history"), "ls\n").unwrap();
    fs::write(home.join(".config/nvim/init.lua"), "-- init\n").unwrap();
    let mut env = Env::new();
    env.insert("HOME".into(), home.clone().into());
    let dirs = Dirs::from_env(&env).unwrap();
    let store = Arc::new(Store::new(dirs, Toolchain::at("/nonexistent"), env));
    let rules = HideRules::defaults(&home).unwrap();
    (dir, store, rules)
}

fn drive(app: &mut App, rx: &Receiver<Event>) {
    // Process events until the listing and the preview of the selection arrived.
    let deadline = Instant::now() + Duration::from_secs(5);
    while (app.files.loading || matches!(app.preview, Load::Loading)) && Instant::now() < deadline {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(50)) {
            app.handle_event(ev);
        }
    }
    assert!(!app.files.loading, "listing never arrived");
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    text(term.backend().buffer())
}

fn started(w: u16) -> (tempfile::TempDir, App, Receiver<Event>, Sender<Event>) {
    let _ = w;
    let (dir, store, rules) = fixture();
    let (tx, rx) = channel();
    let mut app = App::new(store, rules, tx.clone());
    app.start();
    drive(&mut app, &rx);
    (dir, app, rx, tx)
}

#[test]
fn eighty_by_twenty_four_shows_all_panels_the_main_view_and_pending() {
    let (_d, mut app, _rx, _tx) = started(80);
    let screen = render(&mut app, 80, 24);
    for needle in [
        "[1] Status",
        "[2] Files ~",
        "[3] Changes",
        "[4] Git",
        "[5] Machines",
        "Pending",
        "No pending actions",
        ".config/",
        "Projects/",
        ".zshrc",
        "q quit",
    ] {
        assert!(screen.contains(needle), "missing {needle:?} in:\n{screen}");
    }
    // Hide rules apply by default; dotfiles themselves are shown.
    for hidden in [".cache", ".zsh_history", "Caches"] {
        assert!(
            !screen.contains(hidden),
            "{hidden} should be hidden:\n{screen}"
        );
    }
    // The first entry is selected, so the main view previews it.
    assert!(screen.contains("nvim/"), "{screen}");
}

#[test]
fn dot_toggles_hidden_items_and_enter_and_leave_navigate() {
    let (_d, mut app, rx, _tx) = started(80);
    app.handle_event(ch('.'));
    let screen = render(&mut app, 80, 24);
    assert!(
        screen.contains(".cache/") && screen.contains(".zsh_history"),
        "{screen}"
    );
    assert!(screen.contains("(+hidden)"));
    app.handle_event(ch('.'));
    assert!(!render(&mut app, 80, 24).contains(".cache"));

    // Select .config (first row), open it, then go back.
    app.handle_event(ch('l'));
    drive(&mut app, &rx);
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("[2] Files ~/.config"), "{screen}");
    assert!(screen.contains("nvim/"));
    app.handle_event(key(KeyCode::Right));
    drive(&mut app, &rx);
    assert!(render(&mut app, 80, 24).contains("init.lua"));
    app.handle_event(key(KeyCode::Left));
    app.handle_event(ch('h'));
    drive(&mut app, &rx);
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("[2] Files ~ "), "{screen}");
    // At the root, h does nothing.
    app.handle_event(ch('h'));
    assert_eq!(app.files.cwd, app.files.root);
}

#[test]
fn arrow_and_vim_keys_move_the_selection() {
    let (_d, mut app, rx, _tx) = started(80);
    let first = app.files.selected_entry().unwrap().display();
    app.handle_event(ch('j'));
    app.handle_event(key(KeyCode::Down));
    assert_eq!(app.files.selected, 2);
    app.handle_event(ch('k'));
    app.handle_event(key(KeyCode::Up));
    assert_eq!(app.files.selected_entry().unwrap().display(), first);
    // lazygit's top/bottom keys; `G` is reserved for the lazygit handoff.
    app.handle_event(ch('>'));
    assert_eq!(app.files.selected, app.files.visible().len() - 1);
    app.handle_event(ch('<'));
    assert_eq!(app.files.selected, 0);
    app.handle_event(key(KeyCode::End));
    assert_eq!(app.files.selected, app.files.visible().len() - 1);
    app.handle_event(key(KeyCode::Home));
    assert_eq!(app.files.selected, 0);
    for reserved in ['G', 'g'] {
        app.handle_event(ch('j'));
        let at = app.files.selected;
        app.handle_event(ch(reserved));
        assert_eq!(
            app.files.selected, at,
            "{reserved} must not move the selection"
        );
    }
    drive(&mut app, &rx);
    app.handle_event(ch('?'));
    let screen = render(&mut app, 80, 24);
    assert!(
        screen.contains("first / last item") && screen.contains("< / >"),
        "{screen}"
    );
}

#[test]
fn library_is_hidden_except_explicitly_unhidden_paths() {
    let (d, mut app, rx, _tx) = started(80);
    let home = app.files.root.clone();
    for p in [
        "Library/Caches",
        "Library/Preferences",
        "Library/Application Support/Code/User",
        "Library/Application Support/Slack",
    ] {
        fs::create_dir_all(home.join(p)).unwrap();
    }
    let names = |app: &App| {
        app.files
            .visible()
            .iter()
            .map(|e| e.display())
            .collect::<Vec<_>>()
    };
    app.handle_event(ch('r'));
    drive(&mut app, &rx);
    // Library is reachable only because an app inside it is un-hidden.
    let idx = names(&app)
        .iter()
        .position(|n| n == "Library")
        .expect("Library shown");
    app.files.selected = idx;
    app.handle_event(ch('l'));
    drive(&mut app, &rx);
    assert_eq!(names(&app), ["Application Support"]);
    app.handle_event(ch('l'));
    drive(&mut app, &rx);
    assert_eq!(names(&app), ["Code"]);
    // `.` reveals the rest.
    app.handle_event(ch('.'));
    assert_eq!(names(&app), ["Code", "Slack"]);
    drop(d);
}

#[test]
fn dotwardens_own_directories_are_hidden_wherever_xdg_puts_them() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    fs::create_dir_all(home.join("xdg/data/dotwarden/repo")).unwrap();
    fs::create_dir_all(home.join("xdg/data/other")).unwrap();
    fs::create_dir_all(home.join("xdg-state/dotwarden")).unwrap();
    let mut env = Env::new();
    env.insert("HOME".into(), home.clone().into());
    env.insert("XDG_DATA_HOME".into(), home.join("xdg/data").into());
    env.insert("XDG_STATE_HOME".into(), home.join("xdg-state").into());
    let dirs = Dirs::from_env(&env).unwrap();
    let rules = HideRules::defaults(&home)
        .unwrap()
        .always_hide(dirs.own_dirs());
    let store = Arc::new(Store::new(dirs, Toolchain::at("/nonexistent"), env));
    let (tx, rx) = channel();
    let mut app = App::new(store, rules, tx);
    app.start();
    drive(&mut app, &rx);
    let names = |app: &App| {
        app.files
            .visible()
            .iter()
            .map(|e| e.display())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&app), ["xdg", "xdg-state"]);
    app.files.selected = 0;
    app.handle_event(ch('l'));
    drive(&mut app, &rx);
    app.handle_event(ch('l'));
    drive(&mut app, &rx);
    assert_eq!(names(&app), ["other"], "dotwarden's data dir is hidden");
    app.handle_event(ch('.'));
    assert!(names(&app).contains(&"dotwarden".to_string()));
}

#[test]
fn help_overlay_opens_and_closes() {
    let (_d, mut app, _rx, _tx) = started(80);
    app.handle_event(ch('?'));
    let screen = render(&mut app, 80, 24);
    assert!(
        screen.contains("Help (Esc to close)") && screen.contains("show or hide items"),
        "{screen}"
    );
    app.handle_event(key(KeyCode::Esc));
    assert!(!render(&mut app, 80, 24).contains("Help"));
}

#[test]
fn q_and_ctrl_c_quit() {
    let (_d, mut app, _rx, _tx) = started(80);
    app.handle_event(ch('q'));
    assert!(app.quit);
    let (_d2, mut app2, _rx2, _tx2) = started(80);
    app2.handle_event(Event::Input(TermEvent::Key(KeyEvent {
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })));
    assert!(app2.quit);
}

#[test]
fn narrow_terminals_show_only_the_focused_panel_at_45_by_14() {
    let (_d, mut app, _rx, _tx) = started(45);
    let screen = render(&mut app, 45, 14);
    assert!(screen.contains("[2] Files"), "{screen}");
    assert!(screen.contains(".zshrc") && screen.contains("Projects/"));
    for other in [
        "[1] Status",
        "[3] Changes",
        "[4] Git",
        "[5] Machines",
        "Pending",
    ] {
        assert!(
            !screen.contains(other),
            "{other} must not be shown:\n{screen}"
        );
    }
    assert_eq!(screen.lines().count(), 14);
    app.handle_event(ch('1'));
    let screen = render(&mut app, 45, 14);
    assert!(
        screen.contains("[1] Status") && !screen.contains("[2] Files"),
        "{screen}"
    );
    app.handle_event(ch('5'));
    assert!(render(&mut app, 45, 14).contains("[5] Machines"));
    assert_eq!(app.focus, Panel::Machines);
}

#[test]
fn resize_switches_between_layouts() {
    let (_d, mut app, _rx, _tx) = started(80);
    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(text(term.backend().buffer()).contains("[3] Changes"));

    term.backend_mut().resize(45, 14);
    app.handle_event(Event::Input(TermEvent::Resize(45, 14)));
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let narrow = text(term.backend().buffer());
    assert!(
        narrow.contains("[2] Files") && !narrow.contains("[3] Changes"),
        "{narrow}"
    );

    term.backend_mut().resize(120, 40);
    app.handle_event(Event::Input(TermEvent::Resize(120, 40)));
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let wide = text(term.backend().buffer());
    assert!(
        wide.contains("[5] Machines") && wide.contains("Pending"),
        "{wide}"
    );
}

#[test]
fn the_layout_is_usable_at_the_smallest_sizes_without_panicking() {
    let (_d, mut app, _rx, _tx) = started(80);
    for (w, h) in [
        (1, 1),
        (10, 3),
        (20, 5),
        (59, 40),
        (200, 15),
        (60, 16),
        (80, 24),
    ] {
        render(&mut app, w, h);
        app.handle_event(ch('?'));
        render(&mut app, w, h);
        app.handle_event(ch('?'));
    }
}

#[test]
fn other_panels_have_placeholders_and_h_l_switch_between_them() {
    let (_d, mut app, _rx, _tx) = started(80);
    app.handle_event(ch('3'));
    let screen = render(&mut app, 80, 24);
    assert!(
        screen.contains("[3] Changes") && screen.contains("Local ·"),
        "{screen}"
    );
    app.handle_event(ch('l'));
    assert_eq!(app.focus, Panel::Git);
}

// ---- non-blocking ------------------------------------------------------

fn fake_tool(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn keys_are_handled_while_a_slow_tool_is_running() {
    let t = TestEnv::new();
    let fake = t.root.path().join("slow-libexec");
    fake_tool(&fake.join("chezmoi"), "exit 0");
    fake_tool(&fake.join("git/bin/git"), "sleep 3; echo main");
    // An initialised repository, so the status job really invokes the slow git.
    let store = Store::new(t.dirs(), Toolchain::at(&fake), t.env.clone());
    fs::create_dir_all(store.repo()).unwrap();
    fs::write(store.repo().join(".chezmoiroot"), "home\n").unwrap();
    fs::write(store.repo().join("dotwarden.toml"), "format = 1\n").unwrap();
    for f in ["a", "b", "c"] {
        fs::write(t.home.join(f), f).unwrap();
    }

    let (tx, rx) = channel();
    let rules = HideRules::defaults(&t.home).unwrap();
    let mut app = App::new(Arc::new(store), rules, tx.clone());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let started = Instant::now();
    let handle = thread::spawn(move || {
        run_loop(&mut terminal, &mut app, &rx).unwrap();
        (app, terminal)
    });

    // Let the (fast) directory listing arrive; the status job is now stuck in git.
    thread::sleep(Duration::from_millis(400));
    for ev in [
        ch('?'),
        key(KeyCode::Esc),
        ch('j'),
        ch('j'),
        ch('.'),
        ch('2'),
        ch('q'),
    ] {
        tx.send(ev).unwrap();
    }
    let (app, terminal) = handle.join().unwrap();
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(2500),
        "event loop was blocked for {elapsed:?}"
    );
    assert!(app.quit);
    assert!(
        matches!(app.status, Load::Loading),
        "status job should still be running"
    );
    assert!(app.in_flight_jobs() >= 1);
    assert!(
        app.files.show_hidden,
        "key presses were applied while the tool ran"
    );
    assert!(!app.show_help);
    assert!(app.files.selected >= 1, "j moved the selection");
    let screen = text(terminal.backend().buffer());
    assert!(
        screen.contains("loading…"),
        "status panel shows progress:\n{screen}"
    );
}
