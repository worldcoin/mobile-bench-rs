use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use mobench_sdk::timing::{BenchReport, BenchSample, BenchSpec};

const SAMPLE_SIZES: [usize; 3] = [10, 10_000, 1_000_000];

fn report(size: usize) -> BenchReport {
    BenchReport {
        spec: BenchSpec::new("statistics", size as u32, 0).expect("valid benchmark size"),
        samples: (0..size)
            .map(|index| BenchSample {
                duration_ns: (index as u64).wrapping_mul(6_364_136_223_846_793_005),
                cpu_time_ms: Some(index as u64),
                peak_memory_kb: Some(index as u64),
                process_peak_memory_kb: Some(index as u64),
            })
            .collect(),
        phases: Vec::new(),
        timeline: Vec::new(),
    }
}

fn benchmark_sdk_facade(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdk_facade");
    group.sample_size(10);

    for size in SAMPLE_SIZES {
        let report = report(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            BenchmarkId::new("bench_report_summary", size),
            &size,
            |b, _| {
                b.iter(|| black_box(&report).summary());
            },
        );
        group.bench_with_input(BenchmarkId::new("mean_ns", size), &size, |b, _| {
            b.iter(|| black_box(&report).mean_ns());
        });
        group.bench_with_input(BenchmarkId::new("min_max_ns", size), &size, |b, _| {
            b.iter(|| (black_box(&report).min_ns(), black_box(&report).max_ns()));
        });
        group.bench_with_input(BenchmarkId::new("cpu_total_ms", size), &size, |b, _| {
            b.iter(|| black_box(&report).cpu_total_ms());
        });
        group.bench_with_input(BenchmarkId::new("cpu_median_ms", size), &size, |b, _| {
            b.iter(|| black_box(&report).cpu_median_ms());
        });
    }

    group.finish();
}

criterion_group!(benches, benchmark_sdk_facade);
criterion_main!(benches);
