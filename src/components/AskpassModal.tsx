import { createSignal, Show, type Component } from "solid-js";
import { Portal } from "solid-js/web";
import { t } from "../lib/grill-me";
import { Icon } from "./Icon";
import { answerAskpass, type AskpassRequest } from "../lib/ipc";

/**
 * Password modal for git/ssh credential prompts intercepted by the backend
 * askpass bridge (askpass.rs). The prompt text comes straight from ssh/git
 * (e.g. "Enter passphrase for key '~/.ssh/id_ed25519':").
 */
export const AskpassModal: Component<{
  request: AskpassRequest | null;
  onDone: () => void;
}> = (props) => {
  const [secret, setSecret] = createSignal("");

  const finish = (value: string | null) => {
    const req = props.request;
    if (req) void answerAskpass(req.id, value);
    setSecret("");
    props.onDone();
  };

  return (
    <Show when={props.request}>
      {(req) => (
        <Portal>
          <div class="fixed inset-0 z-[100] flex items-center justify-center bg-black/40">
            <div class="w-96 rounded-lg border border-border-subtle bg-surface-1 p-4 shadow-modal">
              <div class="mb-2 flex items-center gap-2">
                <Icon name="alert-circle" class="h-4 w-4 text-accent" />
                <span class="text-sm font-medium text-ink">{t("askpass.title")}</span>
              </div>
              <p class="mb-1 break-words font-mono text-[12px] text-ink-muted">{req().prompt}</p>
              <p class="mb-3 text-[11px] text-ink-faint">{t("askpass.hint")}</p>
              <input
                type="password"
                autofocus
                value={secret()}
                onInput={(e) => setSecret(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") finish(secret());
                  if (e.key === "Escape") finish(null);
                }}
                class="mb-3 w-full rounded border border-border-subtle bg-surface-2 px-2 py-1.5 text-sm text-ink outline-none focus:border-accent"
                placeholder={t("askpass.placeholder")}
              />
              <div class="flex justify-end gap-2">
                <button
                  onClick={() => finish(null)}
                  class="rounded px-3 py-1.5 text-[12px] text-ink-muted hover:bg-surface-2"
                >
                  {t("askpass.cancel")}
                </button>
                <button
                  onClick={() => finish(secret())}
                  class="rounded bg-accent px-3 py-1.5 text-[12px] font-medium text-white hover:opacity-90"
                >
                  {t("askpass.submit")}
                </button>
              </div>
            </div>
          </div>
        </Portal>
      )}
    </Show>
  );
};
