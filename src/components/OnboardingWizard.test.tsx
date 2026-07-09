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
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
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
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
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
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
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

  it("previous button goes back from step 2 to step 1, and from step 1 to step 0", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    // Click next to step 1, then next to step 2
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    // Prev from step 2 → step 1
    const prevBtnStep2 = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.prev"),
    );
    expect(prevBtnStep2).toBeTruthy();
    prevBtnStep2!.click();
    expect(document.body.textContent).toContain("onboarding.features.title");
    // Prev from step 1 → step 0
    const prevBtnStep1 = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.prev"),
    );
    expect(prevBtnStep1).toBeTruthy();
    prevBtnStep1!.click();
    expect(document.body.textContent).toContain("onboarding.welcome.title");
    dispose();
  });

  it("dots reflect current step and clicking goes to that step", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    // Get all buttons and filter just the dot navigation buttons (small round ones with no text)
    const allButtons = Array.from(document.body.querySelectorAll("button"));
    const dotButtons = allButtons.filter((b) => {
      const text = b.textContent?.trim() || "";
      // Dots have no visible text content; nav buttons have "onboarding.next" etc. text
      return text === "";
    });
    // Click dot for step 2
    dotButtons[2]?.click();
    expect(document.body.textContent).toContain("onboarding.signIn.title");
    // Click dot for step 0
    dotButtons[0]?.click();
    expect(document.body.textContent).toContain("onboarding.welcome.title");
    // Click dot for step 1
    dotButtons[1]?.click();
    expect(document.body.textContent).toContain("onboarding.features.title");
    dispose();
  });

  it("sign-in button calls onSignIn when clicked", () => {
    const onSignIn = vi.fn();
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={onSignIn} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
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
        <OnboardingWizard onSignIn={vi.fn()} signingIn={true} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
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
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError="error message" onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
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

  it("apiKeyLink shows API key field when clicked", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.apiKeyLink"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    const input = document.body.querySelector(
      'input[type="password"]',
    ) as HTMLInputElement;
    expect(input).toBeTruthy();
    expect(input.placeholder).toBe("onboarding.signIn.apiKeyPlaceholder");
    dispose();
  });

  it("handleApiKeyContinue does nothing when input is empty", () => {
    const onApiKeySubmit = vi.fn();
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={onApiKeySubmit} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    // Go to step 2
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    // Show API key field
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.apiKeyLink"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    // Button is disabled when input is empty — force-enable it and click to test the handler's early return
    const continueBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.apiKeyContinue"),
    );
    expect(continueBtn).toBeTruthy();
    expect(continueBtn!.disabled).toBe(true);
    // Enable and dispatch click to exercise the handler with empty input
    continueBtn!.disabled = false;
    continueBtn!.click();
    expect(onApiKeySubmit).not.toHaveBeenCalled();
    dispose();
  });

  it("handleApiKeyContinue calls onApiKeySubmit with trimmed input", async () => {
    const onApiKeySubmit = vi.fn().mockResolvedValue(undefined);
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={onApiKeySubmit} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    // Go to step 2
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    // Show API key field
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.apiKeyLink"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    // Type into password input
    const input = document.body.querySelector(
      'input[type="password"]',
    ) as HTMLInputElement;
    expect(input).toBeTruthy();
    input.value = " my-key ";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    // Click continue — wait for Solid to flush and enable the button
    const continueBtn = await vi.waitFor(() => {
      const btn = Array.from(document.body.querySelectorAll("button")).find((b) =>
        b.textContent?.includes("onboarding.signIn.apiKeyContinue"),
      );
      expect(btn).toBeTruthy();
      expect(btn!.disabled).toBe(false);
      return btn!;
    });
    continueBtn.click();
    // Wait for the async handler to complete
    await vi.waitFor(() => {
      expect(onApiKeySubmit).toHaveBeenCalledWith("my-key");
    });
    dispose();
  });

  it("shows loader during API key validation", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={true} apiKeyError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.apiKeyLink"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    expect(document.body.textContent).toContain("onboarding.signIn.apiKeyValidating");
    expect(document.body.querySelector(".animate-spin")).toBeTruthy();
    dispose();
  });

  it("shows apiKeyError when provided", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError="Invalid key" />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.apiKeyLink"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    expect(document.body.textContent).toContain("Invalid key");
    dispose();
  });

  it("apiKeyBack button hides API key field", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.apiKeyLink"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    expect(document.body.querySelector('input[type="password"]')).toBeTruthy();
    const backBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("onboarding.signIn.apiKeyBack"),
    );
    expect(backBtn).toBeTruthy();
    backBtn!.click();
    expect(document.body.querySelector('input[type="password"]')).toBeFalsy();
    dispose();
  });

  it("signingIn shows signing-in message", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={true} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    expect(document.body.textContent).toContain("onboarding.signIn.signingIn");
    dispose();
  });

  it("signInError shows error text on step 2", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError="Login failed" onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    expect(document.body.textContent).toContain("Login failed");
    dispose();
  });

  it("displays all 4 feature cards on step 1", () => {
    const dispose = render(
      () => (
        <OnboardingWizard onSignIn={vi.fn()} signingIn={false} signInError={null} onApiKeySubmit={vi.fn()} apiKeyValidating={false} apiKeyError={null} />
      ),
      document.body,
    );
    const nextBtns = () =>
      Array.from(document.body.querySelectorAll("button")).filter((b) =>
        b.textContent?.includes("onboarding.next"),
      );
    nextBtns()[0]?.click();
    expect(document.body.textContent).toContain("onboarding.features.agent.title");
    expect(document.body.textContent).toContain("onboarding.features.approval.title");
    expect(document.body.textContent).toContain("onboarding.features.subagents.title");
    expect(document.body.textContent).toContain("onboarding.features.indexing.title");
    dispose();
  });
});
