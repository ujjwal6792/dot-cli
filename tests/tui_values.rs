//! The Values screen (`V`), driven with real key events against the real
//! bundled chezmoi and git.

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

#[test]
fn the_values_screen_lists_variables_and_edits_them_through_pending_items() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    fs::write(t.home.join(".conf"), "font = Menlo\n").unwrap();
    t.dw_ok(&["track", ".conf", "--apply"]);
    t.dw_ok(&["values", "set-variable", "FONT", "Menlo", "--apply"]);
    t.dw_ok(&[
        "values",
        "set-variable",
        "FONT",
        "Mono",
        "--target",
        "platform:linux",
        "--apply",
    ]);
    t.dw_ok(&["values", "wire", ".conf", "FONT", "--apply"]);
    let mut tui = Tui::open(t);

    tui.press('V');
    let s = tui.screen();
    assert!(
        s.contains("FONT")
            && s.contains("variable")
            && s.contains("Menlo")
            && s.contains("platform:linux=Mono")
            && s.contains("~/.conf"),
        "{s}"
    );
    // Edit the default: the box is prefilled; a new text becomes a pending item.
    tui.press_key(KeyCode::Enter);
    for _ in 0..5 {
        tui.press_key(KeyCode::Backspace);
    }
    typed(&mut tui, "Monaco");
    tui.press_key(KeyCode::Enter);
    assert!(
        tui.app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Set variable FONT to Monaco"),
        "{:?}",
        tui.app.message
    );
    let list = tui.t.dw_ok(&["pending", "list"]);
    assert!(list.contains("Set variable FONT to Monaco"), "{list}");
    assert!(
        tui.t
            .dw_ok(&["values", "show", "FONT"])
            .contains("default: Menlo"),
        "not applied yet"
    );

    // An override: "TARGET VALUE".
    tui.press('o');
    typed(&mut tui, "machine:alpha Alpha");
    tui.press_key(KeyCode::Enter);
    assert!(
        tui.t
            .dw_ok(&["pending", "list"])
            .contains("for machine:alpha")
    );
    tui.press_key(KeyCode::Esc);
    assert!(
        !tui.screen().contains("Values ·"),
        "Esc returns to the main screen"
    );
}
