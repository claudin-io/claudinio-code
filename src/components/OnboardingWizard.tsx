import { For, Show, createSignal, type Component } from "solid-js";
import { Icon } from "./Icon";

export const OnboardingWizard: Component<{
  onSignIn: () => Promise<void>;
  signingIn: boolean;
  signInError: string | null;
  onApiKeySubmit: (key: string) => Promise<void>;
  apiKeyValidating: boolean;
  apiKeyError: string | null;
}> = (props) => {
  const [step, setStep] = createSignal(0);
  const [showApiKeyField, setShowApiKeyField] = createSignal(false);
  const [apiKeyInput, setApiKeyInput] = createSignal("");

  const features = () => [
    {
      icon: "thinking-face" as const,
      title: "Intelligent Agent",
      desc: "Chat with AI that plans, executes tools, and shows everything in real time on the timeline.",
    },
    {
      icon: "check-circle" as const,
      title: "Safe Approvals",
      desc: "Bash commands and file edits require your permission with Monaco Editor visual diff.",
    },
    {
      icon: "layers" as const,
      title: "Parallel Subagents",
      desc: "Up to 4 simultaneous agents for complex tasks, each with its own timeline.",
    },
    {
      icon: "search" as const,
      title: "Smart Indexing",
      desc: "Semantic search with CodeBERT that understands what your code does, not just names.",
    },
  ];

  const handleApiKeyContinue = async () => {
    const key = apiKeyInput().trim();
    if (!key) return;
    await props.onApiKeySubmit(key);
  };

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
          {"Welcome to Claudinio Code"}
        </h2>
        <p class="mb-2 max-w-sm text-center text-sm text-ink-muted">
          {"Your AI agent for software development. Plan, code, and execute tasks with autonomous agents."}
        </p>
        <p class="text-xs text-ink-faint">{"Maximum productivity with AI that understands your code."}</p>
      </Show>

      {/* Step 1: Features */}
      <Show when={step() === 1}>
        <h2 class="mb-4 text-lg font-semibold text-ink">
          {"What you can do"}
        </h2>
        <div class="grid max-w-md grid-cols-2 gap-3">
          <For each={features()}>
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
          {"Let's get started"}
        </h2>
        <p class="mb-6 max-w-sm text-center text-sm text-ink-muted">
          {"Sign in with your claudin.io account to unlock all features."}
        </p>

        <Show when={!showApiKeyField()} fallback={
          <div class="flex w-full max-w-xs flex-col items-center gap-3">
            <input
              type="password"
              value={apiKeyInput()}
              onInput={(e) => setApiKeyInput(e.currentTarget.value)}
              placeholder={"Paste your API key"}
              class="w-full rounded-md border border-border-subtle bg-surface-0 p-2.5 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
            <button
              onClick={handleApiKeyContinue}
              disabled={props.apiKeyValidating || !apiKeyInput().trim()}
              class="inline-flex items-center gap-2 rounded-md bg-accent px-6 py-2.5 text-sm font-medium text-accent-ink hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Show when={props.apiKeyValidating} fallback={"Continue"}>
                <span>{"Validating…"}</span>
                <Icon name="loader" class="h-4 w-4 animate-spin" />
              </Show>
            </button>
            <Show when={props.apiKeyError}>
              <div class="text-sm text-red-400">{props.apiKeyError}</div>
            </Show>
            <button
              onClick={() => { setShowApiKeyField(false); setApiKeyInput(""); }}
              class="text-xs text-ink-muted hover:text-ink hover:underline"
            >
              {"← Back to sign in"}
            </button>
          </div>
        }>
          <button
            onClick={props.onSignIn}
            disabled={props.signingIn}
            class="inline-flex items-center gap-2 rounded-md bg-accent px-6 py-2.5 text-sm font-medium text-accent-ink hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Icon name="external-link" class="h-4 w-4" />
            <span>{"Sign in with claudin.io"}</span>
          </button>
          <Show when={props.signingIn}>
            <div class="mt-4 text-sm text-ink-muted">
              {"Waiting for browser sign-in…"}
            </div>
          </Show>
          <Show when={props.signInError}>
            <div class="mt-4 text-sm text-red-400">{props.signInError}</div>
          </Show>
          <button
            onClick={() => setShowApiKeyField(true)}
            class="mt-3 text-xs text-ink-muted hover:text-ink hover:underline"
          >
            {"Use API Key instead"}
          </button>
        </Show>
      </Show>

      {/* Navigation buttons */}
      <div class="mt-6 flex items-center gap-3">
        <Show when={step() === 0}>
          <button
            onClick={() => setStep(1)}
            class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-2 px-4 py-1.5 text-sm text-ink hover:bg-surface-3"
          >
            <span>{"Next"}</span>
            <Icon name="chevron-right" class="h-4 w-4" />
          </button>
        </Show>

        <Show when={step() === 1}>
          <button
            onClick={() => setStep(0)}
            class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-2 px-4 py-1.5 text-sm text-ink hover:bg-surface-3"
          >
            <Icon name="chevron-left" class="h-4 w-4" />
            <span>{"Previous"}</span>
          </button>
          <button
            onClick={() => setStep(2)}
            class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-2 px-4 py-1.5 text-sm text-ink hover:bg-surface-3"
          >
            <span>{"Next"}</span>
            <Icon name="chevron-right" class="h-4 w-4" />
          </button>
        </Show>

        <Show when={step() === 2}>
          <button
            onClick={() => setStep(1)}
            class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-2 px-4 py-1.5 text-sm text-ink hover:bg-surface-3"
          >
            <Icon name="chevron-left" class="h-4 w-4" />
            <span>{"Previous"}</span>
          </button>
        </Show>
      </div>
    </div>
  );
};
