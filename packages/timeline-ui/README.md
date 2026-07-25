# @claudinio/timeline-ui

Timeline rendering shared by the desktop app and, from phase 3 of the remote
access plan, the web peer at `app.claudin.io`.

## The rule this package exists to enforce

**Nothing here may import Tauri, or anything that reaches the app's IPC surface.**
That is not a style preference. The web peer has no Tauri, so a dependency on it
is a build error there — and the moment this package is unbuildable for the web,
someone will copy a file instead of fixing it, and the copy will diverge.

Checked by `nothing in packages/ reaches into the app` in
`src/lib/source-hygiene.test.ts`, because a rule stated only in a README does not
fail a build.

## What is here

`markdown.ts` came first because it owns the sanitize pass that stops injected
HTML in model output from executing (see `SECURITY.md`), and §10 of the plan names
XSS on the web origin as a high risk. Two copies of a sanitizer is one sanitizer
and one liability: the fix goes into whichever the author was looking at.

`records.ts` is the shape a session is persisted and streamed in — types and two
normalizers, nothing else. It was carved out of `lib/ipc.ts`, where every type sat
next to an `invoke` call. `lib/ipc.ts` re-exports it so the forty-odd existing
call sites did not have to churn; the boundary that matters is the direction of
the dependency, not which module a caller names.

Without this, the web peer would grow its own guess at the record format — and a
guess renders an old transcript subtly wrong rather than failing.

`chatRecords.ts` translates those records into what the timeline draws. Pure, and
now importing `./records` instead of the app.

## What has not moved yet, and why

`TimelineRows.tsx` is named for this package in the plan but is not a leaf. It
imports five sibling components and calls `openExternalUrl` from `lib/ipc`, which
is Tauri. Moving it means inverting that dependency into a prop the host supplies
— real work rather than a move, and it belongs with the read-only web UI that will
be its second consumer.
