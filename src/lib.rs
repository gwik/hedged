#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

//! This crate provides functionality to perform hedged requests, inspired by
//! the strategies described in ["The Tail at Scale"](https://research.google/pubs/pub40801/).
//!
//! Hedged requests help in mitigating latency variability in distributed systems by initiating
//! redundant operations and using the result of the first one to complete.
//!
//! # Features
//!
//! - `tokio`: Enables asynchronous support, including the [`Hedge::send`] method for performing hedged requests.

use histogram::AtomicHistogram;
use std::{
    sync::atomic::{AtomicU64, Ordering::*},
    time::Duration,
};

/// Error type returned by this crate.
#[derive(Debug, thiserror::Error)]
#[error("hedged error: {source}")]
pub struct Error {
    #[source]
    source: histogram::Error,
}

impl From<histogram::Error> for Error {
    fn from(value: histogram::Error) -> Self {
        Self { source: value }
    }
}

/// A `Hedge` object for managing and performing hedged requests.
///
/// This struct tracks the duration of requests, and calculates the appropriate
/// timeout duration after which a second, redundant request should be issued.
///
/// # Examples
///
/// ```
/// use hedged::Hedge;
/// use std::time::Duration;
///
/// let hedge = Hedge::new(7, 64, Duration::from_secs(30), 10, 0.95)
///     .expect("Failed to create a Hedge");
/// ```
pub struct Hedge {
    histogram: AtomicHistogram,
    current_usec: AtomicU64,
    observation_count: AtomicU64,
    period: u64,
    percentile: f64,
}

impl Default for Hedge {
    fn default() -> Hedge {
        Self {
            histogram: AtomicHistogram::new(7, 64).expect("histogram"),
            current_usec: AtomicU64::new(
                Duration::from_secs(30)
                    .as_micros()
                    .try_into()
                    .expect("valid timeout"),
            ),
            observation_count: AtomicU64::new(0),
            period: 10,
            percentile: 0.95,
        }
    }
}

impl Hedge {
    /// Creates a new instance of `Hedge`.
    ///
    /// # Parameters
    ///
    /// - `p`: The power of resolution for the internal histogram.
    /// - `n`: The number of significant figures for the internal histogram.
    /// - `initial_timeout`: The initial timeout duration for hedged requests.
    /// - `period`: The number of observations between updates to the percentile value.
    /// - `percentile`: The percentile for calculating timeout durations. This should be a float
    ///   value in the `(0.0, 1.0]` range, representing the percentile as a decimal. Usually `0.95`.
    ///
    /// Refer to the [`histogram::Config`](https://docs.rs/histogram/latest/histogram/struct.Config.html) documentation at
    /// for guidance on configuring the histogram parameters `p` and `n`.
    ///
    /// # Returns
    ///
    /// A `Result` wrapping the newly created `Hedge` object. If creating the internal histogram
    /// fails, an error is returned.
    ///
    /// # Panics
    ///
    /// This function will panic if `percentile` is not in the `(0.0, 1.0]` range.
    pub fn new(
        p: u8,
        n: u8,
        initial_timeout: Duration,
        period: u64,
        percentile: f64,
    ) -> Result<Self, histogram::Error> {
        if percentile <= 0.0 || percentile > 1.0 {
            panic!("percentile should in (0.0, 1.0], was {percentile}");
        }
        Ok(Self {
            histogram: AtomicHistogram::new(p, n)?,
            current_usec: AtomicU64::new(
                initial_timeout
                    .as_micros()
                    .try_into()
                    .map_err(|_| histogram::Error::Overflow)?,
            ),
            observation_count: AtomicU64::new(0),
            period,
            percentile: percentile * 100.0,
        })
    }

    /// Sets the initial timeout duration to be used before the completion of the first period,
    /// after which the percentile calculation takes place.
    pub fn with_initial_timeout(self, timeout: Duration) -> Result<Self, histogram::Error> {
        Ok(Self {
            current_usec: AtomicU64::new(
                timeout
                    .as_micros()
                    .try_into()
                    .map_err(|_| histogram::Error::Overflow)?,
            ),
            ..self
        })
    }

    /// Sets the period in number of requests.
    ///
    /// # Panics
    ///
    /// Panics if the period is zero.
    pub fn with_period(self, period: u64) -> Result<Self, histogram::Error> {
        if period == 0 {
            panic!("period must be greater that 0");
        }
        Ok(Self { period, ..self })
    }

    /// Executes a hedged request algorithm.
    ///
    /// This function initiates a request by calling the provided closure `f` to obtain a future.
    /// It then polls the future, awaiting its completion. If the future does not complete within
    /// the specified percentile of the expected duration, a second request is initiated by again
    /// calling `f`. The function then waits for either of the two futures to complete and returns
    /// the result of the first one that does.
    ///
    /// Requires the `tokio` feature.
    ///
    /// # Parameters
    ///
    /// - `f`: A closure that, when called, returns a future that performs a request.
    ///
    /// # Returns
    ///
    /// A tuple of two elements:
    /// 1. The result `R` of the future that completes first.
    /// 2. An `Option` wrapping a `Pin<Box<Fut>>`. If the first request completes before the
    ///    timeout at the current percentile value, this is `None`. If both requests are initiated
    ///    and one completes, this is `Some(future)`, where `future` is the request that did not
    ///    complete. This allows for potential cleanup, especially if the future is not cancel-safe.
    ///
    #[cfg(feature = "tokio")]
    pub async fn send<F, Fut, R>(&self, mut f: F) -> (R, Option<std::pin::Pin<Box<Fut>>>)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = R>,
    {
        // Run the first request.

        use tokio::time::{timeout, Instant};

        let mut first_request = Box::pin((f)());
        let first_start = Instant::now();

        if let Ok(res) = timeout(self.value(), first_request.as_mut()).await {
            let _ = self.observe(first_start.elapsed());
            return (res, None);
        };

        // The first request has timed out. Start a second one and
        // run them concurrently.

        let mut second_request = Box::pin((f)());
        let second_start = Instant::now();

        let (is_first, res) = tokio::select! {
            res = first_request.as_mut() => {
                let _ = self.observe(first_start.elapsed());
                (true, res)
            }
            res = second_request.as_mut() => {
                let _ = self.observe(second_start.elapsed());
                (false, res)
            }
        };

        let rem = if is_first {
            second_request
        } else {
            first_request
        };

        (
            res,
            // Make sure all futures are polled to completion because they may
            // not be cancel-safe.
            Some(rem),
        )
    }

    /// Returns the current value of the percentile.
    pub fn value(&self) -> Duration {
        let current = self.current_usec.load(Relaxed);
        Duration::from_micros(current)
    }

    /// Observes the duration of a single request.
    pub fn observe(&self, duration: Duration) -> Result<(), histogram::Error> {
        self.histogram.increment(
            duration
                .as_micros()
                .try_into()
                .map_err(|_| histogram::Error::Overflow)?,
        )?;

        let observation_count = self.observation_count.fetch_add(1, SeqCst) + 1;
        if observation_count % self.period == 0 {
            self.rollout()?;
        }

        Ok(())
    }

    #[inline(always)]
    fn rollout(&self) -> Result<(), histogram::Error> {
        let snap = self.histogram.snapshot();
        let bucket = snap.percentile(self.percentile)?;
        self.current_usec.store(bucket.end(), Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_rollout_at_period() {
        let initial = Duration::from_secs(30);
        let inner = Hedge::new(7, 64, initial, 10, 0.9).unwrap();

        assert_eq!(initial, inner.value());

        inner.observe(Duration::from_secs(1)).unwrap();
        assert_eq!(initial, inner.value());
        inner.observe(Duration::from_secs(2)).unwrap();
        assert_eq!(initial, inner.value());
        inner.observe(Duration::from_secs(3)).unwrap();
        assert_eq!(initial, inner.value());
        inner.observe(Duration::from_secs(3)).unwrap();
        assert_eq!(initial, inner.value());
        inner.observe(Duration::from_secs(1)).unwrap();
        assert_eq!(initial, inner.value());
        inner.observe(Duration::from_secs(2)).unwrap();
        assert_eq!(initial, inner.value());
        inner.observe(Duration::from_secs(3)).unwrap();
        assert_eq!(initial, inner.value());
        inner.observe(Duration::from_secs(3)).unwrap();
        assert_eq!(initial, inner.value());
        inner.observe(Duration::from_secs(3)).unwrap();
        assert_eq!(initial, inner.value());

        inner.observe(Duration::from_secs(10)).unwrap();
        assert_eq!(3.0, inner.value().as_secs_f64().round());
    }
}
