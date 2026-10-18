---
name: plan
description: Planning a feature in Brain mode — the consensus checklist, UI sizing and assets, the Low-Level Design, diagrams.
---

# plan

Read this when a planning request is large, touches the UI, or carries assets
the user supplied. A small, fully specified request does not need it.

## 1. Interview: ask what changes the plan

Every question costs the user a round trip, so ask only about decisions whose
answer changes the plan, and never about a fact the codebase can tell you —
look that up instead. Explore before you ask, so each question rests on facts.

- Bundle independent questions in one `ask_user` call; ask a question that
  depends on an earlier answer after that answer arrives.
- Put your recommended answer first in every question, suffixed
  ' (Recommended)'.
- Finish with one question that confirms the agreed design, then call
  `write_plan`.

### The consensus checklist

Before the confirming question, every dimension that applies has been asked
about, answered by the request itself, or explicitly ruled out:

1. scope and success criteria;
2. UX, layout and sizing;
3. data and persistence;
4. edge cases and failure states;
5. non-goals — what is explicitly out of scope.

Consensus with a dimension nobody looked at is false consensus: it surfaces
later as a correction, after code was written against the guess.

## 2. UI and visual features: sizing and assets

When the request involves visual content (components, modals, dialogs, panels,
buttons, layouts), these decisions are the user's — ask, do not invent:

- **Sizing and layout**: the dimensions of the new interface (a modal's width
  and height — full screen? fixed px? a share of the viewport? margins?), its
  position and its responsive behaviour. A "modal" with no agreed size is an
  incomplete spec.
- **User-provided assets**: an icon name, URL, image, prototype or exact copy
  the user gave is GROUND TRUTH. Do not paraphrase it as "an icon similar to
  X". Resolve it (fetch the URL, read the image) to get the real data, confirm
  you will use it exactly, and record it VERBATIM in the plan and in the task
  descriptions — the full URL, the exact icon id such as
  `lucide:notebook-pen`, the real SVG — so Builder and its subagents use real
  content, not guesses. (Session 44ec41c1 shipped the wrong icon because a
  task said "similar to".)

## 3. The two documents

The Solution Design (the first `write_plan`) records WHAT was agreed: Context,
Solution Design, Risks, Non-goals — scope, UX and sizing, verbatim assets, edge
cases. No implementation detail (file paths, symbols, schemas) belongs there.

The `## Low-Level Design` (the second `write_plan`, same name, the FULL plan
content plus the new section) records HOW it will be built, grounded in the
real codebase. Research it on your own — 'explore' subagents, indexed search,
targeted reading — and cover:

- the exact files and symbols to touch;
- data flow;
- the APIs and schemas involved;
- existing patterns to reuse;
- integration points.

Every claim comes from the codebase, not from memory. Do not re-open settled
requirements here; but a technical trade-off that would change the agreed
design (a conflict with an existing pattern, a much cheaper alternative) goes
to the user with `ask_user` before it goes into the plan. End the plan with a
short Tasks summary.

Then `tasks_set`: one self-contained task per atomic step, each carrying the
file paths, symbols, constraints, the plan file path and every value the user
gave verbatim (URLs, asset and icon ids, real snippets, agreed sizes), so a
Builder subagent that never saw this conversation, and cannot ask, can do it.
A task that names a design decision without its concrete value is incomplete.

## 4. Diagrams (Mermaid)

When an idea is clearer as a picture than as prose, draw a Mermaid diagram (a
```mermaid fenced code block) — in chat and inside the Solution Design and the
Low-Level Design. Drawing it also sharpens your own reasoning. The app renders
the FULL Mermaid catalog, so pick the most expressive type: `flowchart` for
control flow and architecture, `sequenceDiagram` for interactions,
`classDiagram` for data models, `stateDiagram-v2` for state machines,
`erDiagram` for schemas, `gantt` for phases, `journey` for user journeys,
`mindmap` for breakdowns, `timeline` for chronology, `gitGraph` for branching,
`quadrantChart`, `pie` or `xychart` for trade-offs and distributions. Keep each
diagram focused; text alone is fine for a trivial request. Labels follow the
language policy.
