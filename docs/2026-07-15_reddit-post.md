**Title: A dedicated harness for Claudinio — Claudinio Code!**

Since Claudin.io went online, I've been testing everything — opencode, pi.dev, Claude Code, Cursor, you name it. They're all good tools. But here's the thing: every single one of them felt like they were leaving something on the table. Claudin.io is a beast of a model, and these generic harnesses just weren't tapping into what it can actually do.

So I built my own.

**Claudinio Code** is a native desktop app — not a terminal, not a browser tab, not an extension bolted onto someone else's editor. It's a Tauri v2 + Rust desktop application built from the ground up to be *the* harness for Claudin.io. I started it because I have plans to give even more power to Claudin.io, and this makes it possible.

And honestly? I am very addicted coding with this harness. Claudinio is running smoothly — smoother than anything I've used before.

`[video demo]`

---

### What makes it different

I didn't just wrap an API. I built the harness I *wanted* to use. Here's what that means in practice:

**Brain and Builder modes that mirror how I actually work.** Brain mode plans — read-only exploration, interviewing me about requirements, writing a proper spec. Builder mode executes — writing code, running commands, verifying. The harness enforces this separation so Claudinio doesn't skip planning and start hacking (we've all been there). And when one mode finishes its job, it automatically flips to the other.

**The agent actually reads your codebase.** Not just grep. Claudinio Code indexes your entire workspace with tree-sitter (100+ languages), builds a semantic embedding index running ONNX locally, and connects to LSP servers for go-to-definition and find-references. When Claudinio asks "where is the auth middleware?", it gets a real answer from the index, not a hallucination.

**Parallel subagents.** Claudinio can spawn up to 4 subagents simultaneously — each with its own fresh context, working on independent parts of the problem. You can click into any subagent and see its full thinking timeline. This is the feature that made me realize how much other tools were leaving on the table.

**Full transparency — a visual timeline.** Every thought, every tool call, every subagent action is rendered in a collapsible timeline. You see exactly what Claudinio is thinking and doing. No black box.

**Steering — redirect mid-thought.** While Claudinio is working, you can type guidance and it gets injected into the current turn. "Actually, use the existing hook instead of creating a new one" — and it adapts in real time. Plus interrupt with Escape anytime.

**Golden goals.** Wrap anything in `<goal>` tags and the harness enforces it as a mandatory task. Claudinio *must* complete it before the session can end. It will cycle between Brain and Builder automatically until every golden goal is done. No more "I'll do that in a follow-up."

**Plans with living implementation logs.** When Claudinio plans something, the plan is written to a `.md` file in your workspace. When it builds it, the implementation log — including actual git commits and changed files — gets appended to the same plan. Your plans become living documents that trace the full journey from idea to shipped code.

**A real desktop app.** 15 custom themes (all OKLCH design system — perceptually-uniform colors), native file dialogs, proper window chrome, keyboard shortcuts, drag-and-drop file attachments, auto-updater. It lives in your dock, not your terminal.

---

### Where to get it

It's MIT-licensed and builds for **Windows, macOS (Apple Silicon), and Linux** (x64 and ARM64). Releases are on GitHub with auto-update support.

If you've been using Claudin.io through other tools and feeling like there's more to squeeze out of it — there is. This harness was built to prove it.

Happy to answer questions in the comments. And if you try it, let me know what you think — I'm building this thing actively and feedback goes straight into the next release.
