# @claudinio/timeline-ui

Timeline rendering shared by the desktop app and, from phase 3 of the remote
access plan, the web peer at `app.claudin.io`.

## The rule this package exists to enforce

**Nothing here may import Tauri, or anything that reaches the app's IPC surface.**
That is not a style preference. The web peer has no Tauri, so a dependency on it
is a build error there — and the moment this package is unbuildable for the web,
someone will copy a file instead of fixing it, and the copy will diverge.

`markdown.ts` is here first for that exact reason. It owns the sanitize pass that
stops injected HTML in model output from executing (see `SECURITY.md`), and §10 of
the plan names XSS on the web origin as a high risk. Two copies of a sanitizer is
one sanitizer and one liability: the fix goes into whichever the author was
looking at.

## What has not moved yet, and why

`TimelineRows.tsx` is named for this package in the plan but is not a leaf. It
imports five sibling components and calls `openExternalUrl` from `lib/ipc`, which
is Tauri. Moving it means inverting that dependency into a prop the host supplies
— real work rather than a move, and it belongs with the read-only web UI that will
be its second consumer.

`chatRecords.ts` is portable in substance but imports its record types from
`lib/ipc.ts`, which mixes types with `invoke` calls. It moves once those types are
split out.
