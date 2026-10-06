#!/usr/bin/env python3
"""Where a workspace's sessions spend their context and their output.

Reads every session JSONL in a sessions directory and prints the numbers the
harness plans are gated on (see docs/plans/2026-10-06_harness-paper-learnings.md):
which tool results are re-read the most, what the model writes the most of, how
often context pressure triggers, which tools are actually called, and who
decides to plan.

    python3 scripts/session-stats.py                       # ./.claudinio/sessions
    python3 scripts/session-stats.py path/to/sessions
    python3 scripts/session-stats.py --since 2026-10-01    # sessions created on or after

Stdlib only. Read-only. It prints aggregates, never message contents.

"Carried mass" is characters multiplied by the number of later requests in the
session that re-read them: a 10k-char result read at round 3 of a 40-round
session weighs far more than the same result at round 39. It approximates where
input tokens go, cached or not. Compactions reset it; prunes are not modelled.

Limits worth knowing before quoting a number:
- Thinking is not persisted, and before `subagent_run` records existed nothing
  about subagents was either, so "generated" covers the main session's visible
  output only.
- `done.input_tokens` is what the provider billed as uncached input. Cached
  input is only known for runs that carry `cache_read_tokens`.
"""

import argparse
import collections
import datetime
import glob
import json
import os
import re
import sys

Counter = collections.Counter
SHELL_READERS = {"grep", "rg", "cat", "ls", "find", "sed", "head", "tail", "wc"}


def block_chars(block):
    kind = block.get("type")
    if kind == "text":
        return len(block.get("text") or "")
    if kind == "tool_use":
        return len(json.dumps(block.get("input"), ensure_ascii=False))
    if kind == "tool_result":
        content = block.get("content")
        if isinstance(content, str):
            return len(content)
        return sum(
            len(part.get("text") or "")
            for part in (content or [])
            if isinstance(part, dict) and part.get("type") == "text"
        )
    return 0


def result_text(block):
    """The text of a tool_result block, whichever of its two shapes it has."""
    content = block.get("content")
    if isinstance(content, str):
        return content
    return "".join(
        part.get("text") or ""
        for part in (content or [])
        if isinstance(part, dict) and part.get("type") == "text"
    )


def quantile(values, q):
    values = sorted(values)
    return values[min(len(values) - 1, int(q * len(values)))] if values else 0


def pct(part, whole):
    return f"{100.0 * part / whole:.1f}%" if whole else "n/a"


def tool_label(name):
    return "mcp__*" if (name or "").startswith("mcp__") else (name or "?")


def is_brain(mode):
    return mode in ("brain", "pensador")


class Stats:
    def __init__(self):
        self.sessions = 0
        self.created = []
        self.kinds = Counter()
        self.runs = 0
        self.tokens_in = self.tokens_out = 0
        self.cache_runs = self.cache_read = self.cache_miss = 0
        self.peaks = []
        self.handoffs = Counter()
        self.calls = Counter()
        self.arg_chars = Counter()
        self.result_chars = Counter()
        self.carried = Counter()
        self.generated = Counter()
        self.result_sizes = []
        self.tasks_set_sizes = []
        self.tasks_per_session = []
        self.bash_first = Counter()
        self.reads = self.exact_rereads = self.pointers = 0
        self.read_chars = self.superseded_chars = 0
        self.fresh = Counter()
        self.later_brain = 0
        self.first_prompt = {"brain": [], "builder": []}
        self.rounds = {"brain": [], "builder": []}
        self.rounds_per_run = []
        self.tails = []
        self.elision = []
        self.prefix = collections.defaultdict(list)
        self.windows = Counter()
        self.profiles = Counter()
        self.sub_runs = 0
        self.sub_tools = Counter()
        self.sub_modes = Counter()
        self.routes = Counter()

    def read_session(self, path, since_ms):
        records = []
        with open(path, errors="replace") as handle:
            for line in handle:
                try:
                    records.append(json.loads(line))
                except ValueError:
                    self.kinds["(unparseable)"] += 1
        created = next((r.get("created_at") for r in records if r.get("kind") == "meta"), None)
        if since_ms and (created or 0) < since_ms:
            return
        self.sessions += 1
        if created:
            self.created.append(created)

        tool_of = {}        # tool_use id -> (name, read key)
        live = Counter()    # chars currently in history, by category
        seen_reads = {}     # (path, start, end) -> chars of the last result
        messages = []       # per message: [(category, chars)]
        peak = 0
        tasks_set = 0
        run_rounds = 0
        last_edit = None
        users = [r for r in records if r.get("kind") == "user"]
        linked = any(r.get("kind") == "linked_from" for r in records)

        for rec in records:
            kind = rec.get("kind")
            self.kinds[kind] += 1
            if kind == "status":
                peak = max(peak, rec.get("context_tokens") or 0)
            elif kind == "done":
                self.runs += 1
                self.tokens_in += rec.get("input_tokens", 0)
                self.tokens_out += rec.get("output_tokens", 0)
                if rec.get("cache_read_tokens") is not None:
                    self.cache_runs += 1
                    self.cache_read += rec["cache_read_tokens"]
                    self.cache_miss += rec.get("input_tokens", 0)
                if run_rounds:
                    self.rounds_per_run.append(run_rounds)
                    if last_edit is not None:
                        self.tails.append((run_rounds - last_edit, run_rounds))
                run_rounds, last_edit = 0, None
            elif kind == "handoff_to":
                self.handoffs[rec.get("reason")] += 1
            elif kind == "compacted":
                live = Counter({"summary": len(rec.get("summary") or "")})
            elif kind == "run_config":
                prefix = rec.get("prefix") or {}
                for part, tokens in prefix.items():
                    self.prefix[part].append(tokens)
                self.prefix["total"].append(sum(prefix.values()))
                self.windows[rec.get("context_window")] += 1
                self.profiles[(rec.get("profile") or "?", rec.get("surface") or "?")] += 1
            elif kind == "subagent_run":
                self.sub_runs += 1
                self.sub_modes[rec.get("mode")] += 1
                for name, count in (rec.get("tools") or {}).items():
                    self.sub_tools[tool_label(name)] += count
            elif kind == "mode_route":
                self.routes[(rec.get("verdict"), rec.get("ran_as"), rec.get("source"),
                             bool(rec.get("shadow")))] += 1
            elif kind == "turn":
                role = rec.get("role")
                if role == "assistant":
                    run_rounds += 1
                    # Everything already in history is re-read by this request.
                    self.carried.update(live)
                parts = []
                for block in rec.get("content") or []:
                    btype, chars = block.get("type"), block_chars(block)
                    if btype == "tool_use":
                        name = tool_label(block.get("name"))
                        args = block.get("input") or {}
                        read_key = None
                        if name == "read_file":
                            read_key = (args.get("path") or args.get("file_path"),
                                        args.get("start_line"), args.get("end_line"))
                        tool_of[block.get("id")] = (name, read_key)
                        self.calls[name] += 1
                        self.arg_chars[name] += chars
                        self.generated["args:" + name] += chars
                        live["args:" + name] += chars
                        parts.append(("args", chars))
                        if name == "tasks_set":
                            tasks_set += 1
                            self.tasks_set_sizes.append(chars)
                        if name == "edit_file" or (name == "spawn_agents" and any(
                                a.get("mode") == "code" for a in (args.get("agents") or [])
                                if isinstance(a, dict))):
                            last_edit = run_rounds
                        if name == "bash":
                            command = re.sub(r"^(cd [^;&]+(&&|;)\s*)+", "",
                                             (args.get("command") or "").strip())
                            words = command.split()
                            self.bash_first[words[0][:20] if words else ""] += 1
                    elif btype == "tool_result":
                        name, read_key = tool_of.get(block.get("tool_use_id"), ("?", None))
                        self.result_chars[name] += chars
                        self.result_sizes.append(chars)
                        live["result:" + name] += chars
                        parts.append(("result", chars))
                        if read_key and read_key[0]:
                            self.reads += 1
                            self.read_chars += chars
                            if result_text(block).startswith("[unchanged: "):
                                # Answered with a pointer to the earlier copy,
                                # which stays the one in history.
                                self.exact_rereads += 1
                                self.pointers += 1
                            else:
                                if read_key in seen_reads:
                                    self.exact_rereads += 1
                                    self.superseded_chars += seen_reads[read_key]
                                seen_reads[read_key] = chars
                    elif btype == "text":
                        live[role + "_text"] += chars
                        parts.append(("text", chars))
                        if role == "assistant":
                            self.generated["assistant_text"] += chars
                messages.append(parts)

        self.peaks.append(peak)
        self.tasks_per_session.append(tasks_set)

        # The simplest rule-based elision, measured where it would have run:
        # every tool result outside the first message and the last six, cut to
        # a 300-char head.
        if peak >= 96_000 and len(messages) > 8:
            total = sum(chars for parts in messages for _, chars in parts)
            freed = sum(max(0, chars - 300) for parts in messages[1:-6]
                        for cat, chars in parts if cat == "result")
            if total:
                self.elision.append(freed / total)

        if not users:
            self.fresh["no prompt"] += 1
        elif linked:
            self.fresh["successor"] += 1
        else:
            first_ts = users[0].get("ts", 0)
            start = "builder"
            for rec in records:
                if rec.get("kind") == "mode" and rec.get("ts", 0) <= first_ts:
                    start = "brain" if is_brain(rec.get("mode")) else "builder"
            self.fresh[start] += 1
            self.first_prompt[start].append(len(users[0].get("text") or ""))
            self.rounds[start].append(sum(
                1 for r in records if r.get("kind") == "turn" and r.get("role") == "assistant"))
            if start == "builder" and any(
                    r.get("kind") == "mode" and r.get("ts", 0) > first_ts
                    and is_brain(r.get("mode")) and r.get("origin") == "human"
                    for r in records):
                self.later_brain += 1

    def report(self):
        if not self.sessions:
            print("no sessions found")
            return
        day = lambda ms: datetime.datetime.fromtimestamp(ms / 1000, datetime.timezone.utc).date()
        span = f"{day(min(self.created))} to {day(max(self.created))}" if self.created else "?"
        print(f"SESSIONS {self.sessions} ({span}) | runs {self.runs} | "
              f"input {self.tokens_in:,} tok | output {self.tokens_out:,} tok")

        print("\nCACHE (runs that recorded cached input)")
        if self.cache_runs:
            total = self.cache_read + self.cache_miss
            print(f"  runs {self.cache_runs} | cached {self.cache_read:,} | uncached "
                  f"{self.cache_miss:,} | cached share {pct(self.cache_read, total)}")
        else:
            print("  none yet — recorded from the build that added `cache_read_tokens`")

        print("\nPEAK CONTEXT per session (tokens on the harness's meter)")
        buckets = Counter()
        for peak in self.peaks:
            buckets["not recorded" if peak == 0 else "<32k" if peak < 32_000
                    else "32-64k" if peak < 64_000 else "64-96k" if peak < 96_000
                    else "96-120k" if peak < 120_000 else "120-150k" if peak < 150_000
                    else ">=150k"] += 1
        for name in ("not recorded", "<32k", "32-64k", "64-96k", "96-120k", "120-150k", ">=150k"):
            print(f"  {name:13} {buckets[name]:5} {pct(buckets[name], len(self.peaks))}")
        print(f"  compactions {self.kinds['compacted']} | prunes {self.kinds['pruned']} | "
              f"handoffs {dict(self.handoffs)}")

        if self.prefix["total"]:
            print("\nFIXED PREFIX per run (tokens, median / p90)")
            for part in ("total", "system", "tools", "mcp", "skills", "specs"):
                vals = self.prefix[part]
                print(f"  {part:7} {quantile(vals, .5):7} / {quantile(vals, .9)}")
            print(f"  context windows seen: {dict(self.windows)}")
            profiles = {f"{p}/{s}": n for (p, s), n in self.profiles.items()}
            print(f"  profile/surface per run: {profiles}")

        total_carried = sum(self.carried.values())
        print("\nCARRIED MASS (chars x later requests that re-read them)")
        for name, value in self.carried.most_common(16):
            print(f"  {name:28} {pct(value, total_carried)}")
        by_class = Counter()
        for name, value in self.carried.items():
            by_class[name.split(":")[0]] += value
        print("  by class:", {k: pct(v, total_carried) for k, v in by_class.most_common()})

        total_generated = sum(self.generated.values())
        print(f"\nGENERATED by the main session (visible chars, total {total_generated:,})")
        for name, value in self.generated.most_common(10):
            print(f"  {name:28} {pct(value, total_generated)}")
        with_tasks = [n for n in self.tasks_per_session if n]
        print(f"  tasks_set: {sum(with_tasks)} calls in {len(with_tasks)} sessions | per session "
              f"median {quantile(with_tasks, .5)} max {max(with_tasks or [0])} | chars per call "
              f"median {quantile(self.tasks_set_sizes, .5)} p90 {quantile(self.tasks_set_sizes, .9)}")

        total_calls = sum(self.calls.values())
        print(f"\nTOOL CALLS in the main session (total {total_calls:,})")
        print(f"  {'tool':22} {'calls':>6} {'share':>6} {'avg result chars':>17}")
        for name, count in self.calls.most_common(24):
            print(f"  {name:22} {count:6} {pct(count, total_calls):>6} "
                  f"{self.result_chars[name] // max(count, 1):17}")
        bash_total = sum(self.bash_first.values())
        via_shell = sum(v for k, v in self.bash_first.items() if k in SHELL_READERS)
        print(f"  bash calls that are a search or a read ({', '.join(sorted(SHELL_READERS))}): "
              f"{pct(via_shell, bash_total)}")
        print("  bash first word:", [(k, pct(v, bash_total)) for k, v in self.bash_first.most_common(10)])

        if self.sub_runs:
            sub_total = sum(self.sub_tools.values())
            print(f"\nSUBAGENTS {self.sub_runs} runs {dict(self.sub_modes)} | tool calls {sub_total:,}")
            for name, count in self.sub_tools.most_common(16):
                print(f"  {name:22} {count:6} {pct(count, sub_total):>6}")

        big = sum(1 for c in self.result_sizes if c >= 8_000)
        big_chars = sum(c for c in self.result_sizes if c >= 8_000)
        print(f"\nTOOL RESULTS n {len(self.result_sizes):,} | median {quantile(self.result_sizes, .5)} "
              f"p90 {quantile(self.result_sizes, .9)} | >=8k chars: {pct(big, len(self.result_sizes))} "
              f"of results holding {pct(big_chars, sum(self.result_sizes))} of the chars")
        print(f"  read_file exact repeats (same path and range): {self.exact_rereads} of {self.reads} "
              f"({pct(self.exact_rereads, self.reads)}); superseded copies are "
              f"{pct(self.superseded_chars, self.read_chars)} of read chars; "
              f"{self.pointers} answered with a pointer instead of the file")
        if self.elision:
            print(f"  rule elision on the {len(self.elision)} sessions that peaked >=96k: frees "
                  f"median {quantile(self.elision, .5):.0%}, p10 {quantile(self.elision, .1):.0%}; "
                  f"{sum(1 for x in self.elision if x >= .25)} clear the 25% bar")

        print(f"\nWHO DECIDES TO PLAN {dict(self.fresh)}")
        print(f"  builder-start sessions the user later switched to brain: {self.later_brain} | "
              f"enter_plan_mode calls by the model: {self.calls['enter_plan_mode']}")
        for mode in ("brain", "builder"):
            print(f"  {mode:8} first prompt median {quantile(self.first_prompt[mode], .5)} chars | "
                  f"assistant rounds median {quantile(self.rounds[mode], .5)}")
        if self.routes:
            # Shadow verdicts with an answer are the calibration set: the
            # router's pick next to what the user actually started the session as.
            judged = {k: v for k, v in self.routes.items() if k[3] and k[2] != "unavailable"}
            agree = sum(v for k, v in judged.items() if k[0] == k[1])
            wrong_brain = sum(v for k, v in judged.items() if k[0] == "brain" and k[1] != "brain")
            total = sum(judged.values())
            print(f"  auto-route in shadow: {total} verdicts, agree with the user {pct(agree, total)}, "
                  f"would have sent a builder session to brain {pct(wrong_brain, total)}")
            print("  auto-route records (verdict, ran_as, source, shadow):", dict(self.routes))

        shares = [after / total for after, total in self.tails]
        print(f"\nRUNS rounds median {quantile(self.rounds_per_run, .5)} p90 "
              f"{quantile(self.rounds_per_run, .9)} | after the last edit (n {len(self.tails)}): "
              f"median {quantile([a for a, _ in self.tails], .5)} rounds, "
              f"{quantile(shares, .5):.0%} of the run")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("sessions_dir", nargs="?", default=os.path.join(".claudinio", "sessions"))
    parser.add_argument("--since", help="only sessions created on or after YYYY-MM-DD (UTC)")
    args = parser.parse_args()
    since_ms = 0
    if args.since:
        since = datetime.datetime.strptime(args.since, "%Y-%m-%d").replace(tzinfo=datetime.timezone.utc)
        since_ms = int(since.timestamp() * 1000)
    files = sorted(glob.glob(os.path.join(args.sessions_dir, "*.jsonl")))
    if not files:
        sys.exit(f"no .jsonl files in {args.sessions_dir}")
    stats = Stats()
    for path in files:
        stats.read_session(path, since_ms)
    stats.report()


if __name__ == "__main__":
    main()
