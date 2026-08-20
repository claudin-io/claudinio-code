//! Hex encoding and CSPRNG-backed random strings.
//!
//! Lives in the crate root rather than under `commands/`: the llama supervisor
//! needs a random api-key and is a core module, which may not import the
//! command layer (see the architecture test in `lib.rs`).

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Random hex string of `n_bytes` bytes, sourced from `uuid`'s CSPRNG-backed
/// v4 generator so we don't need to add a dedicated `rand` dependency.
pub(crate) fn random_hex(n_bytes: usize) -> String {
    let mut bytes = Vec::with_capacity(n_bytes);
    while bytes.len() < n_bytes {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes.truncate(n_bytes);
    hex_encode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encode_is_lowercase_and_padded() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn random_hex_has_the_requested_length_and_varies() {
        let a = random_hex(32);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, random_hex(32));
    }
}
