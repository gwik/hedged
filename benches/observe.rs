use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("observe-period-100", |b| {
        use hedged::Hedge;
        let d = Duration::from_secs(10);
        let hedge = Hedge::new(4, 27, Duration::from_secs(10), 100, 0.99).unwrap();
        b.iter(|| hedge.observe(black_box(d)))
    });

    c.bench_function("observe-period-10", |b| {
        use hedged::Hedge;
        let d = Duration::from_secs(10);
        let hedge = Hedge::new(4, 27, Duration::from_secs(10), 10, 0.99).unwrap();
        b.iter(|| hedge.observe(black_box(d)))
    });

    c.bench_function("observe-period-1", |b| {
        use hedged::Hedge;
        let d = Duration::from_secs(10);
        let hedge = Hedge::new(4, 27, Duration::from_secs(10), 1, 0.99).unwrap();
        b.iter(|| hedge.observe(black_box(d)))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
