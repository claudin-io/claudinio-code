import { createSignal, For, Show, type Component } from "solid-js";
import { type AskUserData, type UserAnswer } from "../lib/ipc";
import { t } from "../lib/grill-me";
import { Icon } from "./Icon";

interface QuestionDraft {
  picks: number[];
  otherSelected: boolean;
  otherText: string;
}

const QuestionCard: Component<{
  ask: AskUserData;
  onSubmit: (answers: UserAnswer[]) => void;
}> = (props) => {
  const [drafts, setDrafts] = createSignal<QuestionDraft[]>(
    props.ask.questions.map(() => ({ picks: [], otherSelected: false, otherText: "" })),
  );

  const updateDraft = (qi: number, patch: Partial<QuestionDraft>) => {
    setDrafts((prev) => prev.map((d, i) => (i === qi ? { ...d, ...patch } : d)));
  };

  const pickOption = (qi: number, oi: number, multi: boolean) => {
    const d = drafts()[qi];
    if (multi) {
      const picks = d.picks.includes(oi) ? d.picks.filter((p) => p !== oi) : [...d.picks, oi];
      updateDraft(qi, { picks });
    } else {
      updateDraft(qi, { picks: [oi], otherSelected: false });
    }
  };

  const pickOther = (qi: number, multi: boolean) => {
    const d = drafts()[qi];
    if (multi) {
      updateDraft(qi, { otherSelected: !d.otherSelected });
    } else {
      updateDraft(qi, { picks: [], otherSelected: true });
    }
  };

  const answered = (d: QuestionDraft) =>
    d.picks.length > 0 || (d.otherSelected && d.otherText.trim().length > 0);

  const allAnswered = () => drafts().every(answered);

  const submit = () => {
    if (!allAnswered()) return;
    const answers: UserAnswer[] = props.ask.questions.map((q, qi) => {
      const d = drafts()[qi];
      const parts = d.picks.map((oi) => q.options[oi].label);
      if (d.otherSelected && d.otherText.trim()) parts.push(d.otherText.trim());
      return { question: q.question, answer: parts.join(", ") };
    });
    props.onSubmit(answers);
  };

  return (
    <div class="rounded-lg border border-accent/50 bg-surface-1 p-3">
      <div class="mb-3 flex items-center gap-2">
        <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">
          {t("chat.question.needsAnswer")}
        </span>
      </div>

      <For each={props.ask.questions}>
        {(q, qi) => {
          const multi = () => q.multi_select === true;
          const draft = () => drafts()[qi()];
          return (
            <div class="mb-4 last:mb-3">
              <p class="mb-2 text-[13px] font-medium leading-[1.5] text-ink">{q.question}</p>
              <div class="flex flex-col gap-1">
                <For each={q.options}>
                  {(opt, oi) => (
                    <button
                      onClick={() => pickOption(qi(), oi(), multi())}
                      class={`flex items-start gap-2 rounded-md border px-3 py-1.5 text-left text-[13px] transition-colors ${
                        draft().picks.includes(oi())
                          ? "border-accent bg-accent/10 text-ink"
                          : "border-border-subtle bg-surface-0 text-ink-muted hover:border-accent/40"
                      }`}
                    >
                      <span
                        class={`mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center border ${
                          multi() ? "rounded-sm" : "rounded-full"
                        } ${draft().picks.includes(oi()) ? "border-accent bg-accent" : "border-ink-faint"}`}
                      >
                        <Show when={draft().picks.includes(oi())}>
                          <Icon name="check" class="h-2.5 w-2.5 text-accent-ink" />
                        </Show>
                      </span>
                      <span class="flex min-w-0 flex-col gap-0.5">
                        <span>{opt.label}</span>
                        <Show when={opt.description}>
                          <span class="text-[11px] leading-[1.4] text-ink-faint">
                            {opt.description}
                          </span>
                        </Show>
                      </span>
                    </button>
                  )}
                </For>

                <button
                  onClick={() => pickOther(qi(), multi())}
                  class={`flex items-center gap-2 rounded-md border px-3 py-1.5 text-left text-[13px] transition-colors ${
                    draft().otherSelected
                      ? "border-accent bg-accent/10 text-ink"
                      : "border-border-subtle bg-surface-0 text-ink-muted hover:border-accent/40"
                  }`}
                >
                  <span
                    class={`flex h-3.5 w-3.5 shrink-0 items-center justify-center border ${
                      multi() ? "rounded-sm" : "rounded-full"
                    } ${draft().otherSelected ? "border-accent bg-accent" : "border-ink-faint"}`}
                  >
                    <Show when={draft().otherSelected}>
                      <Icon name="check" class="h-2.5 w-2.5 text-accent-ink" />
                    </Show>
                  </span>
                  {t("chat.question.other")}
                </button>

                <Show when={draft().otherSelected}>
                  <input
                    type="text"
                    value={draft().otherText}
                    onInput={(e) => updateDraft(qi(), { otherText: e.currentTarget.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") submit();
                    }}
                    placeholder={t("chat.question.typeAnswer")}
                    class="mt-1 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-[13px] text-ink placeholder:text-ink-faint focus:border-accent/60 focus:outline-none"
                  />
                </Show>
              </div>
            </div>
          );
        }}
      </For>

      <button
        onClick={submit}
        disabled={!allAnswered()}
        class="flex w-full items-center justify-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-30"
      >
        <Icon name="send" class="h-4 w-4" />
        {t("chat.question.submit")}
      </button>
    </div>
  );
};

export default QuestionCard;
