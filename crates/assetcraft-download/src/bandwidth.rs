//! A shared bandwidth budget for the transfer engine.
//!
//! One limiter is shared by every transfer a [`Downloader`](crate::Downloader)
//! runs, so the cap is on the engine rather than on each file: ten transfers at
//! once still total the limit.
//!
//! The bookkeeping is a token bucket that is allowed to go into debt. A request
//! that exceeds the tokens on hand is *reserved* by letting the balance go
//! negative, and the caller waits for however long that debt takes to clear at
//! the configured rate. Because the debt is recorded, a second request sees the
//! first's reservation and waits longer, which is what holds the aggregate rate
//! at the limit when transfers overlap — a plain "sleep until the bucket has
//! room" check would let every waiter wake at the same moment.

use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

/// A cap on aggregate transfer throughput.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BandwidthLimit {
    bytes_per_second: u64,
    burst_bytes: u64,
}

impl BandwidthLimit {
    /// The slowest rate a limit can express, so the arithmetic never divides by
    /// zero.
    pub const MIN_BYTES_PER_SECOND: u64 = 1;

    /// A limit of `bytes_per_second`, allowing one second of throughput to
    /// accumulate as a burst so a fast connection can still start immediately.
    pub fn new(bytes_per_second: u64) -> Self {
        let rate = bytes_per_second.max(Self::MIN_BYTES_PER_SECOND);
        Self {
            bytes_per_second: rate,
            burst_bytes: rate,
        }
    }

    /// Override how many bytes may accumulate for a burst.
    pub fn with_burst(mut self, burst_bytes: u64) -> Self {
        self.burst_bytes = burst_bytes.max(1);
        self
    }

    /// The sustained rate, in bytes per second.
    pub fn bytes_per_second(&self) -> u64 {
        self.bytes_per_second
    }

    /// The largest balance the bucket may accumulate.
    pub fn burst_bytes(&self) -> u64 {
        self.burst_bytes
    }
}

/// A throttle shared by every transfer in one [`Downloader`](crate::Downloader).
#[derive(Debug)]
pub struct BandwidthLimiter {
    rate: f64,
    capacity: f64,
    state: Mutex<Tokens>,
}

#[derive(Debug)]
struct Tokens {
    /// Bytes available now. Negative when a transfer has reserved throughput
    /// that has not elapsed yet.
    balance: f64,
    /// When `balance` was last brought up to date.
    last: Instant,
}

impl BandwidthLimiter {
    /// Build a limiter for `limit`, starting with a full burst available.
    pub fn new(limit: BandwidthLimit) -> Self {
        let rate = limit.bytes_per_second() as f64;
        let capacity = limit.burst_bytes() as f64;
        Self {
            rate,
            capacity,
            state: Mutex::new(Tokens {
                balance: capacity,
                last: Instant::now(),
            }),
        }
    }

    /// Wait until `bytes` may be transferred without exceeding the limit.
    ///
    /// The reservation is taken before the wait, so concurrent callers queue
    /// behind one another instead of all waking at the same instant.
    pub async fn acquire(&self, bytes: u64) {
        let wait = {
            let mut state = self
                .state
                .lock()
                .expect("the bandwidth lock is never poisoned");
            state.reserve(self.rate, self.capacity, Instant::now(), bytes)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

impl Tokens {
    /// Bring the balance up to `now`, reserve `bytes`, and report how long the
    /// caller must wait before those bytes are paid for.
    ///
    /// Taking the reservation synchronously — rather than sleeping and checking
    /// again — is what makes the debt visible to the next caller.
    fn reserve(&mut self, rate: f64, capacity: f64, now: Instant, bytes: u64) -> Duration {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        // Capping only limits the positive side, so debt taken on earlier is
        // never forgotten by an idle period.
        self.balance = (self.balance + elapsed * rate).min(capacity);
        self.last = now;

        if bytes == 0 {
            return Duration::ZERO;
        }

        self.balance -= bytes as f64;
        if self.balance >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.balance / rate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    /// One fixed origin for every simulated instant, so `at(0.0)` is the same
    /// instant each time and the accounting is driven exactly.
    fn base() -> Instant {
        static BASE: OnceLock<Instant> = OnceLock::new();
        *BASE.get_or_init(Instant::now)
    }

    /// A fixed instant to drive the accounting without sleeping.
    fn at(seconds: f64) -> Instant {
        base() + Duration::from_secs_f64(seconds)
    }

    fn bucket(capacity: u64) -> Tokens {
        Tokens {
            balance: capacity as f64,
            last: at(0.0),
        }
    }

    #[test]
    fn a_fresh_bucket_serves_up_to_the_burst_immediately() {
        let mut tokens = bucket(100);
        // Everything within the burst is free.
        assert_eq!(tokens.reserve(10.0, 100.0, at(0.0), 60), Duration::ZERO);
        assert_eq!(tokens.balance, 40.0);
        assert_eq!(tokens.reserve(10.0, 100.0, at(0.0), 40), Duration::ZERO);
        assert_eq!(tokens.balance, 0.0);
    }

    #[test]
    fn exceeding_the_burst_reports_the_time_to_clear_the_debt() {
        let mut tokens = bucket(100);
        // 150 bytes at 10 B/s: 50 bytes over the burst is 5 seconds of debt.
        assert_eq!(
            tokens.reserve(10.0, 100.0, at(0.0), 150),
            Duration::from_secs(5)
        );
        assert_eq!(tokens.balance, -50.0);
    }

    #[test]
    fn a_second_request_waits_behind_the_first_reservation() {
        let mut tokens = bucket(0);
        // The first caller reserves 100 bytes at 10 B/s: it owes 10 seconds.
        assert_eq!(
            tokens.reserve(10.0, 100.0, at(0.0), 100),
            Duration::from_secs(10)
        );
        // One second later, 10 bytes have been paid off, so a fresh 100-byte
        // request owes 190 bytes — 19 seconds — not another 10.
        assert_eq!(
            tokens.reserve(10.0, 100.0, at(1.0), 100),
            Duration::from_secs(19)
        );
    }

    #[test]
    fn idle_time_refills_the_bucket_up_to_the_burst() {
        let mut tokens = Tokens {
            balance: 0.0,
            last: at(0.0),
        };
        // Five idle seconds at 10 B/s bank 50 bytes, which are then free.
        assert_eq!(tokens.reserve(10.0, 100.0, at(5.0), 50), Duration::ZERO);
        assert_eq!(tokens.balance, 0.0);
    }

    #[test]
    fn idle_time_pays_off_debt() {
        let mut tokens = bucket(0);
        assert_eq!(
            tokens.reserve(10.0, 0.0, at(0.0), 50),
            Duration::from_secs(5)
        );
        // Five seconds later the debt has cleared; a zero-byte probe sees a
        // balance of exactly zero.
        assert_eq!(tokens.reserve(10.0, 0.0, at(5.0), 0), Duration::ZERO);
        assert_eq!(tokens.balance, 0.0);
    }

    #[test]
    fn the_balance_never_grows_past_the_burst() {
        let mut tokens = bucket(0);
        tokens.reserve(10.0, 100.0, at(0.0), 10);
        // An hour idle must not bank an hour of throughput.
        tokens.reserve(10.0, 100.0, at(3600.0), 100);
        assert!(tokens.balance <= 0.0, "got {}", tokens.balance);
        assert!(tokens.balance >= -100.0, "got {}", tokens.balance);
    }

    #[test]
    fn a_zero_byte_request_never_waits() {
        let mut tokens = bucket(0);
        tokens.reserve(10.0, 100.0, at(0.0), 50);
        assert_eq!(tokens.reserve(10.0, 100.0, at(1.0), 0), Duration::ZERO);
    }

    #[test]
    fn a_limit_clamps_to_a_positive_rate() {
        let limit = BandwidthLimit::new(0);
        assert_eq!(
            limit.bytes_per_second(),
            BandwidthLimit::MIN_BYTES_PER_SECOND
        );
        assert!(limit.burst_bytes() >= 1);
    }

    #[tokio::test]
    async fn acquire_throttles_a_transfer_that_exceeds_the_burst() {
        // 100 bytes at 1000 B/s is ~100 ms of debt; the assertion is a lower
        // bound, so a slow machine cannot make it fail.
        let limiter = BandwidthLimiter::new(BandwidthLimit::new(1_000).with_burst(1));
        let start = Instant::now();
        limiter.acquire(100).await;
        assert!(
            start.elapsed() >= Duration::from_millis(80),
            "acquire returned after only {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn acquire_within_the_burst_does_not_sleep() {
        let limiter = BandwidthLimiter::new(BandwidthLimit::new(1_000).with_burst(64));
        let start = Instant::now();
        limiter.acquire(16).await;
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "acquire slept for {:?} within the burst",
            start.elapsed()
        );
    }
}
