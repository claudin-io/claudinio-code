import { createSignal } from "solid-js";

function createThemeSignal() {
  const [mode, setMode] = createSignal<"dark" | "light">("dark");

  if (typeof window !== "undefined") {
    const mql = window.matchMedia("(prefers-color-scheme: light)");
    const update = () => {
      const next = mql.matches ? "light" : "dark";
      setMode(next);
      document.documentElement.dataset.theme = next;
    };
    update();
    mql.addEventListener("change", update);
  }

  return mode;
}

export const theme = createThemeSignal();
