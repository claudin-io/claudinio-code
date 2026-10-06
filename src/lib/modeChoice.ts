import type { JevStatus, SessionMode } from "./ipc";

/** What the mode control can send with a prompt. "auto" is not a mode a
 *  session is ever in: it hands the choice of starting phase to the harness,
 *  which decides once, on the first prompt (see `agent::route` in the backend). */
export type ModeChoice = SessionMode | "auto";

/** Auto is offered only where it can mean something: the router is switched
 *  on, Jev is enabled, and the session has not had its first prompt yet. Past
 *  that point the session is in a real mode and the control shows it. */
export function autoAvailable(jev: JevStatus | undefined, sessionIsFresh: boolean): boolean {
  return sessionIsFresh && jev?.enabled !== false && jev?.route === "on";
}

export function modeToSend(mode: SessionMode, auto: boolean): ModeChoice {
  return auto ? "auto" : mode;
}
