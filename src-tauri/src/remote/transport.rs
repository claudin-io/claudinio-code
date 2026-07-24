//! The outbound connection to the relay.
//!
//! Outbound only, which is invariant I5: no listener is opened on the
//! developer's machine, so remote access works behind NAT and CGNAT with no port
//! forwarding and adds no attack surface to the LAN.
//!
//! Nothing here is allowed to matter to the agent. The relay being unreachable is
//! a discreet offline state, never an error the run can see — I8 says the app is
//! fully functional without any of this.

use std::time::Duration;

/// Reconnect timing: exponential with full jitter, capped.
///
/// Full jitter rather than a fixed sequence because every device that was
/// connected when a relay node restarted will otherwise come back at the same
/// instant, and the reconnect storm finishes what the restart started. Randomised
/// across the whole interval spreads them out.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    cap: Duration,
    attempt: u32,
}

impl Default for Backoff {
    /// 1 s to 60 s, from §7.
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            cap: Duration::from_secs(60),
            attempt: 0,
        }
    }
}

impl Backoff {
    pub fn new(base: Duration, cap: Duration) -> Self {
        Self {
            base,
            cap,
            attempt: 0,
        }
    }

    /// The ceiling for the next delay, before jitter. Exposed so the jitter and
    /// the growth can be tested separately — a test that only looked at the
    /// jittered value could not tell a broken curve from an unlucky draw.
    pub fn ceiling(&self) -> Duration {
        let doubled = self
            .base
            .checked_mul(2u32.saturating_pow(self.attempt))
            .unwrap_or(self.cap);
        doubled.min(self.cap)
    }

    /// How long to wait before the next attempt, and advance.
    pub fn next_delay(&mut self) -> Duration {
        let ceiling = self.ceiling();
        self.attempt = self.attempt.saturating_add(1);
        jitter(ceiling)
    }

    /// Call on a successful connection. Without this a device that reconnects
    /// once an hour would eventually always wait the full minute, because the
    /// attempt counter would never come down.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    pub fn attempts(&self) -> u32 {
        self.attempt
    }
}

/// Uniform in `[0, ceiling]`.
///
/// Uses the system clock rather than pulling in an rng: this picks a retry delay,
/// not a key, and adding a crypto dependency to jitter a reconnect would be the
/// wrong trade.
fn jitter(ceiling: Duration) -> Duration {
    let nanos = ceiling.as_nanos().max(1);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u128)
        .unwrap_or(0);
    // A multiply-shift mix, so consecutive calls in the same millisecond do not
    // return the same value.
    let mixed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    Duration::from_nanos((mixed % nanos) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_doubles_from_the_base() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));

        let ceilings: Vec<u64> = (0..6)
            .map(|_| {
                let c = backoff.ceiling().as_secs();
                backoff.next_delay();
                c
            })
            .collect();

        assert_eq!(ceilings, vec![1, 2, 4, 8, 16, 32]);
    }

    #[test]
    fn the_ceiling_stops_at_the_cap() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));

        for _ in 0..20 {
            backoff.next_delay();
        }

        assert_eq!(backoff.ceiling(), Duration::from_secs(60));
    }

    /// A long-lived device makes a lot of attempts. The doubling must saturate
    /// rather than overflow into a tiny or absurd delay.
    #[test]
    fn a_very_large_attempt_count_does_not_overflow() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));

        for _ in 0..200 {
            backoff.next_delay();
        }

        assert_eq!(backoff.ceiling(), Duration::from_secs(60));
        assert!(backoff.next_delay() <= Duration::from_secs(60));
    }

    #[test]
    fn every_delay_stays_within_its_ceiling() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(5));

        for _ in 0..50 {
            let ceiling = backoff.ceiling();
            let delay = backoff.next_delay();
            assert!(
                delay <= ceiling,
                "delay {delay:?} exceeded its ceiling {ceiling:?}"
            );
        }
    }

    /// The point of full jitter: a relay restart must not bring every device back
    /// at the same instant. Identical delays across calls would mean it does.
    #[test]
    fn delays_are_jittered_rather_than_fixed() {
        let mut distinct = std::collections::HashSet::new();

        for _ in 0..40 {
            let mut backoff = Backoff::new(Duration::from_secs(30), Duration::from_secs(60));
            // Advance a few steps so the ceiling is wide enough for the spread to
            // be visible.
            backoff.next_delay();
            backoff.next_delay();
            distinct.insert(backoff.next_delay().as_nanos());
        }

        assert!(
            distinct.len() > 1,
            "every device would reconnect at the same moment"
        );
    }

    /// Without a reset, a device that drops once an hour would creep up to always
    /// waiting the full cap, and a brief blip would look like an outage.
    #[test]
    fn a_successful_connection_resets_the_curve() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
        for _ in 0..8 {
            backoff.next_delay();
        }
        assert_eq!(backoff.ceiling(), Duration::from_secs(60));

        backoff.reset();

        assert_eq!(backoff.attempts(), 0);
        assert_eq!(backoff.ceiling(), Duration::from_secs(1));
    }

    #[test]
    fn a_zero_ceiling_does_not_divide_by_zero() {
        assert_eq!(jitter(Duration::ZERO), Duration::ZERO);
    }
}
