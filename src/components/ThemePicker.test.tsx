import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import ThemePicker from "./ThemePicker";

vi.mock("../lib/theme", () => ({
  preference: vi.fn(() => "system"),
  resolvedTheme: vi.fn(() => "claudinio"),
  setThemePreference: vi.fn(),
  themeMetadata: {
    claudinio: {
      label: "Claudinio",
      category: "dark",
      previewColors: [
        "oklch(0.145 0.015 280)",
        "oklch(0.185 0.018 280)",
        "oklch(0.95 0.01 280)",
        "oklch(0.62 0.19 277)",
        "oklch(0.72 0.17 155)",
      ],
    },
    dracula: {
      label: "Dracula",
      category: "dark",
      previewColors: [
        "oklch(0.14 0.02 325)",
        "oklch(0.19 0.03 325)",
        "oklch(0.94 0.02 325)",
        "oklch(0.72 0.15 335)",
        "oklch(0.65 0.15 145)",
      ],
    },
    "claudinio-sepia": {
      label: "theme.claudinio-sepia",
      category: "light",
      previewColors: [
        "oklch(0.96 0.015 90)",
        "oklch(0.87 0.025 88)",
        "oklch(0.22 0.03 80)",
        "oklch(0.55 0.16 65)",
        "oklch(0.58 0.17 145)",
      ],
    },
  },
  ALL_THEMES: ["claudinio", "dracula", "claudinio-sepia"],
}));


vi.mock("./Icon", () => ({
  Icon: () => null,
}));

describe("ThemePicker", () => {
  let dispose: () => void;

  afterEach(() => {
    dispose?.();
    vi.clearAllMocks();
  });

  function mount() {
    const container = document.createElement("div");
    document.body.appendChild(container);
    dispose = render(() => <ThemePicker />, container);
    return container;
  }

  it("renders System card as the first child", () => {
    const container = mount();
    const buttons = container.querySelectorAll("button");
    expect(buttons.length).toBe(4); // system + 3 themes
    expect(buttons[0].textContent).toContain("System");
  });

  it("renders all theme cards with their labels", () => {
    const container = mount();
    const text = container.textContent ?? "";
    expect(text).toContain("Claudinio");
    expect(text).toContain("Dracula");
    expect(text).toContain("theme.claudinio-sepia");
  });

  it("shows resolved theme badge on System card", () => {
    const container = mount();
    const systemCard = container.querySelector("button")!;
    expect(systemCard.textContent).toContain("Claudinio");
  });

  it("calls setThemePreference when a theme card is clicked", async () => {
    const { setThemePreference } = await import("../lib/theme");
    mount();
    const buttons = document.body.querySelectorAll("button");
    // Click the Dracula card (index 2: system=0, claudinio=1, dracula=2)
    (buttons[2] as HTMLButtonElement).click();
    expect(setThemePreference).toHaveBeenCalledWith("dracula");
  });

  it("calls setThemePreference with 'system' when System card is clicked", async () => {
    const { setThemePreference } = await import("../lib/theme");
    mount();
    const buttons = document.body.querySelectorAll("button");
    (buttons[0] as HTMLButtonElement).click();
    expect(setThemePreference).toHaveBeenCalledWith("system");
  });

  it("renders with grid-cols-4 layout", () => {
    const container = mount();
    const grid = container.firstElementChild;
    expect(grid?.classList.contains("grid-cols-4")).toBe(true);
  });
});