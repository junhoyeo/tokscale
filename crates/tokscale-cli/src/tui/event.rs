use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, KeyEvent, KeyEventKind, MouseEvent};

pub enum Event {
    Tick,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
}

pub struct EventHandler {
    rx: mpsc::Receiver<Event>,
}

fn should_forward_key_event(key: &KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

impl EventHandler {
    pub fn new_with_pending_keys(tick_rate: Duration, pending_keys: Vec<KeyEvent>) -> Self {
        let (tx, rx) = mpsc::channel();
        for key in pending_keys {
            // The receiver is alive for the whole constructor, so this cannot
            // fail. Queue before spawning to preserve startup input ordering.
            let _ = tx.send(Event::Key(key));
        }
        let event_tx = tx.clone();

        thread::spawn(move || loop {
            if event::poll(tick_rate).unwrap_or(false) {
                match event::read() {
                    Ok(event::Event::Key(key)) => {
                        if should_forward_key_event(&key) && event_tx.send(Event::Key(key)).is_err()
                        {
                            break;
                        }
                    }
                    Ok(event::Event::Mouse(mouse)) => {
                        if event_tx.send(Event::Mouse(mouse)).is_err() {
                            break;
                        }
                    }
                    Ok(event::Event::Resize(w, h)) => {
                        if event_tx.send(Event::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    _ => {
                        // Prevent event starvation from FocusGained/FocusLost bursts
                        if event_tx.send(Event::Tick).is_err() {
                            break;
                        }
                    }
                }
            } else if event_tx.send(Event::Tick).is_err() {
                break;
            }
        });

        drop(tx);
        Self { rx }
    }

    pub fn next(&mut self) -> Result<Event> {
        Ok(self.rx.recv()?)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{should_forward_key_event, Event, EventHandler};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(kind: KeyEventKind) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::NONE,
            kind,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn pending_probe_keys_are_delivered_before_live_input() {
        let expected = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let mut handler =
            EventHandler::new_with_pending_keys(Duration::from_millis(1), vec![expected]);
        match handler.next().unwrap() {
            Event::Key(actual) => assert_eq!(actual, expected),
            _ => panic!("pending key was not the first event"),
        }
    }

    #[test]
    fn forwards_press_and_repeat_but_not_release() {
        assert!(should_forward_key_event(&key(KeyEventKind::Press)));
        assert!(should_forward_key_event(&key(KeyEventKind::Repeat)));
        assert!(!should_forward_key_event(&key(KeyEventKind::Release)));
    }
}
