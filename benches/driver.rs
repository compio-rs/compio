use criterion::{criterion_group, criterion_main, Criterion};

criterion_group!(driver, create_and_post_1024);
criterion_main!(driver);

fn create_and_post_1024(c: &mut Criterion) {
    let mut group = c.benchmark_group("create_and_post");

    group.bench_function("create_and_post", |b| {
        b.iter(|| {
            use compio::driver::{Driver, Poller};

            let driver = Driver::new().expect("created");
            for i in 0..1024_usize {
                driver.post(i, 0).expect("succeeded");
            }
        })
    });

    group.finish();
}
