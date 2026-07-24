//! Command deduplication: the guard that keeps a replayed frame from running
//! twice.
//!
//! A reconnect can replay a command. A replayed `SendMessage` appearing twice in
//! the transcript is annoying; a replayed `ResolveApproval` that runs the tool
//! twice is a duplicated `rm`. So every peer→device command carries a `cmd_id`
//! and passes through here first.
//!
//! # Why there are three answers and not two
//!
//! The obvious design is a set of seen ids: present means duplicate, absent
//! means fresh. That is not enough, and the gap is the interesting part.
//!
//! Recording the id *after* executing leaves a window: crash in between and the
//! id was never written, so a replay executes a second time. Recording it
//! *before* executing closes that window but opens another: crash in between and
//! the command is marked done although it never ran, so a legitimate retry is
//! silently swallowed.
//!
//! Neither is acceptable for a command that spawns a process, so the log records
//! both edges. A command that started and never finished comes back as
//! `Indeterminate`, and the caller refuses it and says so. That is deliberately
//! the unhelpful answer: re-running an `rm` is worse than making someone tap
//! approve again.

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How many command ids to remember. Bounded because the log is per channel and
/// lives for as long as a pairing does; the window only has to outlast a
/// reconnect, not a session.
pub const CAPACITY: usize = 512;

/// What happened to a command that ran to completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Acked,
    Failed { code: String, message: String },
}

/// What the caller should do with a command that just arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Never seen. Execute it — after calling `begin`.
    Fresh,
    /// Already finished. Do not execute; reply with this outcome again.
    Done(Outcome),
    /// Started and never finished: the device died mid-execution and there is no
    /// way to know whether the side effect happened. Refuse, and explain.
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Entry {
    Started,
    Finished(Outcome),
}

#[derive(Serialize, Deserialize)]
struct Record {
    cmd_id: String,
    #[serde(flatten)]
    entry: Entry,
}

/// Append-only log of command ids, with a bounded in-memory window.
///
/// Persisted next to the session, in the same JSONL style as the transcript, so
/// a replay that arrives after the app restarts is still caught.
pub struct CommandLog {
    path: PathBuf,
    /// Oldest first, so eviction is a pop from the front.
    window: VecDeque<(String, Entry)>,
}

impl CommandLog {
    /// Open the log at `path`, replaying what is already there.
    ///
    /// A missing file is an empty log, not an error: the first command on a new
    /// channel has nothing to be a duplicate of.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut window = VecDeque::new();

        if let Ok(file) = std::fs::File::open(&path) {
            for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                // A line this build cannot parse is skipped rather than fatal.
                // Losing one entry costs a duplicate check; refusing to open the
                // log at all would cost every one of them.
                if let Ok(record) = serde_json::from_str::<Record>(&line) {
                    Self::remember(&mut window, record.cmd_id, record.entry);
                }
            }
        }

        Self { path, window }
    }

    fn remember(window: &mut VecDeque<(String, Entry)>, cmd_id: String, entry: Entry) {
        if let Some(slot) = window.iter_mut().find(|(id, _)| *id == cmd_id) {
            slot.1 = entry;
            return;
        }
        window.push_back((cmd_id, entry));
        while window.len() > CAPACITY {
            window.pop_front();
        }
    }

    pub fn verdict(&self, cmd_id: &str) -> Verdict {
        match self.window.iter().find(|(id, _)| id == cmd_id) {
            None => Verdict::Fresh,
            Some((_, Entry::Started)) => Verdict::Indeterminate,
            Some((_, Entry::Finished(outcome))) => Verdict::Done(outcome.clone()),
        }
    }

    /// Mark a command as started. Call this *before* the side effect.
    pub fn begin(&mut self, cmd_id: &str) -> Result<(), String> {
        self.append(cmd_id, Entry::Started)
    }

    /// Mark a command as finished, with the answer to replay if it comes back.
    pub fn finish(&mut self, cmd_id: &str, outcome: Outcome) -> Result<(), String> {
        self.append(cmd_id, Entry::Finished(outcome))
    }

    fn append(&mut self, cmd_id: &str, entry: Entry) -> Result<(), String> {
        let record = Record {
            cmd_id: cmd_id.to_string(),
            entry: entry.clone(),
        };
        let line = serde_json::to_string(&record).map_err(|e| format!("serialize: {e}"))?;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create log dir: {e}"))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("open command log: {e}"))?;
        writeln!(file, "{line}").map_err(|e| format!("write command log: {e}"))?;

        Self::remember(&mut self.window, cmd_id.to_string(), entry);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> (tempfile::TempDir, CommandLog) {
        let dir = tempfile::tempdir().unwrap();
        let log = CommandLog::open(dir.path().join("commands.jsonl"));
        (dir, log)
    }

    #[test]
    fn an_unseen_command_is_fresh() {
        let (_d, log) = log();
        assert_eq!(log.verdict("cmd-1"), Verdict::Fresh);
    }

    #[test]
    fn a_finished_command_replays_its_outcome_instead_of_running_again() {
        let (_d, mut log) = log();

        log.begin("cmd-1").unwrap();
        log.finish("cmd-1", Outcome::Acked).unwrap();

        assert_eq!(log.verdict("cmd-1"), Verdict::Done(Outcome::Acked));
    }

    #[test]
    fn a_failure_is_replayed_too_so_a_retry_gets_the_same_answer() {
        let (_d, mut log) = log();
        let failure = Outcome::Failed {
            code: "policy_denied".into(),
            message: "bash approval is not granted remotely".into(),
        };

        log.begin("cmd-1").unwrap();
        log.finish("cmd-1", failure.clone()).unwrap();

        assert_eq!(log.verdict("cmd-1"), Verdict::Done(failure));
    }

    /// The window between `begin` and `finish` is where a crash hurts. Coming
    /// back as `Indeterminate` is what lets the caller fail closed rather than
    /// running a side effect a second time.
    #[test]
    fn a_command_that_started_and_never_finished_is_indeterminate() {
        let (_d, mut log) = log();

        log.begin("cmd-1").unwrap();

        assert_eq!(log.verdict("cmd-1"), Verdict::Indeterminate);
    }

    /// The whole point of persisting: a replay that arrives after the app
    /// restarts must still be caught. An in-memory set would miss exactly the
    /// case a flaky connection produces.
    #[test]
    fn the_log_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.jsonl");

        {
            let mut log = CommandLog::open(&path);
            log.begin("cmd-1").unwrap();
            log.finish("cmd-1", Outcome::Acked).unwrap();
            log.begin("cmd-2").unwrap();
        }

        let reopened = CommandLog::open(&path);

        assert_eq!(reopened.verdict("cmd-1"), Verdict::Done(Outcome::Acked));
        assert_eq!(
            reopened.verdict("cmd-2"),
            Verdict::Indeterminate,
            "a command in flight when the process died must not look fresh"
        );
        assert_eq!(reopened.verdict("cmd-3"), Verdict::Fresh);
    }

    /// A replayed approval frame must resolve to exactly one execution. This is
    /// the assertion the whole module exists for.
    #[test]
    fn a_frame_replayed_many_times_executes_once() {
        let (_d, mut log) = log();
        let mut executions = 0;

        for _ in 0..5 {
            match log.verdict("approve-rm") {
                Verdict::Fresh => {
                    log.begin("approve-rm").unwrap();
                    executions += 1;
                    log.finish("approve-rm", Outcome::Acked).unwrap();
                }
                Verdict::Done(_) => {}
                Verdict::Indeterminate => panic!("must not be indeterminate here"),
            }
        }

        assert_eq!(executions, 1);
    }

    #[test]
    fn commands_are_independent_of_each_other() {
        let (_d, mut log) = log();

        log.begin("cmd-1").unwrap();
        log.finish("cmd-1", Outcome::Acked).unwrap();

        assert_eq!(log.verdict("cmd-2"), Verdict::Fresh);
    }

    /// The window is bounded, so the oldest ids fall out. That is a deliberate
    /// trade: it only has to outlast a reconnect, and an unbounded log would
    /// grow for the life of a pairing.
    #[test]
    fn the_window_forgets_the_oldest_beyond_capacity() {
        let (_d, mut log) = log();

        for i in 0..CAPACITY + 10 {
            let id = format!("cmd-{i}");
            log.begin(&id).unwrap();
            log.finish(&id, Outcome::Acked).unwrap();
        }

        assert_eq!(log.verdict("cmd-0"), Verdict::Fresh, "evicted");
        assert_eq!(
            log.verdict(&format!("cmd-{}", CAPACITY + 9)),
            Verdict::Done(Outcome::Acked),
            "the newest is still remembered"
        );
        assert_eq!(log.window.len(), CAPACITY);
    }

    /// A truncated or garbled line costs one duplicate check. Refusing to open
    /// the log would cost all of them, which is the worse failure.
    #[test]
    fn a_corrupt_line_does_not_take_the_whole_log_down() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.jsonl");
        {
            let mut log = CommandLog::open(&path);
            log.begin("cmd-1").unwrap();
            log.finish("cmd-1", Outcome::Acked).unwrap();
        }
        // A half-written line, as a crash mid-append would leave.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{{\"cmd_id\":\"cmd-2\",\"sta").unwrap();
        drop(file);

        let reopened = CommandLog::open(&path);

        assert_eq!(reopened.verdict("cmd-1"), Verdict::Done(Outcome::Acked));
        assert_eq!(reopened.verdict("cmd-2"), Verdict::Fresh);
    }
}
