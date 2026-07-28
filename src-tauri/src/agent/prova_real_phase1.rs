//! The phase-1 prova real: two watchers on one session.
//!
//! # What this is for
//!
//! §8 phase 1 ends with: "two local windows attached to the same session both render the
//! live stream; approving in one immediately closes the gate in the other and the JSONL
//! records which one approved."
//!
//! Phase 1 shipped and this was never run. It is the oldest thing on the list and the
//! only one that claimed to have passed without anything checking — which is exactly the
//! shape of the gap that later let the device attach to the relay with no channel token.
//!
//! Unlike the phase-2 and phase-3 prova reals this needs no relay and no second process,
//! so it runs in CI rather than through a script. The point is not isolation: it is that
//! all of it happens on **one session, in the order a real interaction happens**, through
//! the real `BusRegistry` and the real `ApprovalRegistry`. Each of those has unit tests;
//! what none of them shows is the two working together.
//!
//! # The third claim, and why it is not here
//!
//! "the JSONL records which one approved" **cannot pass**: `SessionRecord` has no
//! approval variant and `persist.rs` never mentions an actor. So there is today no way to
//! answer "who approved that?" after the fact.
//!
//! That is not narrowed away to make this file green. It is a contradiction inside the
//! plan: §8 phase 4 schedules "Audit log: every remote command, with actor and decision,
//! written to JSONL and to a local audit file" — the same thing, one phase later. Writing
//! it means threading a store into `ApprovalRegistry::resolve`, which is the single funnel
//! every resolution passes through, local or remote. That is the right place for it and it
//! is a change to the agent's core, so it belongs with the phase that plans for it rather
//! than smuggled in under a test. Recorded in §0.2.

use std::time::Duration;

use crate::agent::approval::{Actor, ApprovalRegistry, ResolveError};
use crate::agent::eventbus::{BusRegistry, Delivery, EventBus, EventSink};
use crate::agent::session::AgentEvent;

const SESSION: &str = "prova-real-phase-1";

fn text(body: &str) -> AgentEvent {
    AgentEvent::TextStep {
        text: body.to_string(),
    }
}

/// Two windows attached to one session both see the whole stream.
///
/// The claim §8 phase 1 makes first. A registry keyed by session is what makes it
/// possible at all — before phase 1 there was a single Tauri `Channel`, so a second
/// window could not attach to a run that was already going.
#[tokio::test]
async fn two_windows_on_one_session_both_see_the_stream() {
    let buses = BusRegistry::default();
    let bus = buses.bus_for(SESSION).await;

    // Both attach before anything is published, which is the ordinary case: a second
    // window is opened while a run is going.
    let mut window_a = bus.subscribe();
    let mut window_b = bus.subscribe();

    bus.send(text("thinking"));
    bus.send(text("about it"));

    for window in [&mut window_a, &mut window_b] {
        assert!(matches!(
            window.recv().await,
            Delivery::Event(ref event) if matches!(**event, AgentEvent::TextStep { ref text } if text == "thinking")
        ));
        assert!(matches!(
            window.recv().await,
            Delivery::Event(ref event) if matches!(**event, AgentEvent::TextStep { ref text } if text == "about it")
        ));
    }
}

/// The same bus, found by session id from two places.
///
/// A second watcher has to be able to *find* a running session's bus, not make its own.
/// Two registries would each deliver half the stream to one window, which is the failure
/// this is here to rule out.
#[tokio::test]
async fn a_second_watcher_finds_the_bus_rather_than_making_one() {
    let buses = BusRegistry::default();

    let mut first = buses.bus_for(SESSION).await.subscribe();
    // A different caller, later, with only the session id to go on.
    let second_handle = buses.bus_for(SESSION).await;
    let mut second = second_handle.subscribe();

    second_handle.send(text("from the second handle"));

    for window in [&mut first, &mut second] {
        assert!(matches!(
            window.recv().await,
            Delivery::Event(ref event)
                if matches!(**event, AgentEvent::TextStep { ref text } if text == "from the second handle")
        ));
    }
}

/// Answering in one window closes the gate in the other, and says who won.
///
/// The second claim, and the one that matters: two answerers must not both execute a
/// tool. `resolve` is a compare-and-swap, so the loser is told the decision rather than
/// told the request does not exist — which is what lets the other window close its gate
/// showing what actually happened instead of an error.
#[tokio::test]
async fn approving_in_one_window_closes_the_gate_in_the_other() {
    let approvals = ApprovalRegistry::new();
    let key = format!("{SESSION}:toolu_1");

    // The agent raises the gate and waits.
    let gate = approvals.register(&key, None).await;
    let waiting = tokio::spawn(gate.wait());

    // Window A answers.
    let decision = approvals
        .resolve(&key, true, Actor::Local)
        .await
        .expect("the first answer is accepted");
    assert!(decision.approved);

    // Window B tries, a moment later, and loses.
    match approvals.resolve(&key, false, Actor::Local).await {
        Err(ResolveError::AlreadyResolved(theirs)) => {
            assert!(
                theirs.approved,
                "the loser must be told what actually happened"
            );
        }
        other => panic!("the second answer should have lost, got {other:?}"),
    }

    // And the agent proceeds on the first answer, with the actor attached.
    let settled = waiting.await.unwrap().expect("the gate resolved");
    assert!(settled.approved);
    assert_eq!(settled.actor, Actor::Local);
}

/// A remote peer answering is the same path, attributed differently.
///
/// Phase 1 built the attribution that phase 4 needs. Worth asserting here rather than
/// only in the remote tests: if a peer's answer were indistinguishable from the local
/// user's, the audit trail phase 4 plans would have nothing to record.
#[tokio::test]
async fn a_peer_answering_is_attributed_to_the_peer() {
    let approvals = ApprovalRegistry::new();
    let key = format!("{SESSION}:toolu_2");
    let gate = approvals.register(&key, None).await;

    approvals
        .resolve(&key, true, Actor::Peer("Safari on iPhone".into()))
        .await
        .unwrap();

    let settled = gate.wait().await.unwrap();
    assert_eq!(settled.actor, Actor::Peer("Safari on iPhone".into()));
}

/// A window that falls behind is told, and the one keeping up is not punished for it.
///
/// The phase-1 promise that matters most once a peer is involved: a phone on a bad
/// connection must not be able to stall the run, and must not be able to cost the window
/// at the machine its stream either. A bounded bus plus an explicit `Gap` is what makes
/// the slow watcher's problem the slow watcher's problem.
///
/// Written so the fast one really does keep up — reading between publishes. An earlier
/// version published everything first and then read, which left *both* subscribers behind
/// and would have passed on two gaps.
#[tokio::test]
async fn a_window_that_falls_behind_is_told_and_the_other_is_not_punished() {
    // Built directly rather than through the registry: its capacity is a constant sized
    // for real use, and falling behind a thousand events would say more about the test
    // than about the bus.
    let bus = EventBus::new(2);

    let mut keeping_up = bus.subscribe();
    let mut falling_behind = bus.subscribe();

    // The fast window reads as the events arrive; the slow one reads nothing.
    let mut fast_texts = Vec::new();
    for i in 0..6 {
        bus.send(text(&format!("event {i}")));
        match keeping_up.recv().await {
            Delivery::Event(event) => match *event {
                AgentEvent::TextStep { text } => fast_texts.push(text),
                other => panic!("unexpected event: {other:?}"),
            },
            other => panic!("the window that kept up was given {other:?}"),
        }
    }

    // Every event, in order, with no gap — the slow sibling cost it nothing.
    assert_eq!(
        fast_texts,
        (0..6).map(|i| format!("event {i}")).collect::<Vec<_>>()
    );

    // And the slow one is told how much it missed rather than silently skipping, which is
    // what makes a gap recoverable by re-reading the transcript.
    let mut missed_total = 0;
    let mut gapped = false;
    for _ in 0..8 {
        match falling_behind.recv().await {
            Delivery::Gap { missed } => {
                assert!(missed > 0, "a gap of nothing is not a gap");
                missed_total += missed;
                gapped = true;
                break;
            }
            Delivery::Event(_) => continue,
            Delivery::Closed => break,
        }
    }
    assert!(gapped, "a watcher that fell behind was never told");
    assert!(missed_total > 0);
}

/// Ending a session releases its gates rather than leaving them pending forever.
///
/// Two windows means two things that might still be waiting when a run ends. A gate that
/// outlived its session would hold a tool call open against a session that no longer
/// exists.
#[tokio::test]
async fn ending_a_session_releases_the_gates_it_left_open() {
    let approvals = ApprovalRegistry::new();
    let key = format!("{SESSION}:toolu_3");
    let gate = approvals.register(&key, None).await;

    approvals.abandon_session(SESSION).await;

    // Released rather than hanging — a gate that outlived its session would hold a tool
    // call open against a session that no longer exists.
    let outcome = tokio::time::timeout(Duration::from_secs(2), gate.wait())
        .await
        .expect("a gate outlived its session, so its tool call would hang forever");

    // And what it returns matters as much as that it returns: an abandoned gate must
    // never read as an approval. Asserting only that it came back would let a bug that
    // resolved it as approved pass.
    match outcome {
        Err(_) => {}
        Ok(decision) => assert!(
            !decision.approved,
            "an abandoned gate resolved as approved, so a tool would run with nobody having said yes"
        ),
    }
}
