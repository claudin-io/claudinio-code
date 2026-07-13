import { describe, it, expect, afterEach } from "vitest";
import { render } from "solid-js/web";
import { Icon, toolIcon, type IconName } from "./Icon";

describe("Icon", () => {
  let dispose: () => void;

  afterEach(() => {
    dispose?.();
  });

  function mount(props: { name: IconName; class?: string; stroke?: boolean }) {
    const container = document.createElement("div");
    document.body.appendChild(container);
    dispose = render(() => <Icon {...props} />, container);
    return container;
  }

  it("renders a known icon name with correct path data", () => {
    const container = mount({ name: "file" });
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
    expect(svg!.getAttribute("width")).toBe("24");
    expect(svg!.getAttribute("height")).toBe("24");
    expect(svg!.getAttribute("viewBox")).toBe("0 0 24 24");

    const paths = container.querySelectorAll("path");
    expect(paths.length).toBe(2);
    expect(paths[0].getAttribute("d")).toBe(
      "M6 4H4v16h2zm10-2H6v2h10zm4 4h-2v14h2zm-2 14H6v2h12zM16 4h2v2h-2zm-4 0h2v6h-2z",
    );
    expect(paths[1].getAttribute("d")).toBe("M12 8h6v2h-6z");
  });

  it("returns null for unknown icon name", () => {
    const container = mount({ name: "nonexistent" as IconName });
    const svg = container.querySelector("svg");
    expect(svg).toBeNull();
  });

  it("applies className to svg element", () => {
    const container = mount({ name: "folder", class: "icon-lg text-blue-500" });
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("class")).toBe("icon-lg text-blue-500");
  });

  it("defaults to fill mode when stroke is not set", () => {
    const container = mount({ name: "search" });
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("fill")).toBe("currentColor");
    expect(svg.getAttribute("stroke")).toBeNull();
    expect(svg.getAttribute("stroke-width")).toBeNull();
  });

  it("switches to stroke mode when stroke prop is true", () => {
    const container = mount({ name: "search", stroke: true });
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("fill")).toBe("none");
    expect(svg.getAttribute("stroke")).toBe("currentColor");
    expect(svg.getAttribute("stroke-width")).toBe("1.5");
  });
});

describe("toolIcon", () => {
  it('maps "read_file" -> "file"', () => {
    expect(toolIcon("read_file")).toBe("file");
  });

  it('maps "grep" -> "search"', () => {
    expect(toolIcon("grep")).toBe("search");
  });

  it('maps "edit_file" -> "pencil"', () => {
    expect(toolIcon("edit_file")).toBe("pencil");
  });

  it('maps "list_dir" -> "folder"', () => {
    expect(toolIcon("list_dir")).toBe("folder");
  });

  it('maps search-family tools -> "search"', () => {
    expect(toolIcon("code_search")).toBe("search");
    expect(toolIcon("symbol_lookup")).toBe("search");
  });

  it('maps mcp__* tools -> "package"', () => {
    expect(toolIcon("mcp__github__list_issues")).toBe("package");
  });

  it('maps unknown tools -> "terminal"', () => {
    expect(toolIcon("")).toBe("terminal");
    expect(toolIcon("some_future_tool")).toBe("terminal");
  });
});
