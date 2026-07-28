//! The pairing code: what the QR carries, and why it carries it that way.
//!
//! # The code is the whole out-of-band channel
//!
//! Noise IK works because the initiator already knows the responder's static
//! key. This code is how it comes to know it. Which means a doctored code is the
//! one attack IK cannot detect on its own — hence the word check in
//! `pairing.rs`, which runs after the handshake and before anything is served.
//!
//! # Everything sensitive goes in the fragment
//!
//! A URL fragment is never sent to the server. Putting the channel token and the
//! device key after `#` means the web origin — its HTTP logs, its CDN, whatever
//! sits in front of it — never receives either. In a query string they would be
//! in the request line of every page load, which would hand the origin the two
//! things it must not have: the ability to attach to the channel, and the key
//! that a substitution attack needs to forge.
//!
//! That is the reason the web app has to read its parameters from
//! `location.hash`, and it is not a detail it may quietly change.

use claudinio_protocol::wire::ChannelId;

/// Where the browser half of this lives.
pub const DEFAULT_WEB_ORIGIN: &str = "https://app.claudin.io";

/// A minted, not-yet-used pairing code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub channel: ChannelId,
    /// The relay's routing capability for this channel.
    ///
    /// A second capability alongside the channel id, and the relay's own design
    /// says why: it is "a bearer capability for routing, not for confidentiality".
    /// The confidentiality guarantee is the handshake. What this adds is that a
    /// token can be rotated without changing the channel, and that the relay has
    /// something to check before it burns bandwidth on a stranger.
    pub token: String,
    /// The device's static public key, hex. Public by design: this is exactly
    /// what a pairing code is for.
    pub device_key: String,
    pub relay_url: String,
    /// Unix millis. The code, not the pairing.
    pub expires_at: u64,
}

impl PairingCode {
    /// Mint a code with a fresh random channel.
    ///
    /// The channel id is the only thing stopping a third party from attaching to
    /// someone else's channel on the relay, so it comes from the OS entropy pool
    /// rather than from a counter, a timestamp or a hash of anything.
    pub fn mint(device_key: String, relay_url: String, expires_at: u64) -> Result<Self, String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| format!("no entropy for a channel id: {e}"))?;
        let mut token_bytes = [0u8; 16];
        getrandom::fill(&mut token_bytes).map_err(|e| format!("no entropy for a token: {e}"))?;
        Ok(Self {
            channel: ChannelId::from_bytes(bytes),
            token: token_bytes.iter().map(|b| format!("{b:02x}")).collect(),
            device_key,
            relay_url,
            expires_at,
        })
    }

    /// The URL a phone's camera opens.
    ///
    /// Reachable straight from the browser with nothing installed, which the plan
    /// requires: a PWA is wanted for push, never as a precondition for pairing.
    pub fn url(&self, web_origin: &str) -> String {
        format!(
            "{}/#c={}&t={}&k={}&r={}&e={}",
            web_origin.trim_end_matches('/'),
            self.channel.to_hex(),
            self.token,
            self.device_key,
            escape(&self.relay_url),
            self.expires_at,
        )
    }

    /// The same URL as an inline SVG.
    ///
    /// Rendered here rather than in the webview because the code is already in
    /// this process, and because it is the device that has to be able to show it
    /// with no network — the relay may well be the thing being set up.
    pub fn qr_svg(&self, web_origin: &str) -> Result<String, String> {
        use qrcode::QrCode;
        use qrcode::render::svg;

        let code = QrCode::new(self.url(web_origin).as_bytes())
            .map_err(|e| format!("the pairing code will not fit in a QR: {e}"))?;
        Ok(code
            .render::<svg::Color>()
            // No quiet zone from the renderer: the panel provides the margin, and
            // a doubled one just shrinks the part a camera has to resolve.
            .quiet_zone(false)
            .min_dimensions(240, 240)
            .dark_color(svg::Color("#000000"))
            .light_color(svg::Color("#ffffff"))
            .build())
    }
}

/// Percent-encode what would otherwise be read as structure.
///
/// Only the five characters that matter: `%` first, or escaping would corrupt the
/// escapes it just wrote, then the two fragment delimiters, then `#` and space. A
/// `wss://host/path` URL contains none of them, so in practice this changes
/// nothing — it is here so that a relay URL with a query string does not silently
/// truncate the parameters after it.
fn escape(s: &str) -> String {
    s.replace('%', "%25")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('#', "%23")
        .replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> PairingCode {
        PairingCode {
            channel: ChannelId::from_bytes([0xab; 16]),
            token: "ef".repeat(16),
            device_key: "cd".repeat(32),
            relay_url: "wss://relay.claudin.io/ws".to_string(),
            expires_at: 1_800_000_120_000,
        }
    }

    /// The one property that matters about the shape of this URL. If either the
    /// channel or the key ever moves in front of the `#`, the web origin starts
    /// receiving both in its access logs.
    #[test]
    fn nothing_secret_appears_before_the_fragment() {
        let url = code().url(DEFAULT_WEB_ORIGIN);
        let (before, after) = url.split_once('#').expect("there is a fragment");

        assert!(!before.contains("abab"), "the channel is in {before}");
        assert!(!before.contains("efef"), "the channel token is in {before}");
        assert!(!before.contains("cdcd"), "the device key is in {before}");
        assert!(after.contains(&"ab".repeat(16)));
        assert!(after.contains(&"cd".repeat(32)));
    }

    #[test]
    fn the_url_carries_everything_the_browser_needs() {
        let url = code().url(DEFAULT_WEB_ORIGIN);
        assert!(url.starts_with("https://app.claudin.io/#"));
        assert!(url.contains("c=abababababababababababababababab"));
        assert!(url.contains(&format!("t={}", "ef".repeat(16))));
        assert!(url.contains(&format!("k={}", "cd".repeat(32))));
        assert!(url.contains("r=wss://relay.claudin.io/ws"));
        assert!(url.contains("e=1800000120000"));
    }

    /// A trailing slash on the origin would produce `//#`, which is a different
    /// path and would 404 on a strict host.
    #[test]
    fn a_trailing_slash_on_the_origin_does_not_double() {
        assert_eq!(
            code().url("https://app.claudin.io/"),
            code().url("https://app.claudin.io")
        );
    }

    /// A relay URL with a query string must not swallow the parameters after it.
    #[test]
    fn a_relay_url_cannot_truncate_the_code() {
        let mut c = code();
        c.relay_url = "wss://relay.claudin.io/ws?region=eu&tier=free".to_string();
        let url = c.url(DEFAULT_WEB_ORIGIN);

        // Both the escaped delimiters survive, and `e=` is still findable as a
        // parameter rather than having been absorbed into `r=`.
        assert!(url.contains("region%3Deu%26tier%3Dfree"));
        assert!(url.ends_with("&e=1800000120000"));
    }

    #[test]
    fn escaping_does_not_corrupt_its_own_escapes() {
        assert_eq!(escape("100%&x=1"), "100%25%26x%3D1");
    }

    /// Two codes minted back to back must not share a channel. A collision would
    /// put two pairings on one relay channel.
    #[test]
    fn every_minted_code_gets_its_own_channel() {
        let a = PairingCode::mint("cd".repeat(32), "wss://r".into(), 0).unwrap();
        let b = PairingCode::mint("cd".repeat(32), "wss://r".into(), 0).unwrap();
        assert_ne!(a.channel, b.channel);
    }

    /// The relay refuses a token shorter than 16 characters, so a minted one that
    /// fell under would make every attach fail — the shape of the bug this whole
    /// change came out of.
    #[test]
    fn a_minted_token_is_long_enough_for_the_relay() {
        let code = PairingCode::mint("cd".repeat(32), "wss://r".into(), 0).unwrap();
        assert!(code.token.len() >= 16, "token was {}", code.token);
    }

    #[test]
    fn every_minted_code_gets_its_own_token() {
        let a = PairingCode::mint("cd".repeat(32), "wss://r".into(), 0).unwrap();
        let b = PairingCode::mint("cd".repeat(32), "wss://r".into(), 0).unwrap();
        assert_ne!(a.token, b.token);
    }

    /// Not a strong randomness test — that is the OS's job — but it catches the
    /// failure that would matter: a channel of all zeros, which is what a
    /// forgotten fill leaves behind.
    #[test]
    fn a_minted_channel_is_not_all_zeroes() {
        let code = PairingCode::mint("cd".repeat(32), "wss://r".into(), 0).unwrap();
        assert_ne!(code.channel.as_bytes(), &[0u8; 16]);
    }

    #[test]
    fn the_qr_renders_as_svg() {
        let svg = code().qr_svg(DEFAULT_WEB_ORIGIN).unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(svg.contains("</svg>"));
    }

    /// The QR has to hold a code with a long relay URL too. Version 40 tops out
    /// around 2900 bytes, and a pairing code is ~200, but the failure would be a
    /// blank panel at the moment of pairing.
    #[test]
    fn a_long_relay_url_still_fits_in_a_qr() {
        let mut c = code();
        c.relay_url = format!("wss://{}.example.com/ws", "a".repeat(200));
        assert!(c.qr_svg(DEFAULT_WEB_ORIGIN).is_ok());
    }
}
