"""Tests for scripts/session-stats.py --cost.

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


if __name__ == "__main__":
    unittest.main()
