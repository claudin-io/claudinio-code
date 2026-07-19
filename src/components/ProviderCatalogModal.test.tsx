import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import type { CatalogProvider, ConnectedProviderInfo } from "../lib/ipc";

const __test = vi.hoisted(() => ({
  mockFetchProviderCatalog: vi.fn(),
  mockConnectProvider: vi.fn(),
  mockDisconnectProvider: vi.fn(),
  mockOpenExternalUrl: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  fetchProviderCatalog: __test.mockFetchProviderCatalog,
  connectProvider: __test.mockConnectProvider,
  disconnectProvider: __test.mockDisconnectProvider,
  openExternalUrl: __test.mockOpenExternalUrl,
}));

vi.mock("../lib/grill-me", () => ({
  t: (key: string, ...args: string[]) => (args.length ? `${key}:${args.join(",")}` : key),
}));

vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class ?? ""} />
  ),
}));

import { ProviderCatalogModal } from "./ProviderCatalogModal";

function flush() {
  return new Promise((r) => setTimeout(r, 10));
}

const CATALOG: CatalogProvider[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    api: "https://api.deepseek.com",
    env: ["DEEPSEEK_API_KEY"],
    doc: "https://platform.deepseek.com/docs",
    protocol: "openai",
    models: [
      { id: "deepseek-chat", name: "DeepSeek Chat", costInput: 0.27, costOutput: 1.1, context: 65536 },
    ],
  },
  {
    id: "groq",
    name: "Groq",
    api: "https://api.groq.com/openai/v1",
    env: ["GROQ_API_KEY"],
    protocol: "openai",
    models: [],
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    api: "https://openrouter.ai/api/v1",
    env: ["OPENROUTER_API_KEY"],
    protocol: "openai",
    models: [],
  },
];

describe("ProviderCatalogModal", () => {
  let dispose: (() => void) | undefined;
  let container: HTMLDivElement;
  const onClose = vi.fn();
  const onChanged = vi.fn();

  function mount(providers: Record<string, ConnectedProviderInfo> = {}) {
    container = document.createElement("div");
    document.body.appendChild(container);
    const [prov] = createSignal(providers);
    dispose = render(
      () => <ProviderCatalogModal providers={prov} onClose={onClose} onChanged={onChanged} />,
      container,
    );
  }

  beforeEach(() => {
    vi.clearAllMocks();
    __test.mockFetchProviderCatalog.mockResolvedValue({ providers: CATALOG });
  });

  afterEach(() => {
    dispose?.();
    container?.remove();
  });

  it("lists catalog providers but hides OpenRouter (it has its own OAuth card)", async () => {
    mount();
    await flush();
    const text = container.textContent ?? "";
    expect(text).toContain("DeepSeek");
    expect(text).toContain("Groq");
    expect(text).not.toContain("OpenRouter");
  });

  it("filters providers by search", async () => {
    mount();
    await flush();
    const search = container.querySelector<HTMLInputElement>("input[type=text]")!;
    search.value = "deep";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();
    expect(container.textContent).toContain("DeepSeek");
    expect(container.textContent).not.toContain("Groq");
  });

  it("selecting a provider prefills the base URL and shows the env hint", async () => {
    mount();
    await flush();
    Array.from(container.querySelectorAll("button"))
      .find((b) => b.textContent?.includes("DeepSeek"))!
      .click();
    await flush();
    const urlInput = Array.from(container.querySelectorAll<HTMLInputElement>("input[type=text]")).find(
      (i) => i.value.includes("deepseek"),
    );
    expect(urlInput?.value).toBe("https://api.deepseek.com");
    expect(container.textContent).toContain("providerCatalog.envHint:DEEPSEEK_API_KEY");
  });

  it("connect success reports the model count and refreshes the app", async () => {
    __test.mockConnectProvider.mockResolvedValue(["deepseek-chat", "deepseek-reasoner"]);
    mount();
    await flush();
    Array.from(container.querySelectorAll("button"))
      .find((b) => b.textContent?.includes("DeepSeek"))!
      .click();
    await flush();
    const keyInput = container.querySelector<HTMLInputElement>("input[type=password]")!;
    keyInput.value = "sk-test";
    keyInput.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();
    Array.from(container.querySelectorAll("button"))
      .find((b) => b.textContent === "settings.providers.connect")!
      .click();
    await flush();
    expect(__test.mockConnectProvider).toHaveBeenCalledWith(
      "deepseek",
      "sk-test",
      "https://api.deepseek.com",
    );
    expect(container.textContent).toContain("providerCatalog.connectSuccess:2");
    expect(onChanged).toHaveBeenCalled();
  });

  it("connect failure shows the error inline", async () => {
    __test.mockConnectProvider.mockRejectedValue("Authentication failed — check your API key");
    mount();
    await flush();
    Array.from(container.querySelectorAll("button"))
      .find((b) => b.textContent?.includes("DeepSeek"))!
      .click();
    await flush();
    const keyInput = container.querySelector<HTMLInputElement>("input[type=password]")!;
    keyInput.value = "sk-bad";
    keyInput.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();
    Array.from(container.querySelectorAll("button"))
      .find((b) => b.textContent === "settings.providers.connect")!
      .click();
    await flush();
    expect(container.textContent).toContain("providerCatalog.connectError");
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("connected providers show a disconnect button that calls through", async () => {
    __test.mockDisconnectProvider.mockResolvedValue(undefined);
    mount({ deepseek: { connected: true, baseUrl: "https://api.deepseek.com" } });
    await flush();
    Array.from(container.querySelectorAll("button"))
      .find((b) => b.textContent?.includes("DeepSeek"))!
      .click();
    await flush();
    Array.from(container.querySelectorAll("button"))
      .find((b) => b.textContent === "settings.providers.disconnect")!
      .click();
    await flush();
    expect(__test.mockDisconnectProvider).toHaveBeenCalledWith("deepseek");
    expect(onChanged).toHaveBeenCalled();
  });

  it("catalog load failure offers a forced retry", async () => {
    __test.mockFetchProviderCatalog.mockRejectedValueOnce("network down");
    __test.mockFetchProviderCatalog.mockResolvedValueOnce({ providers: CATALOG });
    mount();
    await flush();
    expect(container.textContent).toContain("providerCatalog.loadError");
    Array.from(container.querySelectorAll("button"))
      .find((b) => b.textContent === "providerCatalog.retry")!
      .click();
    await flush();
    expect(__test.mockFetchProviderCatalog).toHaveBeenLastCalledWith(true);
    expect(container.textContent).toContain("DeepSeek");
  });
});
