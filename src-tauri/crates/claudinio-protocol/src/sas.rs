//! The short authentication string both screens show after a handshake.
//!
//! §6.3 step 5: the device and the browser each derive words from the handshake
//! hash and the user confirms they match. That is what catches the manual-entry
//! pairing path, where the device's key came from a typed code rather than a
//! scanned one — a relay that substituted a key produces a different hash, so
//! the words differ.
//!
//! The check only works if both sides derive identically, which makes the
//! wordlist and the derivation a wire contract rather than a UI detail. It lives
//! here, in the crate both consumers share, and the TypeScript copy is generated
//! from this file. A mismatch would tell users to compare words that can never
//! agree, and they would learn to click through it.

/// Kept deliberately short and unambiguous when read aloud: pairing is often two
/// people, or one person reading a phone while looking at a laptop.
pub const WORDS: [&str; 32] = [
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
];

/// How many words are shown. Three of 32 is 15 bits — enough that a relay cannot
/// search for a substituted key that lands on the same words within the 120 s
/// pairing window, and few enough that people actually read them.
pub const WORD_COUNT: usize = 3;

/// Derive the words from a handshake hash.
///
/// Takes big-endian pairs from the front of the hash. Both ends run this, so the
/// only thing that matters is that they run it the same way.
pub fn derive(handshake_hash: &[u8]) -> Vec<&'static str> {
    (0..WORD_COUNT)
        .map(|i| {
            let hi = *handshake_hash.get(i * 2).unwrap_or(&0) as usize;
            let lo = *handshake_hash.get(i * 2 + 1).unwrap_or(&0) as usize;
            WORDS[((hi << 8) | lo) % WORDS.len()]
        })
        .collect()
}

/// The words as one string, formatted the way both UIs show them.
pub fn format(handshake_hash: &[u8]) -> String {
    derive(handshake_hash).join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_hash_gives_the_same_words() {
        let hash = [0xAB, 0xCD, 0x12, 0x34, 0x56, 0x78];
        assert_eq!(derive(&hash), derive(&hash));
    }

    /// The whole point: a different handshake — which is what a substituted key
    /// produces — must give different words.
    #[test]
    fn a_different_hash_gives_different_words() {
        let honest = derive(&[0x00, 0x01, 0x00, 0x02, 0x00, 0x03]);
        let tampered = derive(&[0x00, 0x04, 0x00, 0x05, 0x00, 0x06]);
        assert_ne!(honest, tampered);
    }

    #[test]
    fn it_always_yields_the_advertised_number_of_words() {
        assert_eq!(derive(&[0u8; 32]).len(), WORD_COUNT);
        // A short hash must not panic; pairing failing closed is fine, panicking
        // in the middle of it is not.
        assert_eq!(derive(&[1, 2]).len(), WORD_COUNT);
        assert_eq!(derive(&[]).len(), WORD_COUNT);
    }

    #[test]
    fn words_are_unique_so_two_positions_cannot_be_confused() {
        let mut sorted = WORDS;
        sorted.sort_unstable();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(deduped.len(), WORDS.len(), "the wordlist has a duplicate");
    }

    /// Pinned. Changing the derivation or the list silently would make every
    /// already-paired browser disagree with its device, and the failure would
    /// look like an attack rather than a bug.
    #[test]
    fn the_derivation_is_pinned() {
        let hash: Vec<u8> = (0u8..32).collect();
        // Hand-checked: (0<<8)|1 = 1 -> WORDS[1]; (2<<8)|3 = 515, 515 % 32 = 3;
        // (4<<8)|5 = 1029, 1029 % 32 = 5.
        assert_eq!(format(&hash), "basalt · dahlia · fathom");
    }

    /// The TypeScript copy is generated, never hand-maintained, and CI fails if
    /// it drifts from this file.
    #[test]
    fn ts_bindings_are_written() {
        let words = WORDS
            .iter()
            .map(|w| format!("  \"{w}\","))
            .collect::<Vec<_>>()
            .join("\n");

        let contents = format!(
            "// This file is generated from src/sas.rs. Do not edit it manually.\n\
             //\n\
             // The device and the browser must derive the same short authentication\n\
             // string, or pairing asks users to compare words that can never agree.\n\
             \n\
             export const SAS_WORDS = [\n{words}\n] as const;\n\
             \n\
             export const SAS_WORD_COUNT = {WORD_COUNT};\n\
             \n\
             export function deriveSas(handshakeHash: Uint8Array): string[] {{\n\
             \x20 return Array.from({{ length: SAS_WORD_COUNT }}, (_, i) => {{\n\
             \x20   const hi = handshakeHash[i * 2] ?? 0;\n\
             \x20   const lo = handshakeHash[i * 2 + 1] ?? 0;\n\
             \x20   return SAS_WORDS[((hi << 8) | lo) % SAS_WORDS.length];\n\
             \x20 }});\n\
             }}\n\
             \n\
             export function formatSas(handshakeHash: Uint8Array): string {{\n\
             \x20 return deriveSas(handshakeHash).join(\" \\u00b7 \");\n\
             }}\n"
        );

        let dir = std::path::Path::new("bindings");
        std::fs::create_dir_all(dir).expect("create bindings dir");
        std::fs::write(dir.join("sas.ts"), contents).expect("write sas.ts");
    }
}
