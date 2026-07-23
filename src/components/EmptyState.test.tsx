import { describe, it, expect, vi } from "vitest";
import { render } from "solid-js/web";
import { EmptyState } from "./EmptyState";


describe("EmptyState", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("shows empty state text", () => {
    const dispose = render(
      () => (
        <EmptyState
          recentProjects={[]}
          openRecent={vi.fn()}
          openFolder={vi.fn()}
        />
      ),
      document.body,
    );
    expect(document.body.textContent).toContain("Claudinio Code");
    expect(document.body.textContent).toContain("Open a project folder to start using the agent.");
    expect(document.body.textContent).toContain("Open folder");
    dispose();
  });

  it("renders recent projects when provided", () => {
    const dispose = render(
      () => (
        <EmptyState
          recentProjects={["/path/alpha", "/other/beta"]}
          openRecent={vi.fn()}
          openFolder={vi.fn()}
        />
      ),
      document.body,
    );

    expect(document.body.textContent).toContain("Recent");
    expect(document.body.textContent).toContain("alpha");
    expect(document.body.textContent).toContain("beta");
    dispose();
  });

  it("does not show recent section when empty", () => {
    const dispose = render(
      () => (
        <EmptyState
          recentProjects={[]}
          openRecent={vi.fn()}
          openFolder={vi.fn()}
        />
      ),
      document.body,
    );

    expect(document.body.textContent).not.toContain("Recent");
    dispose();
  });

  it("clicking open folder button calls openFolder", () => {
    const openFolder = vi.fn();
    const dispose = render(
      () => (
        <EmptyState
          recentProjects={[]}
          openRecent={vi.fn()}
          openFolder={openFolder}
        />
      ),
      document.body,
    );

    const button = document.body.querySelector("button")!;
    button.click();
    expect(openFolder).toHaveBeenCalledTimes(1);
    dispose();
  });

  it("clicking a project calls openRecent with its path", () => {
    const openRecent = vi.fn();
    const projects = ["/projects/my-app"];
    const dispose = render(
      () => (
        <EmptyState
          recentProjects={projects}
          openRecent={openRecent}
          openFolder={vi.fn()}
        />
      ),
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const projectBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("my-app"),
    );
    expect(projectBtn).toBeTruthy();
    projectBtn!.click();
    expect(openRecent).toHaveBeenCalledWith("/projects/my-app");
    dispose();
  });
});
