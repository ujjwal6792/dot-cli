//! Everything the event loop reacts to arrives as one [`Event`] on one channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossterm::event::{self, Event as TermEvent};

use super::jobs::Done;

#[derive(Debug)]
pub enum Event {
    /// A key press, resize, ... from the terminal.
    Input(TermEvent),
    /// A background job finished.
    Job(Done),
    /// Every couple of seconds: re-read what the CLI may have changed.
    Tick,
}

const TICK: Duration = Duration::from_secs(2);

static PAUSED: AtomicBool = AtomicBool::new(false);

/// Stop (or resume) reading terminal input, so a child process such as the
/// editor owns the keyboard. Returns after the reader has had time to notice.
pub fn pause_input(paused: bool) {
    PAUSED.store(paused, Ordering::SeqCst);
    if paused {
        thread::sleep(Duration::from_millis(250));
    }
}

/// Read terminal input on its own thread so the event loop only ever waits on
/// the channel. Stops when `stop` is set (within ~100 ms).
pub fn spawn_input(tx: Sender<Event>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut last_tick = std::time::Instant::now();
        while !stop.load(Ordering::SeqCst) {
            if last_tick.elapsed() >= TICK {
                last_tick = std::time::Instant::now();
                if tx.send(Event::Tick).is_err() {
                    break;
                }
            }
            if PAUSED.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(ev) => {
                        if tx.send(Event::Input(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    })
}
