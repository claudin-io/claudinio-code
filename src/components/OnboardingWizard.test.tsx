import { describe, it, expect, vi } from "vitest";
import { render } from "solid-js/web";
import { OnboardingWizard } from "./OnboardingWizard";

vi.mock("../lib/grill-me", () => ({
  t: (key: string) => key,
}));

describe("OnboardingWizard", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("renders step 0 (welcome) by default", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} />
      ),
      document.body,
    );
    expect(document.body.textContent).toContain("onboarding.welcome.title");
    expect(document.body.textContent).toContain("onboarding.welcome.subtitle");
    expect(document.body.textContent).toContain("onboarding.welcome.tagline");
    dispose();
  });

  it("next button advances to step 1 (features)", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} />
      ),
      document.body,
    );
    const nextBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.next"),
    );
    expect(nextBtn).toBeTruthy();
    nextBtn!.click();
    expect(document.body.textContent).toContain("onboarding.features.title");
    expect(document.body.textContent).toContain("onboarding.features.agent.title");
    dispose();
  });

  it("next button advances to step 2 (sign in)", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    expect(document.body.textContent).toContain("onboarding.signIn.title");
    expect(document.body.textContent).toContain("onboarding.signIn.button");
    dispose();
  });

  it("previous button goes back from step 2 to step 1", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const prevBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.prev"),
    );
    expect(prevBtn).toBeTruthy();
    prevBtn!.click();
    expect(document.body.textContent).toContain("onboarding.features.title");
    dispose();
  });

  it("dots reflect current step", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} />
      ),
      document.body,
    );
    const dotButtons = Array.from(document.body.querySelectorAll("button")).filter(
      (b) =>
        !b.textContent?.includes("onboarding.next") &&
        !b.textContent?.includes("onboarding.prev"),
    );
    dotButtons[2]?.click();
    expect(document.body.textContent).toContain("onboarding.signIn.title");
    dispose();
  });

  it("sign-in button calls onSignIn when clicked", () => {
    const onSignIn = vi.fn();
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={onSignIn} signingIn={false} signInError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const signInBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.button"),
    );
    expect(signInBtn).toBeTruthy();
    signInBtn!.click();
    expect(onSignIn).toHaveBeenCalledTimes(1);
    dispose();
  });

  it("sign-in button is disabled while signingIn", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={true} signInError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const signInBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.button"),
    );
    expect(signInBtn).toBeTruthy();
    expect(signInBtn!.hasAttribute("disabled")).toBe(true);
    dispose();
  });

  it("shows error message when signInError is provided", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError="error message" />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    expect(document.body.textContent).toContain("error message");
    dispose();
  });
});
