import { For, Show, type Component } from "solid-js";
import { hunkHeader, type Diff, type DiffLine } from "./diff";

/// A diff, drawn for a phone held in one hand.
///
/// §8 phase 3 ends with "a diff is legible and approvable on a phone held in one
/// hand", and §7 of the threat model rests on a human reading one before approving.
/// So the decisions here are about whether the change can be *read*, not about how it
/// looks:
///
/// - **Unified, never side by side.** Two columns on a 390 px screen gives each about
///   twenty characters, and code wraps into unreadable ribbons. One column is the only
///   layout that survives the width.
/// - **A sign as well as a colour.** `+` and `-` are in the text, so the diff still
///   reads in bright sunlight, on a cheap panel, and to someone who cannot separate
///   red from green. Colour alone carrying the meaning would make the gate unusable
///   for about one man in twelve.
/// - **Line numbers, and they scroll away.** The numbers are how the change is found
///   in the file afterwards, but they are not what is being read, so they do not
///   compete for the width.
/// - **Truncation is stated.** A silent cut is how someone approves the part they
///   could see.
///
/// Styling comes from `diff.css`, which the consumer imports. No inline styles: the
/// web peer's CSP has no `'unsafe-inline'`, so a `style` attribute would be dropped and
/// the diff would render unformatted — legible-ish on a desktop, useless on a phone.

interface DiffViewProps {
  diff: Diff;
  /// Shown above the change, when there is one. A diff with no path is a diff whose
  /// subject the reader has to infer.
  path?: string;
}

export const DiffView: Component<DiffViewProps> = (props) => {
  const empty = () => props.diff.hunks.length === 0;

  return (
    <div class="cdiff">
      <Show when={props.path}>
        {(path) => <div class="cdiff-path">{path()}</div>}
      </Show>

      <div class="cdiff-counts">
        <span class="cdiff-added">{`+${props.diff.added}`}</span>
        <span class="cdiff-removed">{`−${props.diff.removed}`}</span>
        <Show when={props.diff.wholeBlock}>
          {/* Said plainly rather than dressed up. A whole-block replacement is not a
              minimal diff, and letting it look like one would overstate the change. */}
          <span class="cdiff-note">{"shown as a full replacement — too large to diff"}</span>
        </Show>
      </div>

      <Show when={empty()}>
        <div class="cdiff-note">{"No change."}</div>
      </Show>

      <For each={props.diff.hunks}>
        {(hunk) => (
          <div class="cdiff-hunk">
            <div class="cdiff-header">{hunkHeader(hunk)}</div>
            <For each={hunk.lines}>{(line) => <Line line={line} />}</For>
          </div>
        )}
      </For>

      <Show when={props.diff.truncated}>
        {(cut) => (
          <div class="cdiff-truncated">
            {`${cut().lines} more line${cut().lines === 1 ? "" : "s"}`}
            {cut().hunks > 0
              ? ` in ${cut().hunks} more hunk${cut().hunks === 1 ? "" : "s"} not shown.`
              : " not shown."}
          </div>
        )}
      </Show>
    </div>
  );
};

const Line: Component<{ line: DiffLine }> = (props) => {
  const sign = () =>
    props.line.kind === "added" ? "+" : props.line.kind === "removed" ? "-" : " ";

  return (
    <div
      class="cdiff-line"
      classList={{
        "cdiff-line-added": props.line.kind === "added",
        "cdiff-line-removed": props.line.kind === "removed",
      }}
    >
      <span class="cdiff-num" aria-hidden="true">
        {props.line.oldNumber ?? ""}
      </span>
      <span class="cdiff-num" aria-hidden="true">
        {props.line.newNumber ?? ""}
      </span>
      {/* The sign is inside the text, not a decoration beside it, so copying a line
          copies the diff rather than a stripped version of it. */}
      <span class="cdiff-text">{`${sign()}${props.line.text}`}</span>
    </div>
  );
};
