# hedged

This crate provides functionality to perform hedged requests, inspired by
the strategies described in ["The Tail at Scale"](https://research.google/pubs/pub40801/).

Hedged requests help in mitigating latency variability in distributed systems by initiating
redundant operations and using the result of the first one to complete.

## Features

 - `tokio`: Enables asynchronous support, including the [`Hedge::send`] method for performing hedged requests.

## Usage

```rust
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

use hedged::Hedge;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let hedge = Hedge::new(7, 64, Duration::from_secs(30), 1, 0.95)?;

    let fast_request = || async { "ok" };

    let (result, rem) = hedge.send(fast_request).await;
    assert!(rem.is_none());
    assert_eq!(result, "ok");

    let called = Arc::new(AtomicU32::new(0));
    let slow_request = {
        || {
            called.fetch_add(1, Ordering::SeqCst);
            async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                "ok"
            }
        }
    };

    let (_, rem) = hedge.send(slow_request).await;

    // a second request was performed.
    assert!(rem.is_some());
    assert_eq!(called.load(Ordering::SeqCst), 2);

    // Wait for completion in a new task if the request is not cancel-safe.
    // For example a connection might need to be returned to a pool for reuse.
    if let Some(rem) = rem {
        tokio::spawn(rem);
    }

    Ok(())
}
```
