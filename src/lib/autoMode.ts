import { createSignal, type Accessor } from "solid-js";
import type { JevStatus, SessionMode } from "./ipc";
import { autoAvailable, modeToSend, type ModeChoice } from "./modeChoice";

interface AutoModeDeps {
  /** The mode the control shows when Auto is not on. */
  mode: Accessor<SessionMode>;
  /** True until the conversation has its first prompt. */
  sessionIsFresh: Accessor<boolean>;
  /** Reads the router's settings; re-read whenever Auto is armed, so a change
   *  in Settings applies to the next conversation. */
  loadJev: () => Promise<JevStatus | undefined>;
}

/**
 * The state behind the "Auto" choice of the mode control: whether the choice
 * of starting phase is left to the harness for the next prompt.
 *
 * Auto is never a mode a session is in. It is armed when a conversation
 * starts, spent by the first prompt, and given up the moment the user picks a
 * mode by hand — including while the settings read that would arm it is still
 * on its way.
 */
export function createAutoMode(deps: AutoModeDeps) {
  const [jev, setJev] = createSignal<JevStatus | undefined>(undefined);
  const [armed, setArmed] = createSignal(false);
  // Every arm, hand pick and send moves this on; an answer from an older
  // settings read finds it changed and leaves the state alone.
  let generation = 0;

  const disarm = () => {
    generation++;
    setArmed(false);
  };

  return {
    /** Whether the control should offer Auto at all. */
    offered: () => autoAvailable(jev(), deps.sessionIsFresh()),
    /** Whether the control shows Auto as selected. Looks at the conversation
     *  too, so an old session reopened never shows "no mode". */
    active: () => armed() && autoAvailable(jev(), deps.sessionIsFresh()),
    /** A new conversation: Auto is on if the router is. */
    arm: async () => {
      const mine = ++generation;
      try {
        const status = await deps.loadJev();
        // What the router's settings are is true whoever clicked what in the
        // meantime; only whether Auto is selected was decided by that click.
        setJev(status);
        if (mine !== generation) return;
        setArmed(autoAvailable(status, deps.sessionIsFresh()));
      } catch {
        if (mine === generation) setArmed(false);
      }
    },
    /** The user picked Auto on the control. */
    select: () => {
      generation++;
      setArmed(true);
    },
    /** The user picked a mode, or opened a session that already has one. */
    disarm,
    /**
     * What to send with the next prompt, and Auto is spent. Safe to call
     * before or after the prompt is added to the conversation — it does not
     * look (see `modeToSend`).
     */
    take: (): ModeChoice => {
      const choice = modeToSend(deps.mode(), armed(), jev());
      disarm();
      return choice;
    },
  };
}
