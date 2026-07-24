//! Where agent events go, and how more than one thing can watch them.
//!
//! The agent used to hold a `tauri::ipc::Channel<AgentEvent>` and write straight
//! into it. That coupled the harness to Tauri and, more importantly, allowed
//! exactly one watcher: whoever owned the channel. A second window — and later a
//! paired phone — needs the same stream, and neither may be able to stall the
//! agent loop by reading slowly.
//!
//! So the agent writes to an `EventSink` it knows nothing about, and the fan-out
//! is a `broadcast` channel: a subscriber that falls behind is dropped into a
//! `Gap` and told how much it missed, rather than being waited for. Recovering
//! from a gap is a re-read of the JSONL transcript, which is the source of truth
//! anyway — see `persist::replay`.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::agent::session::AgentEvent;

/// Somewhere agent events can be written. The agent depends on this and on
/// nothing else — not on Tauri, not on a bus, not on a remote peer.
pub trait EventSink: Send + Sync {
    fn send(&self, event: AgentEvent);
}

/// Shareable sink handle, cloned into subagents and spawned tasks.
pub type EventTx = Arc<dyn EventSink>;

/// Drops everything.
///
/// Only tests need it today, so it is gated rather than shipped as unused
/// surface. A headless run path — the daemon in phase 5 — would move it out.
#[cfg(test)]
pub struct NullSink;

#[cfg(test)]
impl EventSink for NullSink {
    fn send(&self, _event: AgentEvent) {}
}

/// What a subscriber got.
#[derive(Debug, Clone)]
pub enum Delivery {
    Event(Box<AgentEvent>),
    /// The subscriber read too slowly and the bus moved on without it. The
    /// count is what it missed — enough to know the stream is no longer
    /// contiguous, which is the point: loss is reported, never silent.
    Gap {
        missed: u64,
    },
    /// The bus is gone; no further events will arrive.
    Closed,
}

/// Fan-out of agent events to any number of watchers.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AgentEvent>,
}

impl EventBus {
    /// `capacity` is how far a subscriber may fall behind before it is dropped
    /// into a `Gap`. Bounded on purpose: an unbounded buffer turns a slow
    /// watcher into unbounded memory in the agent process.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Attach a watcher. It receives events published from now on; anything
    /// earlier is recovered by replaying the transcript, not from the bus.
    pub fn subscribe(&self) -> Subscriber {
        Subscriber {
            rx: self.tx.subscribe(),
        }
    }
}

impl EventSink for EventBus {
    fn send(&self, event: AgentEvent) {
        // An error here means nobody is listening, which is normal — the app
        // runs fine with every window closed. It must never surface as a
        // failure in the agent loop.
        let _ = self.tx.send(event);
    }
}

pub struct Subscriber {
    rx: broadcast::Receiver<AgentEvent>,
}

impl Subscriber {
    pub async fn recv(&mut self) -> Delivery {
        match self.rx.recv().await {
            Ok(event) => Delivery::Event(Box::new(event)),
            Err(broadcast::error::RecvError::Lagged(missed)) => Delivery::Gap { missed },
            Err(broadcast::error::RecvError::Closed) => Delivery::Closed,
        }
    }
}

/// Tags everything a subagent emits before handing it to the parent sink.
///
/// This replaces a wrapper that round-tripped every event through JSON — it
/// built a second Tauri channel, let the IPC layer serialize the event, parsed
/// it back out of the string, wrapped it and re-sent. The tagging was always
/// the only part that mattered.
pub struct SubagentSink {
    parent: EventTx,
    subagent_id: String,
}

impl SubagentSink {
    pub fn new(parent: EventTx, subagent_id: &str) -> Self {
        Self {
            parent,
            subagent_id: subagent_id.to_string(),
        }
    }
}

impl EventSink for SubagentSink {
    fn send(&self, event: AgentEvent) {
        self.parent.send(AgentEvent::Subagent {
            subagent_id: self.subagent_id.clone(),
            event: Box::new(event),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn text(s: &str) -> AgentEvent {
        AgentEvent::TextStep { text: s.into() }
    }

    fn text_of(delivery: &Delivery) -> String {
        match delivery {
            Delivery::Event(e) => match e.as_ref() {
                AgentEvent::TextStep { text } => text.clone(),
                other => panic!("expected TextStep, got {other:?}"),
            },
            other => panic!("expected an event, got {other:?}"),
        }
    }

    /// The reason the bus exists: two windows on the same session both see the
    /// live stream.
    #[tokio::test]
    async fn every_subscriber_gets_every_event() {
        let bus = EventBus::new(16);
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();

        bus.send(text("hello"));

        assert_eq!(text_of(&first.recv().await), "hello");
        assert_eq!(text_of(&second.recv().await), "hello");
    }

    /// A window opened mid-run catches up by replaying the JSONL, not from the
    /// bus — so the bus deliberately does not hand it the past.
    #[tokio::test]
    async fn a_late_subscriber_gets_only_what_follows_it() {
        let bus = EventBus::new(16);
        bus.send(text("before"));

        let mut late = bus.subscribe();
        bus.send(text("after"));

        assert_eq!(text_of(&late.recv().await), "after");
    }

    /// Publishing with nobody watching is the normal state of a closed window.
    /// It must not error and must not block.
    #[tokio::test]
    async fn publishing_into_an_empty_room_is_fine() {
        // No subscriber at all: must not panic, error or block.
        let bus = EventBus::new(4);
        bus.send(text("nobody home"));
    }

    /// The load-bearing property: a slow watcher is dropped into a gap, never
    /// waited for. If this were false, a stalled remote peer could freeze the
    /// agent loop on the developer's own machine.
    #[tokio::test]
    async fn a_slow_subscriber_is_told_it_fell_behind_instead_of_blocking() {
        let bus = EventBus::new(2);
        let mut slow = bus.subscribe();

        // Publish well past capacity without ever reading.
        for i in 0..10 {
            bus.send(text(&format!("event-{i}")));
        }

        match slow.recv().await {
            Delivery::Gap { missed } => assert_eq!(missed, 8),
            other => panic!("expected a gap, got {other:?}"),
        }

        // After the gap the stream resumes from what is still buffered, so the
        // subscriber keeps working rather than being disconnected.
        assert_eq!(text_of(&slow.recv().await), "event-8");
    }

    #[tokio::test]
    async fn a_subscriber_learns_when_the_bus_is_gone() {
        let bus = EventBus::new(4);
        let mut sub = bus.subscribe();
        drop(bus);

        assert!(matches!(sub.recv().await, Delivery::Closed));
    }

    #[tokio::test]
    async fn one_slow_subscriber_does_not_starve_a_fast_one() {
        let bus = EventBus::new(4);
        let mut fast = bus.subscribe();
        let _slow = bus.subscribe();

        for i in 0..4 {
            bus.send(text(&format!("event-{i}")));
        }

        for i in 0..4 {
            assert_eq!(text_of(&fast.recv().await), format!("event-{i}"));
        }
    }

    /// Records what a sink was handed, so wrapping can be asserted on.
    #[derive(Default)]
    struct RecordingSink {
        seen: Mutex<Vec<AgentEvent>>,
    }

    impl EventSink for RecordingSink {
        fn send(&self, event: AgentEvent) {
            self.seen.lock().unwrap().push(event);
        }
    }

    #[test]
    fn a_subagent_sink_tags_what_passes_through_it() {
        let recorder = Arc::new(RecordingSink::default());
        let sink = SubagentSink::new(recorder.clone(), "agent-7");

        sink.send(text("from the subagent"));

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        match &seen[0] {
            AgentEvent::Subagent { subagent_id, event } => {
                assert_eq!(subagent_id, "agent-7");
                assert!(
                    matches!(event.as_ref(), AgentEvent::TextStep { text } if text == "from the subagent")
                );
            }
            other => panic!("expected a tagged Subagent event, got {other:?}"),
        }
    }

    #[test]
    fn the_null_sink_swallows_everything() {
        NullSink.send(text("into the void"));
    }
}
