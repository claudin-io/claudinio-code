---
name: design
description: Designing a feature before code — the consensus checklist, UI sizing and assets, the design document, diagrams.
---

# design

Read this when a request is large, touches the UI, or carries assets the user
supplied. A small, fully specified request does not need it: do the work.

## 1. Interview: ask what changes the work

Every question costs the user a round trip, so ask only about decisions whose
answer changes the work, and never about a fact the codebase can tell you —
look that up instead. Explore before you ask, so each question rests on facts.

- Bundle independent questions in one `ask_user` call; ask a question that
  depends on an earlier answer after that answer arrives.
- Put your recommended answer first in every question, suffixed
  ' (Recommended)'.
- Finish with one question that confirms the agreed design, then write it
  down (section 3).

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
  you will use it exactly, and record it VERBATIM in the document and in any
  subagent's goal — the full URL, the exact icon id such as
  `lucide:notebook-pen`, the real SVG — so whoever builds it uses real
  content, not guesses. (Session 44ec41c1 shipped the wrong icon because a
  task said "similar to".)

## 3. The design document

A design worth writing down goes in the session's living document:
`doc_create` with 3-6 sections, each an independent question the design
needs answered — how the code does it today, the options, the data, the UI —
not the Risks or Next steps of one answer. Give every independent section to
an explore subagent in one `spawn_agents` call (`section`), and write yourself
only the section that depends on them: the decision, with what was agreed
(scope, UX and sizing, verbatim assets, edge cases, non-goals) and how it will
be built, grounded in the real codebase:

- the exact files and symbols to touch;
- data flow;
- the APIs and schemas involved;
- existing patterns to reuse;
- integration points.

Every claim comes from the codebase, not from memory. A technical trade-off
that would change the agreed design (a conflict with an existing pattern, a
much cheaper alternative) goes to the user with `ask_user` before it goes into
the document. Then build it: the document replaces a task list, so its
sections are the plan, and a subagent you delegate a change to gets every
value the user gave verbatim (URLs, asset and icon ids, real snippets, agreed
sizes) — it never saw this conversation and cannot ask.

## 4. Diagrams (Mermaid)

When an idea is clearer as a picture than as prose, draw a Mermaid diagram (a
```mermaid fenced code block) — in chat and inside the document. Drawing it also sharpens your own reasoning. The app renders
the FULL Mermaid catalog, so pick the most expressive type: `flowchart` for
control flow and architecture, `sequenceDiagram` for interactions,
`classDiagram` for data models, `stateDiagram-v2` for state machines,
`erDiagram` for schemas, `gantt` for phases, `journey` for user journeys,
`mindmap` for breakdowns, `timeline` for chronology, `gitGraph` for branching,
`quadrantChart`, `pie` or `xychart` for trade-offs and distributions. Keep each
diagram focused; text alone is fine for a trivial request. Labels follow the
language policy.
