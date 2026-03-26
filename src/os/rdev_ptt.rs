use std::sync::mpsc::Sender;

use anyhow::Result;
use rdev::{Event, EventType, Key, listen};

pub fn listen_for_ptt_events(tx: Sender<bool>, ptt_key: Key) -> Result<()> {
    listen(move |event: Event| {
        let pressed = match event.event_type {
            EventType::KeyPress(key) if key == ptt_key => Some(true),
            EventType::KeyRelease(key) if key == ptt_key => Some(false),
            _ => None,
        };
        if let Some(pressed) = pressed {
            let _ = tx.send(pressed);
        }
    })
    .map_err(|error| anyhow::anyhow!("global key listener failed: {error:?}"))
}
