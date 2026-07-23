---
workflow: product-launch-video
flow: automation
storyboard: yes
message: "Claudinio Code is open source — a native desktop harness that really reads your code and asks before it touches anything"
destination: youtube
aspect: 1920x1080
language: en
length: 75s
angle: open-source-launch
audience: "developers who already use AI coding agents and distrust the black box"
---

## Intent

Announce that Claudinio Code is now open source (MIT, `github.com/claudin-io/claudinio-code`)
and, in the same breath, show why it is worth a look. Feature-led, not
manifesto-led: the launch is the occasion, the product is the argument.

Tone: technical, confident, unhyped. The audience are developers who have
already tried three agent tools this year — they respond to specifics
(77 languages, BM25 + embeddings fused with RRF, embeddings that never leave
the machine) and switch off at "revolutionary AI-powered". Every number in the
video comes from the README; nothing gets rounded up for effect.

## Assets

- docs/assets/logo.png — the Claudinio mark; opens the video and closes the CTA.
- src/App.css — the app's real design system (`:root` oklch tokens + the
  `@theme inline` block at line 486). Ported into the compositions so the UI
  shown on screen is styled by the app's own CSS, not an approximation.
- src/assets/fonts/InterVariable.woff2, JetBrainsMono-Regular.woff2,
  JetBrainsMono-Medium.woff2 — the app's real fonts.
- src/components/*.tsx — source of the real Tailwind class strings the UI mocks
  reuse verbatim (ChatPanel, ApprovalCard, DiffViewer, FileTree).

## Customizations

- **Rebuild the UI in HTML instead of capturing a site or shooting screenshots.**
  No screenshot exists in the repo, and claudin.io is the service's landing page,
  not the OSS app. The app's Tailwind v4 `@theme` block drops straight into the
  HyperFrames Tailwind browser runtime, so the real component class strings
  render pixel-faithfully — and stay animatable (text streaming in, the diff
  landing line by line, the timeline expanding), which a still cannot do.
- The app-window shell (3-panel chrome) is built once as a reusable
  sub-composition; each frame injects only its own panel content.

## Notes

- 8 beats: open source → what it is → Brain/Builder → code intelligence →
  visible timeline + subagents → approval gates → skills/MCP/providers → CTA.
  Code intelligence and approval gates are the real differentiators and get the
  longest holds.
- The app UI is English-only by design; the video matches it.
- Do not show a terminal. The whole point of beat 2 is that this is not a
  terminal wrapper.
- Narration runs on local Kokoro (offline); no HeyGen credential is used.
