import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import type { ConnectedProviderInfo } from "../lib/ipc";

const __test = vi.hoisted(() => ({
  mockSaveCustomProvider: vi.fn(),
  mockProbeCustomProvider: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  saveCustomProvider: __test.mockSaveCustomProvider,
  probeCustomProvider: __test.mockProbeCustomProvider,
}));

vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class ?? ""} />
  ),
}));

import {
  CustomProviderModal,
  parseContextWindow,
  parseModelList,
  readContextWindow,
  requestUrlPreview,
} from "./CustomProviderModal";

function flush() {
  return new Promise((r) => setTimeout(r, 10));
}

const LITELLM: ConnectedProviderInfo = {
  connected: true,
  baseUrl: "http://localhost:4000/v1",
  label: "LiteLLM",
  protocol: "anthropic",
  custom: true,
  models: ["claude-sonnet", "gpt-4o"],
  hasApiKey: true,
};

describe("parseModelList", () => {
  it("splits on newlines and commas, trimming and dropping duplicates", () => {
    expect(parseModelList(" gpt-4o \nclaude-sonnet, gpt-4o,,\n\n qwen3")).toEqual([
      "gpt-4o",
      "claude-sonnet",
      "qwen3",
    ]);
    expect(parseModelList("  \n ")).toEqual([]);
  });
});

describe("parseContextWindow", () => {
  it("reads plain token counts and the k shorthand", () => {
    expect(parseContextWindow("32768")).toBe(32768);
    expect(parseContextWindow(" 128k ")).toBe(128000);
    expect(parseContextWindow("32 K")).toBe(32000);
    expect(parseContextWindow("65,536")).toBe(65536);
  });

  it("is null for blank, zero and anything that is not a number", () => {
    expect(parseContextWindow("")).toBeNull();
    expect(parseContextWindow("0")).toBeNull();
    expect(parseContextWindow("big")).toBeNull();
    expect(parseContextWindow("-4096")).toBeNull();
  });

  // "32.768" is how most of the world writes thirty-two thousand. Read as a
  // decimal it was saved as 33 tokens, and every request was then refused.
  it("reads digit groups with either separator as the same number", () => {
    expect(parseContextWindow("32.768")).toBe(32768);
    expect(parseContextWindow("32,768")).toBe(32768);
    expect(parseContextWindow("128.000")).toBe(128000);
    expect(parseContextWindow("1.048.576")).toBe(1048576);
    expect(parseContextWindow("1_048_576")).toBe(1048576);
  });

  it("reads a decimal before the k with either separator", () => {
    expect(parseContextWindow("32.5k")).toBe(32500);
    expect(parseContextWindow("32,5k")).toBe(32500);
    expect(parseContextWindow("8k")).toBe(8000);
  });
});

describe("readContextWindow", () => {
  it("tells a blank field from one it cannot read", () => {
    expect(readContextWindow("  ")).toEqual({ tokens: null });
    for (const typo of ["32k tokens", "32kb", "2^15", "1e5", "big", "-4096", "32.76", "3,2768"]) {
      expect(readContextWindow(typo), typo).toHaveProperty("error");
    }
  });

  it("refuses a window too small to be one, and says what was probably meant", () => {
    const read = readContextWindow("32");
    expect(read).toHaveProperty("error");
    expect((read as { error: string }).error).toContain("32k");
    expect(readContextWindow("0")).toHaveProperty("error");
    expect(readContextWindow("1024")).toEqual({ tokens: 1024 });
  });

  it("refuses a number no model has", () => {
    expect(readContextWindow("99999999999")).toHaveProperty("error");
  });
});

describe("requestUrlPreview", () => {
  it("appends the chat route to an OpenAI base as given", () => {
    expect(requestUrlPreview("http://localhost:11434/v1/", "openai")).toBe(
      "http://localhost:11434/v1/chat/completions",
    );
  });

  it("lands on the same Anthropic route with or without /v1", () => {
    expect(requestUrlPreview("http://localhost:4000/v1", "anthropic")).toBe("http://localhost:4000/v1/messages");
    expect(requestUrlPreview("http://localhost:4000", "anthropic")).toBe("http://localhost:4000/v1/messages");
  });

  it("is empty until there is a URL", () => {
    expect(requestUrlPreview("  ", "openai")).toBe("");
  });
});

describe("CustomProviderModal", () => {
  let dispose: (() => void) | undefined;
  let container: HTMLDivElement;
  const onClose = vi.fn();
  const onChanged = vi.fn();
  const onRemove = vi.fn();

  function mount(providerId: string | null = null, providers: Record<string, ConnectedProviderInfo> = {}) {
    container = document.createElement("div");
    document.body.appendChild(container);
    const [prov] = createSignal(providers);
    dispose = render(
      () => (
        <CustomProviderModal
          providerId={providerId}
          providers={prov}
          onClose={onClose}
          onChanged={onChanged}
          onRemove={onRemove}
        />
      ),
      container,
    );
  }

  function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
    el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }

  const field = (name: string) => container.querySelector<HTMLInputElement>(`input[data-field="${name}"]`)!;
  const keyInput = () => container.querySelector<HTMLInputElement>("input[type=password]")!;
  const modelsArea = () => container.querySelector<HTMLTextAreaElement>("textarea")!;
  const button = (label: string) =>
    Array.from(container.querySelectorAll("button")).find((b) => b.textContent === label)!;

  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    dispose?.();
    container?.remove();
  });

  it("cannot be saved before it has a name and a base URL", async () => {
    mount();
    expect(button("Add provider").disabled).toBe(true);
    type(field("name"), "LiteLLM");
    await flush();
    expect(button("Add provider").disabled).toBe(true);
    type(field("base-url"), "http://localhost:4000/v1");
    await flush();
    expect(button("Add provider").disabled).toBe(false);
  });

  it("saves a new OpenAI-compatible provider and closes", async () => {
    __test.mockSaveCustomProvider.mockResolvedValue({ providerId: "litellm", models: ["gpt-4o"] });
    mount();
    type(field("name"), " LiteLLM ");
    type(field("base-url"), "http://localhost:4000/v1");
    type(keyInput(), "sk-test");
    type(modelsArea(), "gpt-4o\nclaude-sonnet");
    await flush();
    button("Add provider").click();
    await flush();
    expect(__test.mockSaveCustomProvider).toHaveBeenCalledWith({
      providerId: null,
      name: "LiteLLM",
      baseUrl: "http://localhost:4000/v1",
      protocol: "openai",
      apiKey: "sk-test",
      models: ["gpt-4o", "claude-sonnet"],
      contextWindow: null,
    });
    expect(onChanged).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("sends no key and no models when both are left blank", async () => {
    __test.mockSaveCustomProvider.mockResolvedValue({ providerId: "ollama", models: ["llama3"] });
    mount();
    type(field("name"), "Ollama");
    type(field("base-url"), "http://localhost:11434/v1");
    await flush();
    button("Add provider").click();
    await flush();
    expect(__test.mockSaveCustomProvider).toHaveBeenCalledWith(
      expect.objectContaining({ apiKey: null, models: [] }),
    );
  });

  it("switching the API format changes the protocol and the request preview", async () => {
    __test.mockSaveCustomProvider.mockResolvedValue({ providerId: "gw", models: ["m"] });
    mount();
    type(field("name"), "Gateway");
    type(field("base-url"), "https://llm.corp.example");
    await flush();
    const preview = () => container.querySelector("[data-request-preview]")!.textContent;
    expect(preview()).toBe("Requests go to https://llm.corp.example/chat/completions");
    container.querySelector<HTMLButtonElement>('button[data-protocol="anthropic"]')!.click();
    await flush();
    expect(preview()).toBe("Requests go to https://llm.corp.example/v1/messages");
    button("Add provider").click();
    await flush();
    expect(__test.mockSaveCustomProvider).toHaveBeenCalledWith(
      expect.objectContaining({ protocol: "anthropic" }),
    );
  });

  it("a failed save stays open and shows the reason", async () => {
    __test.mockSaveCustomProvider.mockRejectedValue("Authentication failed — check your API key");
    mount();
    type(field("name"), "LiteLLM");
    type(field("base-url"), "http://localhost:4000/v1");
    await flush();
    button("Add provider").click();
    await flush();
    expect(container.textContent).toContain("Could not save: Authentication failed — check your API key");
    expect(onChanged).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("Fetch models fills the list from the endpoint", async () => {
    __test.mockProbeCustomProvider.mockResolvedValue(["llama3", "qwen3"]);
    mount();
    type(field("base-url"), "http://localhost:11434/v1");
    await flush();
    button("Fetch models").click();
    await flush();
    expect(__test.mockProbeCustomProvider).toHaveBeenCalledWith("http://localhost:11434/v1", "openai", "", null);
    expect(modelsArea().value).toBe("llama3\nqwen3");
    expect(container.textContent).toContain("Found 2 models.");
  });

  it("a failed fetch keeps what was typed and says why", async () => {
    __test.mockProbeCustomProvider.mockRejectedValue("could not list models from http://x/models (HTTP 404)");
    mount();
    type(field("base-url"), "http://x");
    type(modelsArea(), "my-model");
    await flush();
    button("Fetch models").click();
    await flush();
    expect(modelsArea().value).toBe("my-model");
    expect(container.textContent).toContain("Could not fetch models: could not list models from");
  });

  it("editing starts from the saved entry and keeps the key when left blank", async () => {
    __test.mockSaveCustomProvider.mockResolvedValue({ providerId: "litellm", models: ["claude-sonnet"] });
    mount("litellm", { litellm: LITELLM });
    expect(container.textContent).toContain("Edit custom provider");
    expect(field("name").value).toBe("LiteLLM");
    expect(field("base-url").value).toBe("http://localhost:4000/v1");
    expect(modelsArea().value).toBe("claude-sonnet\ngpt-4o");
    expect(keyInput().value).toBe("");
    expect(keyInput().placeholder).toBe("Leave blank to keep the saved key");
    expect(
      container.querySelector('button[data-protocol="anthropic"]')!.getAttribute("aria-pressed"),
    ).toBe("true");
    type(modelsArea(), "claude-sonnet");
    await flush();
    button("Save changes").click();
    await flush();
    expect(__test.mockSaveCustomProvider).toHaveBeenCalledWith({
      providerId: "litellm",
      name: "LiteLLM",
      baseUrl: "http://localhost:4000/v1",
      protocol: "anthropic",
      apiKey: null,
      models: ["claude-sonnet"],
      contextWindow: null,
    });
  });

  it("sends a typed context window, and reopens with the saved one", async () => {
    __test.mockSaveCustomProvider.mockResolvedValue({ providerId: "ollama", models: ["qwen3"] });
    mount();
    type(field("name"), "Ollama");
    type(field("base-url"), "http://localhost:11434/v1");
    type(field("context-window"), "32k");
    await flush();
    button("Add provider").click();
    await flush();
    expect(__test.mockSaveCustomProvider).toHaveBeenCalledWith(
      expect.objectContaining({ contextWindow: 32000 }),
    );
    dispose?.();
    container.remove();

    mount("litellm", { litellm: { ...LITELLM, contextWindow: 16384 } });
    expect(field("context-window").value).toBe("16384");
  });

  // A window the field cannot read used to be sent as "not set": the dialog
  // closed without a word and a saved window was cleared.
  it("does not save a context window it cannot read, and says why", async () => {
    mount("litellm", { litellm: { ...LITELLM, contextWindow: 16384 } });
    type(field("context-window"), "32k tokens");
    await flush();
    expect(container.querySelector("[data-context-error]")?.textContent).toContain("like 32768 or 32k");
    expect(button("Save changes").disabled).toBe(true);
    button("Save changes").click();
    await flush();
    expect(__test.mockSaveCustomProvider).not.toHaveBeenCalled();

    type(field("context-window"), "32.768");
    await flush();
    expect(container.querySelector("[data-context-error]")).toBeNull();
    button("Save changes").click();
    await flush();
    expect(__test.mockSaveCustomProvider).toHaveBeenCalledWith(
      expect.objectContaining({ contextWindow: 32768 }),
    );
  });

  it("fetching while editing passes the id so the saved key is reused", async () => {
    __test.mockProbeCustomProvider.mockResolvedValue(["claude-sonnet"]);
    mount("litellm", { litellm: LITELLM });
    button("Fetch models").click();
    await flush();
    expect(__test.mockProbeCustomProvider).toHaveBeenCalledWith(
      "http://localhost:4000/v1",
      "anthropic",
      "",
      "litellm",
    );
  });

  it("only an existing provider can be removed, and removing closes the form", async () => {
    mount();
    expect(button("Remove provider")).toBeUndefined();
    dispose?.();
    container.remove();

    mount("litellm", { litellm: LITELLM });
    button("Remove provider").click();
    await flush();
    expect(onRemove).toHaveBeenCalledWith("litellm");
    expect(onClose).toHaveBeenCalled();
  });

  it("Escape and the backdrop both close without saving", async () => {
    mount();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    (container.firstElementChild as HTMLElement).click();
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(__test.mockSaveCustomProvider).not.toHaveBeenCalled();
  });
});
