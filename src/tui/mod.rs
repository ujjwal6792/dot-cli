//! Terminal UI shell (spec §10): panels, Files browser, help overlay and a
//! non-blocking event loop.

pub mod app;
pub mod event;
pub mod files;
pub mod jobs;
pub mod ui;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use anyhow::{Result, anyhow};
use ratatui::Terminal;
use ratatui::backend::Backend;

use crate::hide::HideRules;
use crate::store::Store;
use app::App;
use event::Event;

/// The event loop. It only ever waits on the channel: terminal input and job
/// results are both events, so a slow tool can never delay a key press.
pub fn run_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rx: &Receiver<Event>,
) -> Result<()> {
    app.start();
    loop {
        terminal
            .draw(|f| ui::draw(f, app))
            .map_err(|e| anyhow!("{e}"))?;
        if app.quit {
            return Ok(());
        }
        let Ok(first) = rx.recv() else { return Ok(()) };
        app.handle_event(first);
        while !app.quit {
            let Ok(ev) = rx.try_recv() else { break };
            app.handle_event(ev);
        }
        if let Some(h) = app.take_handoff() {
            hand_off(terminal, app, h)?;
        }
    }
}

/// Give the terminal to the editor and take it back afterwards.
fn hand_off<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    mut h: app::Handoff,
) -> Result<()> {
    event::pause_input(true);
    ratatui::restore();
    let status = h.command.status();
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    terminal.clear().map_err(|e| anyhow!("{e}"))?;
    event::pause_input(false);
    app.finish_handoff(h.session, status.map_err(|e| e.to_string()));
    Ok(())
}

pub fn run(store: Store) -> Result<()> {
    let store = Arc::new(store);
    let rules =
        HideRules::load(&store.dirs.home, &store.repo())?.always_hide(store.dirs.own_dirs());
    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let input = event::spawn_input(tx.clone(), Arc::clone(&stop));
    let mut app = App::new(store, rules, tx);
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, &mut app, &rx);
    ratatui::restore();
    stop.store(true, Ordering::SeqCst);
    let _ = input.join();
    result
}
