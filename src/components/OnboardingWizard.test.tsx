import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { OnboardingWizard } from "./OnboardingWizard";


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
    expect(document.body.textContent).toContain("Welcome to Claudinio Code");
    expect(document.body.textContent).toContain("Your AI agent for software development. Plan, code, and execute tasks with autonomous agents.");
    expect(document.body.textContent).toContain("Maximum productivity with AI that understands your code.");
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
      b.textContent?.includes("Next"),
    );
    expect(nextBtn).toBeTruthy();
    nextBtn!.click();
    expect(document.body.textContent).toContain("What you can do");
    expect(document.body.textContent).toContain("Intelligent Agent");
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    expect(document.body.textContent).toContain("Let's get started");
    expect(document.body.textContent).toContain("Sign in with claudin.io");
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    // Prev from step 2 → step 1
    const prevBtnStep2 = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Previous"),
    );
    expect(prevBtnStep2).toBeTruthy();
    prevBtnStep2!.click();
    expect(document.body.textContent).toContain("What you can do");
    // Prev from step 1 → step 0
    const prevBtnStep1 = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Previous"),
    );
    expect(prevBtnStep1).toBeTruthy();
    prevBtnStep1!.click();
    expect(document.body.textContent).toContain("Welcome to Claudinio Code");
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
      // Dots have no visible text content; nav buttons have "Next" etc. text
      return text === "";
    });
    // Click dot for step 2
    dotButtons[2]?.click();
    expect(document.body.textContent).toContain("Let's get started");
    // Click dot for step 0
    dotButtons[0]?.click();
    expect(document.body.textContent).toContain("Welcome to Claudinio Code");
    // Click dot for step 1
    dotButtons[1]?.click();
    expect(document.body.textContent).toContain("What you can do");
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const signInBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Sign in with claudin.io"),
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const signInBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Sign in with claudin.io"),
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
        b.textContent?.includes("Next"),
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Use API Key instead"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    const input = document.body.querySelector(
      'input[type="password"]',
    ) as HTMLInputElement;
    expect(input).toBeTruthy();
    expect(input.placeholder).toBe("Paste your API key");
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    // Show API key field
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Use API Key instead"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    // Button is disabled when input is empty — force-enable it and click to test the handler's early return
    const continueBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Continue"),
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    // Show API key field
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Use API Key instead"),
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
        b.textContent?.includes("Continue"),
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Use API Key instead"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    expect(document.body.textContent).toContain("Validating\u2026");
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Use API Key instead"),
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    const apiKeyLink = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Use API Key instead"),
    );
    expect(apiKeyLink).toBeTruthy();
    apiKeyLink!.click();
    expect(document.body.querySelector('input[type="password"]')).toBeTruthy();
    const backBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("\u2190 Back to sign in"),
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    nextBtns()[0]?.click();
    expect(document.body.textContent).toContain("Waiting for browser sign-in\u2026");
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
        b.textContent?.includes("Next"),
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
        b.textContent?.includes("Next"),
      );
    nextBtns()[0]?.click();
    expect(document.body.textContent).toContain("Intelligent Agent");
    expect(document.body.textContent).toContain("Safe Approvals");
    expect(document.body.textContent).toContain("Parallel Subagents");
    expect(document.body.textContent).toContain("Smart Indexing");
    dispose();
  });
});
