use std::time::{Duration, Instant};

use hedged::Hedge;
use rand::Rng;
use reqwest::Client;

#[poem::handler]
async fn handle() -> String {
    let wait = if rand::thread_rng().gen::<u32>() % 11 == 0 {
        let wait = Duration::from_millis(100)
            + Duration::from_millis(400).mul_f64(rand::thread_rng().gen());
        tracing::info!("!!! SLOW {wait:?}");
        wait
    } else {
        Duration::from_micros(500)
    };

    tokio::time::sleep(wait).await;
    "OK".into()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "hedged_reqwest,reqwest=debug")
    }

    tracing_subscriber::fmt::init();

    let listener = poem::listener::TcpListener::bind("127.0.0.1:9322");
    let server = poem::Server::new(listener).run(handle);
    tokio::spawn(server);

    let hedge = Hedge::new(7, 64, Duration::from_secs(1), 1, 0.95).unwrap();
    let client = Client::builder()
        .pool_max_idle_per_host(2)
        .http1_only() // so we can see connections being dialed and/or reused
        .build()?;

    let make_request = move || {
        tracing::trace!("make request");
        client.get("http://127.0.0.1:9322/").send()
    };

    for i in 0..100 {
        let start = Instant::now();
        let (res, rem) = hedge.send(&make_request).await;
        res?;
        tracing::debug!(
            "[{i}] latency = {:?}, p95 = {:?}",
            start.elapsed(),
            hedge.value()
        );
        if let Some(rem) = rem {
            tracing::info!("a second request was made");
            // return the connection to the pool
            tokio::spawn(rem);
        }
    }

    Ok(())
}
