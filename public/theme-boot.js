// Applies the stored theme before first paint, so the app never flashes the
// wrong background. Kept as a separate file rather than an inline <script> so
// the production CSP can use a plain `script-src 'self'` with no inline
// allowance — inline script execution is what turns injected markup in the chat
// into running code, so that directive has to stay strict.
(function () {
  var theme;
  try {
    var stored = localStorage.getItem("claudinio_theme");
    if (stored) {
      // Legacy migration: old "dark"/"light"/"sepia" keys → new theme ids
      if (stored === "dark") theme = "claudinio";
      else if (stored === "light") theme = "claudinio-light";
      else if (stored === "sepia") theme = "claudinio-sepia";
      else theme = stored;
    }
  } catch (_) {}
  if (!theme) {
    theme = window.matchMedia("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
  }
  document.documentElement.dataset.theme = theme;
  if (navigator.userAgent.includes("Mac OS X")) {
    document.documentElement.classList.add("is-macos");
  }
})();
