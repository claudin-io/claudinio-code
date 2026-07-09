import { For, Show, createSignal, type Component } from "solid-js";
import { Icon } from "./Icon";
import { t } from "../lib/grill-me";

export const OnboardingWizard: Component<{
  onSignIn: () => Promise<void>;
  signingIn: boolean;
  signInError: string | null;
}> = (props) => {
  const [step, setStep] = createSignal(0);

  const features = [
    {
      icon: "thinking-face" as const,
      title: t("onboarding.features.agent.title"),
      desc: t("onboarding.features.agent.desc"),
    },
    {
      icon: "check-circle" as const,
      title: t("onboarding.features.approval.title"),
      desc: t("onboarding.features.approval.desc"),
    },
    {
      icon: "layers" as const,
      title: t("onboarding.features.subagents.title"),
      desc: t("onboarding.features.subagents.desc"),
    },
    {
      icon: "search" as const,
      title: t("onboarding.features.indexing.title"),
      desc: t("onboarding.features.indexing.desc"),
    },
  ];

  return (
    <div class="flex h-full flex-col items-center justify-center px-6">
      {/* Dots indicator */}
      <div class="mb-6 flex gap-2">
        <For each={[0, 1, 2]}>
          {(i) => (
            <button
              onClick={() => setStep(i)}
              class={`h-2 w-2 rounded-full ${step() === i ? "bg-accent" : "border-2 border-border-subtle"}`}
            />
          )}
        </For>
      </div>

      {/* Step 0: Welcome */}
      <Show when={step() === 0}>
        <img src="/reddit_icon_256.png" class="mb-4 h-20 w-20" />
        <h2 class="mb-2 text-lg font-semibold text-ink">
          {t("onboarding.welcome.title")}
        </h2>
        <p class="mb-2 max-w-sm text-center text-sm text-ink-muted">
          {t("onboarding.welcome.subtitle")}
        </p>
        <p class="text-xs text-ink-faint">{t("onboarding.welcome.tagline")}</p>
      </Show>

      {/* Step 1: Features */}
      <Show when={step() === 1}>
        <h2 class="mb-4 text-lg font-semibold text-ink">
          {t("onboarding.features.title")}
        </h2>
        <div class="grid max-w-md grid-cols-2 gap-3">
          <For each={features}>
            {(feature) => (
              <div class="flex flex-col items-center gap-2 rounded-lg border border-border-subtle bg-surface-0 p-4 text-center">
                <Icon name={feature.icon} class="h-8 w-8 text-accent" />
                <strong class="text-xs font-semibold text-ink">{feature.title}</strong>
                <span class="text-[11px] text-ink-muted">{feature.desc}</span>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* Step 2: Sign In */}
      <Show when={step() === 2}>
        <Icon name="goal" class="mb-4 h-16 w-16 text-accent" />
        <h2 class="mb-2 text-lg font-semibold text-ink">
          {t("onboarding.signIn.title")}
        </h2>
        <p class="mb-6 max-w-sm text-center text-sm text-ink-muted">
          {t("onboarding.signIn.subtitle")}
        </p>
        <button
          onClick={props.onSignIn}
          disabled={props.signingIn}
          class="inline-flex items-center gap-2 rounded-md bg-accent px-6 py-2.5 text-sm font-medium text-accent-ink hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
        >
          <Icon name="external-link" class="h-4 w-4" />
          <span>{t("onboarding.signIn.button")}</span>
        </button>
        <Show when={props.signingIn}>
          <div class="mt-4 text-sm text-ink-muted">
            {t("onboarding.signIn.signingIn")}
          </div>
        </Show>
        <Show when={props.signInError}>
          <div class="mt-4 text-sm text-red-400">{props.signInError}</div>
        </Show>
      </Show>

      {/* Navigation buttons */}
      <div class="mt-6 flex items-center gap-3">
        <Show when={step() === 0}>
          <button
            onClick={() => setStep(1)}
            class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-2 px-4 py-1.5 text-sm text-ink hover:bg-surface-3"
          >
            <span>{t("onboarding.next")}</span>
            <Icon name="chevron-right" class="h-4 w-4" />
          </button>
        </Show>

        <Show when={step() === 1}>
          <button
            onClick={() => setStep(0)}
            class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-2 px-4 py-1.5 text-sm text-ink hover:bg-surface-3"
          >
            <Icon name="chevron-left" class="h-4 w-4" />
            <span>{t("onboarding.prev")}</span>
          </button>
          <button
            onClick={() => setStep(2)}
            class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-2 px-4 py-1.5 text-sm text-ink hover:bg-surface-3"
          >
            <span>{t("onboarding.next")}</span>
            <Icon name="chevron-right" class="h-4 w-4" />
          </button>
        </Show>

        <Show when={step() === 2}>
          <button
            onClick={() => setStep(1)}
            class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-2 px-4 py-1.5 text-sm text-ink hover:bg-surface-3"
          >
            <Icon name="chevron-left" class="h-4 w-4" />
            <span>{t("onboarding.prev")}</span>
          </button>
        </Show>
      </div>
    </div>
  );
};
