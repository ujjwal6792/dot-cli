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
}

/// Read terminal input on its own thread so the event loop only ever waits on
/// the channel. Stops when `stop` is set (within ~100 ms).
pub fn spawn_input(tx: Sender<Event>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
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
