import { createSignal } from "solid-js";

/// A message another part of the UI wants to put in a workspace's composer.
///
/// It fills the box rather than sending: a click in Settings should not spend
/// the user's tokens, and a prompt worth running is a prompt worth reading
/// first. The user presses enter.
export interface ComposerSeed {
  workspace: string;
  text: string;
}

const [seed, setSeed] = createSignal<ComposerSeed | null>(null);

export function seedComposer(workspace: string, text: string): void {
  setSeed({ workspace, text });
}

/// Read and clear the pending seed, if it belongs to this workspace. Clearing
/// on read is what stops the same prompt reappearing every time the panel
/// re-renders or the user switches tabs back.
export function takeComposerSeed(workspace: string): string | null {
  const pending = seed();
  if (!pending || pending.workspace !== workspace) return null;
  setSeed(null);
  return pending.text;
}

/// Reactive handle, so a panel can watch for a seed without polling.
export { seed as pendingComposerSeed };

/// The prompt that asks the agent to give a project a test suite.
///
/// Written as a `<goal>` so it becomes a golden task: the harness then refuses
/// to let the run finish until the suite actually passes, rather than taking
/// the agent's word that it wrote some tests.
///
/// It must be ASCII — the backend rejects non-English user input before the
/// workflow starts.
export function bootstrapTestsPrompt(detected: string[]): string {
  const stacks =
    detected.length > 0
      ? `The harness already detected: ${detected.join(", ")}. Fit into what is there rather than introducing a second framework.`
      : "The harness detected no test runner at all, so part of this task is choosing and wiring one up. Pick whatever is idiomatic for this project's language and package manager, and add it as a dev dependency.";

  return [
    "<goal>this project has a working test suite that the quality harness can run</goal>",
    "",
    "Set up automated tests for this project.",
    "",
    stacks,
    "",
    "What matters:",
    "1. Start by reading the code to find the behaviour worth protecting - the logic with branches, edge cases and error paths. Do not write a test per file for the sake of coverage.",
    "2. Every test must ASSERT something specific. A test that calls a function and only checks it did not throw is worse than no test: it turns a red suite green without adding any protection.",
    "3. When you are done, call run_quality with layers [\"tests\", \"mutation\"]. Mutation deliberately breaks the code to check the tests notice - it is the check that tells us whether these tests are real. Surviving mutants mean the assertions are too weak, so strengthen them and run it again.",
    "4. Tell me what you chose to test and, just as importantly, what you deliberately left untested and why.",
  ].join("\n");
}
