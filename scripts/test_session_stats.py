"""Tests for scripts/session-stats.py --cost and --meters.

    python3 -m unittest scripts/test_session_stats.py

Stdlib only, like the script. The sessions directory is synthetic: one
session written by a build that records per-request `usage` (two runs, one of
which spawned a subagent whose transcript sits in `<id>.sub/`), and one from
an older build that never wrote `usage` lines.
"""

import importlib.util
import json
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SPEC = importlib.util.spec_from_file_location("session_stats", os.path.join(HERE, "session-stats.py"))
session_stats = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(session_stats)


def usage(round_, inp, read, created, out, cost=None, subagent=None):
    rec = {"kind": "usage", "round": round_, "model": "m", "input_tokens": inp,
           "cache_read_tokens": read, "cache_creation_tokens": created,
           "output_tokens": out, "ts": round_}
    if cost is not None:
        rec["cost"] = cost
    if subagent is not None:
        rec["subagent"] = subagent
    return rec


def write(path, records):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as handle:
        for rec in records:
            handle.write(json.dumps(rec) + "\n")


class CostReport(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        write(os.path.join(d, "s1.jsonl"), [
            {"kind": "meta", "session_id": "s1", "created_at": 1_000, "workspace": None},
            # Run 1: two requests, one spawn_agents call.
            {"kind": "user", "text": "fix it", "ts": 1},
            usage(1, 100, 900, 0, 10, cost=0.01),
            {"kind": "turn", "role": "assistant", "ts": 2, "content": [
                {"type": "tool_use", "id": "tu_1", "name": "spawn_agents", "input": {}}]},
            usage(2, 50, 1_000, 200, 20, cost=0.02),
            {"kind": "done", "input_tokens": 150, "output_tokens": 30, "ts": 3},
            # Run 2: one request whose provider reported no cost.
            {"kind": "user", "text": "and the docs", "ts": 4},
            usage(1, 40, 0, 0, 5),
            {"kind": "done", "input_tokens": 40, "output_tokens": 5, "ts": 5},
        ])
        write(os.path.join(d, "s1.sub", "tu_1-0.jsonl"), [
            {"kind": "meta", "session_id": "s1:sub:tu_1:0", "created_at": 1_001, "workspace": None},
            usage(1, 300, 0, 0, 50, cost=0.005, subagent="tu_1-0"),
            usage(2, 100, 300, 0, 50, cost=0.004, subagent="tu_1-0"),
        ])
        write(os.path.join(d, "old.jsonl"), [
            {"kind": "meta", "session_id": "old", "created_at": 900, "workspace": None},
            {"kind": "user", "text": "hi", "ts": 1},
            {"kind": "done", "input_tokens": 10, "output_tokens": 5, "ts": 2},
        ])

    def tearDown(self):
        self.tmp.cleanup()

    def test_cost_per_run_and_per_session_include_the_subagents(self):
        s = session_stats.cost_stats(self.tmp.name)
        self.assertEqual(s["sessions"], 1)
        self.assertEqual(s["sessions_without_usage"], 1)
        self.assertEqual(len(s["runs"]), 2)
        first, second = s["runs"]
        self.assertAlmostEqual(first["cost"], 0.01 + 0.02 + 0.005 + 0.004)
        self.assertEqual(first["rounds"], 2)
        self.assertEqual(first["subagent_rounds"], 2)
        self.assertEqual(first["unpriced"], 0)
        self.assertAlmostEqual(second["cost"], 0.0)
        self.assertEqual(second["unpriced"], 1, "a request with no reported cost is counted, not guessed")
        self.assertAlmostEqual(s["session_costs"][0], 0.039)

    def test_cache_and_subagent_shares(self):
        s = session_stats.cost_stats(self.tmp.name)
        # Input processed: uncached 590, cache reads 2200, cache writes 200.
        self.assertEqual((s["input"], s["cache_read"], s["cache_creation"]), (590, 2_200, 200))
        self.assertAlmostEqual(s["cache_read_share"], 2_200 / 2_990)
        self.assertAlmostEqual(s["cache_creation_share"], 200 / 2_990)
        # Subagent tokens: 350 + 400 + 100 (+0 written) over everything.
        everything = 590 + 2_200 + 200 + 135
        self.assertAlmostEqual(s["subagent_share"], 800 / everything)

    def test_the_report_names_the_numbers(self):
        text = session_stats.cost_report(session_stats.cost_stats(self.tmp.name))
        for want in ("COST", "per run", "per session", "rounds per run",
                     "cache-read share", "cache-creation share", "subagent share",
                     "1 request(s) reported no cost", "1 session(s) predate"):
            self.assertIn(want, text)

    def test_since_filters_on_the_session_creation_time(self):
        s = session_stats.cost_stats(self.tmp.name, since_ms=2_000)
        self.assertEqual(s["sessions"], 0)
        self.assertEqual(s["runs"], [])


class CacheBreaks(unittest.TestCase):
    """`cache_break` records — one per history rewrite — are counted per session."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        write(os.path.join(d, "a.jsonl"), [
            {"kind": "meta", "session_id": "a", "created_at": 1, "workspace": None},
            {"kind": "user", "text": "go", "ts": 1},
            usage(1, 10, 0, 0, 1, cost=0.01),
            {"kind": "pruned", "drop_calls": [], "truncate_results": [], "ts": 2},
            {"kind": "cache_break", "reason": "prune", "ts": 2},
            usage(2, 10, 0, 0, 1, cost=0.01),
            {"kind": "compacted", "summary": "...", "tail_turns": 2, "ts": 3},
            {"kind": "cache_break", "reason": "compaction", "ts": 3},
            usage(3, 10, 0, 0, 1, cost=0.01),
            {"kind": "cache_break", "reason": "handoff", "ts": 4},
            {"kind": "done", "input_tokens": 30, "output_tokens": 3, "ts": 5},
        ])
        write(os.path.join(d, "b.jsonl"), [
            {"kind": "meta", "session_id": "b", "created_at": 1, "workspace": None},
            {"kind": "user", "text": "go", "ts": 1},
            usage(1, 10, 0, 0, 1, cost=0.01),
            {"kind": "done", "input_tokens": 10, "output_tokens": 1, "ts": 2},
        ])

    def tearDown(self):
        self.tmp.cleanup()

    def test_cache_breaks_are_counted_per_session_and_by_reason(self):
        s = session_stats.cost_stats(self.tmp.name)
        self.assertEqual(sorted(s["cache_breaks"]), [0, 3], "one count per session, zero included")
        self.assertEqual(s["cache_break_reasons"], {"prune": 1, "compaction": 1, "handoff": 1})

    def test_the_report_names_the_cache_breaks(self):
        text = session_stats.cost_report(session_stats.cost_stats(self.tmp.name))
        self.assertIn("cache breaks per session median", text)
        self.assertIn("compaction 1", text)
        self.assertIn("handoff 1", text)
        self.assertIn("prune 1", text)

    def test_a_session_from_before_cache_breaks_were_recorded_counts_zero(self):
        s = session_stats.cost_stats(self.tmp.name)
        self.assertIn(0, s["cache_breaks"])


class SubagentTranscriptsFindTheirRun(unittest.TestCase):
    """Each transcript in `<id>.sub/` is booked to the run that caused it."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_tool_use_id_the_file_name_had_to_sanitize_still_finds_its_spawn(self):
        d = self.tmp.name
        write(os.path.join(d, "s.jsonl"), [
            {"kind": "meta", "session_id": "s", "created_at": 1, "workspace": None},
            {"kind": "user", "text": "go", "ts": 1},
            usage(1, 10, 0, 0, 1, cost=0.01),
            {"kind": "turn", "role": "assistant", "ts": 2, "content": [
                {"type": "tool_use", "id": "call:7.a", "name": "spawn_agents", "input": {}}]},
            {"kind": "done", "input_tokens": 10, "output_tokens": 1, "ts": 3},
        ])
        # persist::subagent_transcript_path writes "call:7.a" as "call_7_a".
        write(os.path.join(d, "s.sub", "call_7_a-0.jsonl"), [
            {"kind": "meta", "session_id": "s:sub", "created_at": 2, "workspace": None},
            usage(1, 5, 0, 0, 1, cost=0.002, subagent="call_7_a-0"),
        ])
        s = session_stats.cost_stats(d)
        self.assertEqual(len(s["runs"]), 1, "the subagent belongs to the run that spawned it")
        self.assertEqual(s["runs"][0]["subagent_rounds"], 1)
        self.assertAlmostEqual(s["runs"][0]["cost"], 0.012)

    def test_a_compaction_summary_is_booked_to_the_run_it_compacted(self):
        d = self.tmp.name
        write(os.path.join(d, "s.jsonl"), [
            {"kind": "meta", "session_id": "s", "created_at": 1, "workspace": None},
            {"kind": "user", "text": "first", "ts": 10},
            usage(11, 10, 0, 0, 1, cost=0.01),
            {"kind": "compacted", "summary": "...", "tail_turns": 2, "ts": 15},
            usage(16, 10, 0, 0, 1, cost=0.01),
            {"kind": "done", "input_tokens": 20, "output_tokens": 2, "ts": 20},
            {"kind": "user", "text": "second", "ts": 30},
            usage(31, 10, 0, 0, 1, cost=0.01),
            {"kind": "done", "input_tokens": 10, "output_tokens": 1, "ts": 32},
        ])
        write(os.path.join(d, "s.sub", "compact-0.jsonl"), [
            {"kind": "meta", "session_id": "s-summarizer", "created_at": 12, "workspace": None},
            usage(1, 100, 0, 0, 30, cost=0.004, subagent="compact-0"),
            usage(2, 120, 0, 0, 40, cost=0.005, subagent="compact-0"),
        ])
        s = session_stats.cost_stats(d)
        self.assertEqual(len(s["runs"]), 2, "a compaction is not a run of its own")
        first, second = s["runs"]
        self.assertEqual((first["rounds"], first["subagent_rounds"]), (2, 2))
        self.assertAlmostEqual(first["cost"], 0.029)
        self.assertEqual(second["subagent_rounds"], 0)


def call(id_, name, result):
    """A tool call and its result, as the loop persists them."""
    return [
        {"kind": "turn", "role": "assistant", "ts": 1, "content": [
            {"type": "tool_use", "id": id_, "name": name, "input": {}}]},
        {"kind": "turn", "role": "user", "ts": 1, "content": [
            {"type": "tool_result", "tool_use_id": id_, "content": result}]},
    ]


class ToolErrors(unittest.TestCase):
    """M4: tool results that are errors over all tool calls, main and subagents
    apart, booked to the tool. Same rule as `eval/metrics.rs`: a result that
    starts with `Error`, or a `spawn_agents` call that started none of its
    agents (`not started:`, which never says `Error`)."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        write(os.path.join(d, "s1.jsonl"), [
            {"kind": "meta", "session_id": "s1", "created_at": 1, "workspace": None},
            *call("t1", "read_file", "Error: end_line must be >= start_line"),
            *call("t2", "grep", "src/a.py:3: def load()"),
            *call("t3", "spawn_agents", "## config — failed (0 rounds)\nnot started: the document "
                                        "has no section ''. Two agents cannot hold one section.\n---"),
            # The image-carrying shape of a result: a list of blocks.
            *call("t4", "spawn_agents", [{"type": "text", "text": "## retry — done (5 rounds)\nok\n---"}]),
        ])
        write(os.path.join(d, "s1.sub", "t4-0.jsonl"), [
            {"kind": "meta", "session_id": "s1:sub", "created_at": 2, "workspace": None},
            *call("u1", "doc_read", "Error: you were given section s2 only; s3 belongs to another agent"),
            *call("u2", "read_file", "def retry(): ..."),
            *call("u3", "doc_write", "Wrote s2."),
        ])

    def tearDown(self):
        self.tmp.cleanup()

    def test_errors_are_counted_apart_for_main_and_subagents_and_booked_to_the_tool(self):
        s = session_stats.tool_error_stats(self.tmp.name)
        self.assertEqual((s["main_calls"], s["main_errors"]), (4, 2))
        self.assertEqual((s["subagent_calls"], s["subagent_errors"]), (3, 1))
        self.assertEqual(dict(s["by_tool"]), {"read_file": 1, "spawn_agents": 1, "doc_read": 1})

    def test_the_report_names_m4_and_the_tools(self):
        text = session_stats.meters_report(session_stats.tool_error_stats(self.tmp.name),
                                           session_stats.document_stats(self.tmp.name))
        self.assertIn("M4 tool-call error rate: main 50.0% (2/4) | subagents 33.3% (1/3)", text)
        self.assertIn("spawn_agents 1", text)


class DocumentSections(unittest.TestCase):
    """M6: the sections of each living document a session linked, by owner,
    as the file stands now — read from the workspace the session names."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        ws = self.tmp.name
        self.sessions = os.path.join(ws, ".claudinio", "sessions")
        os.makedirs(os.path.join(ws, ".claudinio", "docs"))
        with open(os.path.join(ws, ".claudinio", "docs", "notes.md"), "w") as handle:
            handle.write(
                "# Notes\n\n"
                "## Config\n<!-- claudinio: id=s1 status=done by=sub:config -->\nenv wins\n"
                "<!-- claudinio: end -->\n\n"
                "## Retry\n<!-- claudinio: id=s2 status=done by=sub:retry -->\n429 and 503\n"
                "<!-- claudinio: end -->\n\n"
                "## CLI\n<!-- claudinio: id=s3 status=done by=main -->\n--dry-run\n"
                "<!-- claudinio: end -->\n\n"
                "[Open questions] <!-- claudinio: id=s4 status=todo -->\n")
        linked = {"kind": "document_linked", "path": ".claudinio/docs/notes.md", "ts": 2}
        write(os.path.join(self.sessions, "a.jsonl"), [
            {"kind": "meta", "session_id": "a", "created_at": 1, "workspace": ws}, linked])
        # A successor carries the same document: counted once.
        write(os.path.join(self.sessions, "b.jsonl"), [
            {"kind": "meta", "session_id": "b", "created_at": 3, "workspace": ws}, linked])
        write(os.path.join(self.sessions, "c.jsonl"), [
            {"kind": "meta", "session_id": "c", "created_at": 4, "workspace": ws}])

    def tearDown(self):
        self.tmp.cleanup()

    def test_sections_are_counted_by_owner_once_per_document(self):
        s = session_stats.document_stats(self.sessions)
        self.assertEqual(s["sessions"], 3)
        self.assertEqual(s["sessions_with_document"], 2)
        self.assertEqual(len(s["documents"]), 1)
        self.assertEqual((s["sections"], s["written"], s["by_sub"], s["by_main"]), (4, 3, 2, 1))

    def test_the_report_names_m6(self):
        text = session_stats.meters_report(session_stats.tool_error_stats(self.sessions),
                                           session_stats.document_stats(self.sessions))
        self.assertIn("M6 sections written by subagents: 66.7% (2 of 3 written; main 1)", text)


if __name__ == "__main__":
    unittest.main()
