import type { JevStatus, SessionMode } from "./ipc";

/** What the mode control can send with a prompt. "auto" is not a mode a
 *  session is ever in: it hands the choice of starting phase to the harness,
 *  which decides once, on the first prompt (see `agent::route` in the backend). */
export type ModeChoice = SessionMode | "auto";

/** Whether there is a router to hand the choice to: it is switched on, and Jev
 *  — which it asks — is enabled and has a credential to run on. The setting is
 *  on by default, so without the last check everyone who never set Jev up
 *  would be offered an Auto that can only ever answer "Builder". */
export function routerOn(jev: JevStatus | undefined): boolean {
  return jev?.enabled !== false && jev?.route === "on" && jev.backend != null;
}

/** Auto is offered only where it can mean something: there is a router, and
 *  the session has not had its first prompt yet. Past that point the session
 *  is in a real mode and the control shows it. */
export function autoAvailable(jev: JevStatus | undefined, sessionIsFresh: boolean): boolean {
  return sessionIsFresh && routerOn(jev);
}

/**
 * What goes out with a prompt. `armed` is whether Auto was left on for this
 * conversation — and deliberately not whether the conversation still looks
 * fresh: sending a prompt adds it to the conversation first, so by the time
 * the request is built nothing looks fresh any more. Asking that question at
 * send time is how Auto once sent "builder" for every session it was on for.
 */
export function modeToSend(mode: SessionMode, armed: boolean, jev: JevStatus | undefined): ModeChoice {
  return armed && routerOn(jev) ? "auto" : mode;
}
