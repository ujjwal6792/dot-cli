//! Secrets in the TUI: masked until unlocked, masked input, lock, encrypt (`E`).

mod common;

use std::fs;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use common::TestEnv;
use dotwarden::hide::HideRules;
use dotwarden::tui::app::App;
use dotwarden::tui::event::Event;
use dotwarden::tui::ui;

fn key(c: KeyCode) -> Event {
    Event::Input(TermEvent::Key(KeyEvent {
        code: c,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }))
}

struct Tui {
    t: TestEnv,
    app: App,
    rx: Receiver<Event>,
    _tx: Sender<Event>,
}

impl Tui {
    fn open(t: TestEnv) -> Self {
        let store = Arc::new(t.store());
        let rules = HideRules::for_store(&store).unwrap();
        let (tx, rx) = channel();
        let mut app = App::new(store, rules, tx.clone());
        app.start();
        let mut tui = Tui {
            t,
            app,
            rx,
            _tx: tx,
        };
        tui.settle();
        tui
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.rx.recv_timeout(Duration::from_millis(300)) {
                Ok(ev) => self.app.handle_event(ev),
                Err(_) => {
                    if self.app.in_flight_jobs() == 0 && !self.app.actions_busy() {
                        return;
                    }
                }
            }
            assert!(Instant::now() < deadline, "the TUI never settled");
        }
    }

    fn press(&mut self, c: char) {
        self.app.handle_event(key(KeyCode::Char(c)));
        self.settle();
    }

    fn press_key(&mut self, k: KeyCode) {
        self.app.handle_event(key(k));
        self.settle();
    }

    fn screen(&mut self) -> String {
        self.screen_at(100, 30)
    }

    fn screen_at(&mut self, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| ui::draw(f, &mut self.app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn typed(tui: &mut Tui, s: &str) {
    for c in s.chars() {
        tui.press(c);
    }
}

const PASS: &str = "correct horse battery";
const VALUE: &str = "TUI-Secret-4455aabb";

fn env_with_vault() -> TestEnv {
    let mut t = TestEnv::new();
    let rt = t.root.path().canonicalize().unwrap().join("rt");
    fs::create_dir_all(&rt).unwrap();
    t.set("XDG_RUNTIME_DIR", rt);
    t.set("DOTWARDEN_SESSION_BACKEND", "file");
    t.set("DOTWARDEN_TEST_WORK_FACTOR", "4");
    t.set("DOTWARDEN_TEST_NEW_PASSPHRASE", PASS);
    t.set("DOTWARDEN_TEST_SECRET_VALUE", VALUE);
    t.dw_ok(&["init", "--name", "alpha"]);
    t.dw_ok(&["vault", "init"]);
    t.dw_ok(&["values", "add-secret", "TOKEN"]);
    t.dw_ok(&["vault", "lock"]);
    t
}

#[test]
fn secret_values_show_only_while_unlocked_and_the_prompt_is_masked() {
    let mut tui = Tui::open(env_with_vault());
    tui.press('V');
    let s = tui.screen();
    assert!(
        s.contains("TOKEN") && s.contains("secret") && s.contains("••••••") && !s.contains(VALUE),
        "{s}"
    );
    tui.press('u');
    typed(&mut tui, PASS);
    let s = tui.screen();
    assert!(
        !s.contains(PASS) && s.contains("•••••"),
        "typed text is masked: {s}"
    );
    tui.press_key(KeyCode::Enter);
    let s = tui.screen();
    assert!(s.contains(VALUE), "unlocked: {:?} {s}", tui.app.message);
    tui.press('L');
    let s = tui.screen();
    assert!(!s.contains(VALUE), "locked again: {s}");
}

#[test]
fn e_stores_the_selected_tracked_file_encrypted_as_a_pending_item() {
    let t = env_with_vault();
    fs::write(t.home.join(".note"), "just text\n").unwrap();
    t.dw_ok(&["track", ".note", "--apply"]);
    let mut tui = Tui::open(t);
    tui.press('2');
    for _ in 0..60 {
        let s = tui.screen();
        if s.lines()
            .any(|l| l.contains(".note") && (l.contains('▶') || l.contains('>')))
        {
            break;
        }
        tui.press('j');
    }
    tui.press('E');
    let pending = tui.t.dw_ok(&["pending"]);
    assert!(pending.contains("encrypted"), "{pending}");
}
