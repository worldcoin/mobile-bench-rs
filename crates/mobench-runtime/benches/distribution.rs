use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use mobench_runtime::Distribution;

const SAMPLE_SIZES: [usize; 3] = [10, 10_000, 1_000_000];

fn samples(size: usize) -> Vec<u64> {
    (0..size)
        .map(|index| (index as u64).wrapping_mul(6_364_136_223_846_793_005))
        .collect()
}

fn benchmark_distribution(c: &mut Criterion) {
    let mut group = c.benchmark_group("distribution");
    group.sample_size(10);

    for size in SAMPLE_SIZES {
        let values = samples(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::new("from_slice", size), &size, |b, _| {
            b.iter(|| Distribution::from_slice(black_box(&values)));
        });
        group.bench_with_input(BenchmarkId::new("cli_v1_summary", size), &size, |b, _| {
            b.iter(|| {
                Distribution::from_slice(black_box(&values))
                    .cli_v1_summary()
                    .expect("non-empty benchmark distribution")
            });
        });
        group.bench_with_input(BenchmarkId::new("sdk_v1_summary", size), &size, |b, _| {
            b.iter(|| Distribution::from_slice(black_box(&values)).sdk_v1_summary());
        });
    }

    group.finish();
}

criterion_group!(benches, benchmark_distribution);
criterion_main!(benches);
