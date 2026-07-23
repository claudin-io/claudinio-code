---
format: 1920x1080
duration: 75s
message: "Claudinio Code is open source — a native desktop harness that really reads your code and asks before it touches anything"
arc: "Hook (black box) → Product + open source → Feature-Benefit cascade (modes · code intelligence · visibility · approvals · extensibility) → CTA"
audience: "developers who already use AI coding agents and distrust the black box"
mode: collaborative
music: "restrained technical electronic bed — steady pulse, low synth, no drums until the CTA, never competes with the voice"
---

## Video direction

**Palette system** — from `frame.md`, by role, nothing invented. `cream` (#edeef5) is the
ground of every frame; `ink` (#090910) is the voice; `coral` (#5c60e6 — the Claudinio indigo)
is the **voltage and appears exactly once per frame**, always on that frame's focal element.
`tile` / `tile-strong` are the half-step surfaces content gathers on. `navy` / `navy-soft` /
`navy-elev` are the app's own `surface-1/2/3` and appear **only** where the product's interface
shows itself — which is the point: the app UI is the one dark object in a light film, so the
eye goes to it in every frame. Type by role only: `display` for headlines, `lead` for the
benefit line, `kicker` (mono, uppercase, ✱ prefix) for the eyebrow, `mono-label` / `code`
for anything the app itself would render in JetBrains Mono.

**Motion grammar + reveal model** — one camera, one feel. Long-tail decel settles
(`power3`) everywhere; **no overshoot, no bounce** — this is a developer tool, and bounce
reads as a toy. Every frame is **paced to the voiceover**: at t=0 only what the line is saying
enters, and each further element reveals on its own spoken cue, with the majority of reveals
living in the **back half** of the shot. The app UI is never dumped complete — it assembles
the way it would actually populate (rows arrive, results resolve, counters climb), because a
UI that builds itself is the proof that it is a real interface and not a screenshot.

**Rhythm / held-frame allocation** — the film must not run at one energy. Deliberate held
beats: **frame 6 scene 2** (the agent stopped, waiting on approval — the stillness IS the
feature, and it is the single most important hold in the video), **frame 4's closing seal**,
and the last ~1.5s of **frame 8**. Frames 3, 4 and 5 are the busy ones; 1, 7 and 8 are
comparatively calm. During any hold the only sanctioned aliveness is low-amplitude
**subtle jitter** — never breathing, never a back-half pan or push.

**Negative list** — never appears: a terminal or shell prompt (frame 2's whole argument is
that this is not a terminal wrapper); browser chrome, nav bars, footers, scrollbars; purple-blue
"AI" gradients, bokeh, glassmorphism; drop shadows heavier than the pack's hairline elevation;
rounded-bouncy easing; any number not present in `README.md`. Both motion failure modes are
banned by name: **slideshow** (everything dumped in the first 25%, then frozen) and
**screensaver** (elements floating independently to fake life).

## Frame 1 — Working in the dark

- scene: The pain stated in type, then a file silently rewrites itself with no one watching
- duration: 7.339s
- transition_in: cut
- status: animated
- blueprint: kinetic-type-beats (Reproduce)
- narrative_role: Hook
- voiceover: "Most coding agents work in the dark — they grep, they guess, and they edit your files before you've seen what changed."
- asset_candidates: none (typographic frame; a bare code surface using colors.navy)
- src: compositions/frames/01-dark.html
- focal: the third strip panel (the edit that already landed)
- roles: type = foreground · navy code strip = supporting
- sfx: impact-bass-1

Opens cold on the viewer's own experience, in outcome language: not "here is a
tool", but "you cannot see what your agent is doing." The line resolves the pain
in the same breath it names it, so beat 2 can land the value claim. No product,
no logo, no UI yet — the frame earns the right to introduce them.

Reproduce: the statement builds across full-screen beats, each its own move, onto the
locked finale — exactly the blueprint's shape. Signature move (the beat-built statement) kept.

Scene 1 (0.0–2.40s): cream field, empty. The kicker sets upper-left, then the display line
builds by **per-word staggered reveal** (`dynamic-content-sequencing`) on a long-tail settle,
each word landing on its spoken word (`Most`@0.05 · `coding`@0.35 · `agents`@0.82 ·
`work`@1.31 · `dark.`@2.08). Nothing else on screen. Rule-of-thirds, headline occupying the
upper-left two thirds, ~45% of canvas.
Scene 2 (2.40–4.30s): the navy strip panels arrive **one per spoken cue** — panel 1 on
`grep,`@2.76, panel 2 on `guess,`@3.66 — each a **hard-cut swap** into place
(`discrete-text-sequence`) at matched velocity, left to right along the lower third. The
headline stays put; the strip is the development. Full-width strip below the type, 3 depth
layers (cream ground · navy panels · ink type).
Scene 3 (4.30–5.49s): panel 3 lands on `edit`@4.47 / `files`@5.06, in coral — the only coral
in the frame.
Scene 4 (5.49–7.339s): on `before`@5.49 → `changed.`@6.78 a diff line inside panel 3 silently
rewrites itself. Then everything **holds still**; at most subtle jitter (`sine-wave-loop`, low
amplitude) on that panel. The stillness is the discomfort.

## Frame 2 — Claudinio Code, in the open

- scene: Tight on one timeline row, then one continuous zoom-out reveals the whole three-panel desktop app; MIT + open source lands on the held frame
- duration: 8.533s
- transition_in: crossfade
- status: animated
- blueprint: zoom-out-workspace-reveal (Reproduce)
- narrative_role: Product_Intro
- voiceover: "Claudinio Code is a native desktop harness for AI coding agents. And as of today, it's open source. MIT."
- asset_candidates: app-shell, logo.png
- src: compositions/frames/02-open.html
- focal: app-shell (the whole window, once revealed)
- roles: app-shell = cutout (hero) · logo.png = supporting (small, beside the wordmark)
- sfx: whoosh-cinematic

The value claim, by beat 2 as the spine requires. The zoom-out is the argument:
the detail you were just shown is part of a real desktop application, not a
terminal wrapper. The open-source line lands on the settled wide frame — the
occasion arrives after the thing itself is visible.

Reproduce: open tight on one detail, let micro-action play, then ONE continuous decelerating
zoom-out reveals the containing whole. Signature move (the single uninterrupted zoom-out) kept —
and there is **no zoom-in anywhere** in this frame.

Scene 1 (0.0–1.29s): full-bleed on a single navy timeline row at extreme close scale — a tool
result streaming in character by character. The viewer cannot yet tell what they are looking
at. Layered-depth, the row filling 100% of frame.
Scene 2 (1.29–4.40s): on `native`@1.29 → `harness`@2.49, **one continuous decelerating
zoom-out** (`viewport-change`) pulls back to reveal the row's container — the chat panel —
then the whole three-panel window: file tree, viewer, timeline, settling as `agents,`@3.92
lands. The move never reverses and never re-pushes. The window ends occupying ~70% of the
canvas, centered, cream around it.
Scene 3 (4.40–6.20s): the frame locks. Wordmark and the lead line reveal beneath the window,
lower-left, by per-word staggered reveal — element-level payoff, camera now static.
Scene 4 (6.20–8.533s): on `open`@6.20 / `source,`@6.62 the coral chip **spring-pops** in
(`spring-pop-entrance`, smooth settle, no overshoot) at lower-right, opposing the wordmark;
`MIT.`@7.55 sets inside it. Holds still to the end.

## Frame 3 — Brain and Builder

- scene: The segmented mode toggle flips from Brain to Builder; a written plan scrolls past and hands off into a visibly fresh session
- duration: 13.547s
- transition_in: cut
- status: animated
- blueprint: agent-progress-theater (Adapt)
- narrative_role: Key_Feature
- voiceover: "Two modes. Brain explores read-only and writes the plan. Builder executes it in a fresh session that carries only the plan — so execution never inherits a context window full of exploration."
- asset_candidates: mode-toggle, plan-block, app-shell
- src: compositions/frames/03-modes.html
- focal: the mode toggle at the flip, then the fresh session card
- roles: mode-toggle = cutout · plan-block = supporting · app-shell = background (dim, behind the stage)
- sfx: click-soft, whoosh-short

First capability, and the one no competitor frames this way. The benefit is the
second half of the line — a clean context window — so the visual must show the
handoff as a real boundary (session ends, new session opens carrying the plan),
not as a label change.

Adapt: keep the working-state theater — a trigger hands the frame to the machine, it visibly
works, the receipt cascades in. Changed: the receipt is not a checklist but a **written plan**,
and the shot continues past it into the session handoff, which the blueprint does not cover.
Signature move (state mutation as the demo) kept and carried by the toggle flip.

Scene 1 (0.0–1.31s): asymmetric 60/40 — headline left, stage right. The kicker and
"Two modes." enter (`Two`@0.03 · `modes.`@0.37); the toggle assembles on the right with
**Brain** active in coral. Nothing else yet.
Scene 2 (1.31–3.25s): on `Brain`@1.31 → `read-only`@2.59, the tool row beneath the toggle
populates — and is **visibly short**: read tools only, with `edit_file` and `bash` struck out.
Cluster→outward expansion (`center-outward-expansion`) from the toggle.
Scene 3 (3.25–4.59s): on `writes`@3.25 / `plan.`@3.96, the plan document **types itself** into
the middle of the stage (`discrete-text-sequence` with caret) — headings then bullets, mono,
on navy.
Scene 4 (4.59–5.98s): on `Builder`@4.59 / `executes`@5.06, the toggle **hard-cuts** to Builder
at peak velocity; the coral moves with it, the tool row refills to full width in the same beat.
This is the frame's signature moment — the state mutation IS the demo.
Scene 5 (5.98–8.95s): on `fresh`@5.98 → `plan,`@8.53, the session-1 card shrinks and dims out
while the session-2 card arrives at the same center carrying only the plan — **scale-swap**
handoff (`scale-swap-transition`). Split-screen briefly, resolving to the new session alone.
Scene 6 (8.95–13.547s): the lead line lands beneath on `so`@8.95 → `exploration.`@12.61 — and
the frame holds still on the fresh, nearly-empty session. This is a long tail; let it be still
rather than filling it.

## Frame 4 — It actually reads your codebase

- scene: The index fills — file counts, symbols, then embeddings joining; two queries run side by side, one conceptual, one an exact spelling, both landing the right hit; a local-only seal holds at the end
- duration: 13.781s
- transition_in: crossfade
- status: animated
- blueprint: prompt-type-submit-generate (Adapt)
- narrative_role: Key_Feature
- voiceover: "It actually reads your codebase. Tree-sitter indexes seventy-seven languages; keyword matching and local embeddings fuse into one ranking. And the embeddings run on your machine — your code never leaves it."
- asset_candidates: index-progress, search-result, app-shell
- src: compositions/frames/04-codebase.html
- focal: the fused ranking, then the local-only seal
- roles: search-result = cutout · index-progress = supporting · app-shell = background (dim ~40%)
- sfx: typing, chime

The longest frame, because this is the real differentiator. Two queries are
non-negotiable: showing only the semantic one invites "so it's just embeddings",
and only the exact one invites "so it's just grep". The privacy line closes it —
for this audience, local embeddings are a buying reason, not a footnote.

Adapt: keep the ask→answer loop — a query types into a real input and the machine answers.
Changed: **two** queries run as a paired stage rather than one, because the whole claim is
that two retrieval methods fuse. Signature move (type → submit → the machine answers) kept,
run twice in parallel.

Scene 1 (0.0–2.47s): kicker and display headline enter alone, upper-left, per-word staggered
reveal (`It`@0.03 · `actually`@0.23 · `reads`@0.90 · `code base.`@1.71). Cream field, nothing
else. ~40% of canvas.
Scene 2 (2.47–5.83s): on `Treesitter`@2.47 / `indexes`@3.48, the index strip reveals across the
full width and its counters **count up** (`counting-dynamic-scale`) — files, then symbols —
while a progress fill sweeps (`stat-bars-and-fills`). The language counter lands reading 77
exactly on `77`@4.21, and `languages.`@4.92 settles it.
Scene 3 (5.83–8.27s): on `Keyword`@5.83 / `matching`@6.27, the stage splits into two query
cards; the left **types on** the conceptual question. On `local`@7.05 / `embeddings`@7.37 the
right types the exact token — both with carets (`discrete-text-sequence` +
`context-sensitive-cursor`), staggered so the eye reads left then right. Split-screen, 3 depth
layers.
Scene 4 (8.27–9.99s): on `fuse`@8.27 → `ranking.`@9.23, each card's ranked list resolves and
the two lists **converge into a single ranking** at the center — a cluster→outward expansion
run in reverse (`center-outward-expansion`), the two columns collapsing into one. Coral lands
on the top hit. This is the frame's payoff.
Scene 5 (9.99–13.781s): on `embeddings`@10.39 / `machine.`@11.56 the local-only seal **draws
itself on** (`svg-path-draw`) at lower-right; `never`@12.80 / `leaves`@13.14 land on it and the
frame **holds completely still** — an allocated held beat. Subtle jitter at most.

## Frame 5 — Nothing is a black box

- scene: The frame travels down one long assistant turn — phase dividers, thinking, tool calls with their results, the token and cost footer — then four subagent cards fan out, each with its own live timeline
- duration: 10.069s
- transition_in: cut
- status: animated
- blueprint: transcript-scroll-artifact-reveal (Reproduce)
- narrative_role: Key_Feature
- voiceover: "Every turn is a timeline you can open — phases, thinking, each tool call, tokens, cost. Subagents run in parallel, and each keeps its own."
- asset_candidates: timeline, subagents, app-shell
- src: compositions/frames/05-timeline.html
- focal: the transcript column during the travel, the subagent grid after the pivot
- roles: timeline = cutout · subagents = supporting · app-shell = background (dim ~40%)
- sfx: click-soft

The direct payoff of frame 1's pain — this frame exists to close that loop, and
the vertical travel is the proof: the transcript keeps going, which is the point.
Do not summarize the timeline into a diagram; the density is the argument.

Reproduce: travel vertically along one long full-bleed content surface, reading the generated
work as evidence, then ONE focal interaction pivots into the artifact reveal. Signature move
(the vertical traversal as proof) kept; the pivot is the fan-out to subagents.

Scene 1 (0.0–2.72s): the turn sits collapsed as a single navy row, left column. On
`timeline`@0.98 → `open,`@2.15 it **expands open** downward — rows unfolding. Asymmetric 40/60.
Scene 2 (2.72–6.88s): the frame **travels down** the transcript (`viewport-change`, vertical,
constant velocity) and each row lights exactly as the VO names it via **keyword glow**
(`asr-keyword-glow`) synced to the word rail: `phases,`@2.72 · `thinking,`@3.55 ·
`each tool call,`@4.36 · `tokens,`@5.41 / `cost.`@5.84 where the footer arrives in coral. The
column continues past the bottom edge: there is visibly more than fits, which is the argument.
Scene 3 (6.88–8.49s): on `Subagents`@6.88 / `parallel`@7.53, the travel stops and four cards
**expand outward** from the `spawn_agents` row into a 2×2 grid on the right
(`center-outward-expansion`), staggered by index. The transcript stays visible, dimmed, left.
Scene 4 (8.49–10.069s): on `each`@8.49 / `keeps`@8.84 / `own.`@9.60 each card's own
mini-timeline populates a few rows, staggered — then the whole frame holds. No camera move in
the back half.

## Frame 6 — It asks before it touches anything

- scene: A bash approval card stops the run and waits; then a proposed edit opens as a side-by-side diff, and only on approve does the file change
- duration: 6.72s
- transition_in: crossfade
- status: animated
- blueprint: cursor-ui-demo (Adapt)
- narrative_role: Key_Feature
- voiceover: "Shell commands stop and ask. File edits arrive as a diff you approve before a single byte is written."
- asset_candidates: approval-bash, approval-diff, app-shell
- src: compositions/frames/06-approvals.html
- focal: the Allow button at the press
- roles: approval-bash = cutout · approval-diff = supporting · app-shell = background (dim ~40%)
- sfx: click

The second differentiator and the emotional close of the argument that started
in frame 1. The pause must read as a real stop — the agent visibly waiting — or
the promise reads as decoration. Land the write only after the approve.

Adapt: keep the cursor driving a reconstructed UI so the screen changes state shot to shot.
Changed: the blueprint's continuous click-through is broken by a **deliberate dead stop** —
the cursor arrives and does nothing while the agent waits. Signature move (cursor-driven state
change) kept, but the shot's meaning lives in the pause before it.

This frame is the shortest of the feature beats (6.72s) and carries five moves — keep every
entrance tight and let the hold in Scene 2 be genuinely empty. Do not compress the hold to buy
time for the diff; the hold is the feature.

Scene 1 (0.0–1.79s): the running agent occupies the left — a spinner and a streaming row. On
`Shell`@0.06 / `commands`@0.45 the approval card slides up over it and on `stop`@1.22 /
`ask.`@1.79 **the spinner stops mid-rotation**. The command reads `cargo test --all` in mono
on navy. Centered-left, ~45%.
Scene 2 (1.79–2.40s): **held still.** Nothing moves except a single low-amplitude jitter on
the "waiting on you" line (`sine-wave-loop`). Short in seconds but it must read as a dead
stop — the product's promise is a pause, so the shot performs the pause.
Scene 3 (2.40–4.14s): on `File`@2.40 / `edits`@2.68 / `diff`@3.83, the diff panel opens to the
right — split-tilt entry (`split-tilt-cards`), original and modified panes — and the changed
lines land staggered, red then green gutters, nothing yet written.
Scene 4 (4.14–4.94s): on `you`@4.14 / `approve`@4.81 the cursor moves to Allow and **presses**
— compression then spring recovery with a ripple (`cursor-click-ripple` +
`press-release-spring`). Coral fires here and only here.
Scene 5 (4.94–6.72s): on `before`@4.94 → `written.`@6.13 the modified pane commits — the
changed lines settle into the file and the gutter clears. Holds still on the written result.

## Frame 7 — Yours to extend

- scene: A skill file drops into .agents/skills and is discovered; MCP servers connect; the model list assembles showing claudin.io alongside other providers
- duration: 8.725s
- transition_in: cut
- status: animated
- blueprint: grid-card-assemble (Reproduce)
- narrative_role: Benefits
- voiceover: "Drop in a skill file and it's discovered. Connect MCP servers. And it runs on any Anthropic-compatible API."
- asset_candidates: skills-mcp, provider-list
- src: compositions/frames/07-extend.html
- focal: the provider card (the third)
- roles: skills-mcp = supporting · provider-list = cutout
- sfx: pop

Breadth, enumerated at once rather than demonstrated — three capabilities that
each deserve their own video get one assembling grid. This is also the frame
that makes "open source" concrete: it is extensible because you can reach into it.

Reproduce: N items self-assemble in a staggered cascade into a grid and hold. Signature move
(the staggered self-assembly) kept — one card per spoken cue rather than all three at once.

Scene 1 (0.0–0.60s): kicker and "Yours to extend." enter alone, upper-left, per-word staggered
reveal. Triptych stage empty beneath. Very short — the VO starts naming card 1 almost at once.
Scene 2 (0.60–2.98s): on `skill`@0.60 / `file`@1.08, card 1 assembles — the card body first,
then its mono rows staggered downward — and a `SKILL.md` chip drops into it on
`discovered.`@1.99.
Scene 3 (2.98–5.42s): on `Connect`@2.98 / `MCP`@3.97 / `servers`@4.24, card 2 assembles the
same way; two server rows connect with a short line draw (`svg-path-draw`).
Scene 4 (5.42–8.21s): on `runs`@5.84 → `compatible`@7.24, card 3 assembles and its provider
rows cascade in — claudin.io first and in coral, the others in ink. This card is the focal.
Scene 5 (8.21–8.725s): `API.`@8.21 lands and the triptych holds, all three equal weight, still.

## Frame 8 — Go read the source

- scene: The app recedes and the logo assembles into a centered lockup with the repo URL, MIT, and the three platforms
- duration: 6.421s
- transition_in: crossfade
- status: animated
- blueprint: logo-assemble-lockup (Reproduce)
- narrative_role: CTA
- voiceover: "Claudinio Code. MIT, on GitHub. macOS, Windows, Linux. Go read the source."
- asset_candidates: logo.png
- src: compositions/frames/08-cta.html
- focal: logo.png
- roles: logo.png = cutout (hero, centered)
- sfx: impact-bass-2

The ask is "read the source", not "sign up" — it matches the occasion and the
audience. The URL must be legible and held long enough to be typed from memory.

Reproduce: the mark comes to exist on a cleared stage and resolves into a centered lockup,
extended to the URL and CTA. Signature move (the mark building itself) kept.

Scene 1 (0.0–0.99s): cleared cream stage. The logo **assembles** at center on `Claudinio`@0.11
— its rounded square draws on (`svg-path-draw`) and the mark blooms from zero on a smooth
settle. Centered, hero.
Scene 2 (0.99–1.73s): on `Code,`@0.99 the wordmark reveals beneath it by per-word staggered
reveal, locking the lockup.
Scene 3 (1.73–3.54s): on `MIT`@1.73 / `GitHub,`@2.45 the repo URL **types on** in coral mono
beneath the wordmark (`discrete-text-sequence` with caret) — the one element the viewer must
be able to read and retype, so it types steadily and then holds untouched to the end.
Scene 4 (3.54–5.31s): the platform chips arrive staggered left to right on `Mac`@3.54 /
`OS`@3.81 / `Windows,`@3.99 / `Linux,`@4.81 (`center-outward-expansion` from the lockup); the
MIT chip is already set from Scene 3.
Scene 5 (5.31–6.421s): "Go read the source." lands on `go`@5.31 → `source.`@5.80 and the whole
lockup **holds completely still** for the last ~0.6s. This is the only frame with a real exit;
everything settles and stops.
