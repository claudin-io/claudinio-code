// This file is generated from src/sas.rs. Do not edit it manually.
//
// The device and the browser must derive the same short authentication
// string, or pairing asks users to compare words that can never agree.

export const SAS_WORDS = [
  "anchor",
  "basalt",
  "cinder",
  "dahlia",
  "ember",
  "fathom",
  "granite",
  "harbour",
  "indigo",
  "juniper",
  "kelvin",
  "lantern",
  "marble",
  "nomad",
  "obsidian",
  "pewter",
  "quartz",
  "rivet",
  "saffron",
  "tundra",
  "umber",
  "vellum",
  "walnut",
  "xenon",
  "yarrow",
  "zephyr",
  "alcove",
  "beacon",
  "cobalt",
  "driftwood",
  "eddy",
  "flint",
] as const;

export const SAS_WORD_COUNT = 3;

export function deriveSas(handshakeHash: Uint8Array): string[] {
  return Array.from({ length: SAS_WORD_COUNT }, (_, i) => {
    const hi = handshakeHash[i * 2] ?? 0;
    const lo = handshakeHash[i * 2 + 1] ?? 0;
    return SAS_WORDS[((hi << 8) | lo) % SAS_WORDS.length];
  });
}

export function formatSas(handshakeHash: Uint8Array): string {
  return deriveSas(handshakeHash).join(" \u00b7 ");
}
