use mobench_runtime::{
    CliV1Summary, Distribution, ResourceAccumulator, ResourceAggregate, ResourceSample,
    SdkV1Summary, rounded_percent_u64, saturating_sum_u64, saturating_u128_to_u64,
    saturating_usize_to_u32, sdk_v1_mean_u64, sdk_v1_std_dev_u64,
};
use proptest::prelude::*;

#[test]
fn cli_v1_summary_uses_integer_floor_and_nearest_rank_p95() {
    let samples = (1..=12).collect::<Vec<_>>();

    let summary = Distribution::from_slice(&samples)
        .cli_v1_summary()
        .expect("non-empty summary");

    assert_eq!(
        summary,
        CliV1Summary {
            mean_ns: 6,
            median_ns: 6,
            p95_ns: 12,
            min_ns: 1,
            max_ns: 12,
        }
    );
}

#[test]
fn sdk_v1_summary_keeps_fractional_statistics_and_rounded_index_percentiles() {
    let even = Distribution::from_slice(&[1, 2]).sdk_v1_summary();
    assert_eq!(
        even,
        SdkV1Summary {
            mean_ns: 1.5,
            median_ns: 1.5,
            std_dev_ns: 2_f64.sqrt() / 2.0,
            min_ns: 1,
            max_ns: 2,
            p95_ns: 2.0,
            p99_ns: 2.0,
        }
    );

    let twelve = Distribution::from_slice(&(1..=12).collect::<Vec<_>>()).sdk_v1_summary();
    assert_eq!(twelve.p95_ns, 11.0);
}

#[test]
fn sdk_v1_percentile_clamps_the_public_percentile_input() {
    let distribution = Distribution::from_slice(&[10, 20, 30]);

    assert_eq!(distribution.sdk_v1_percentile(-1.0), 10.0);
    assert_eq!(distribution.sdk_v1_percentile(50.0), 20.0);
    assert_eq!(distribution.sdk_v1_percentile(101.0), 30.0);
    assert_eq!(Distribution::from_slice(&[]).sdk_v1_percentile(95.0), 0.0);
}

#[test]
fn sdk_v1_individual_statistics_match_the_summary_contract() {
    let distribution = Distribution::from_slice(&[1, 2, 9]);
    let summary = distribution.sdk_v1_summary();

    assert_eq!(distribution.sdk_v1_mean(), summary.mean_ns);
    assert_eq!(distribution.sdk_v1_median(), summary.median_ns);
    assert_eq!(distribution.sdk_v1_std_dev(), summary.std_dev_ns);
    assert_eq!(distribution.min(), Some(summary.min_ns));
    assert_eq!(distribution.max(), Some(summary.max_ns));
    assert_eq!(Distribution::from_slice(&[]).min(), None);
    assert_eq!(Distribution::from_slice(&[]).max(), None);
}

#[test]
fn owned_distribution_matches_the_borrowed_contract() {
    let samples = vec![9, 1, 2, u64::MAX];

    assert_eq!(
        Distribution::from_vec(samples.clone()).cli_v1_summary(),
        Distribution::from_slice(&samples).cli_v1_summary()
    );
    assert_eq!(
        Distribution::from_vec(samples.clone()).sdk_v1_summary(),
        Distribution::from_slice(&samples).sdk_v1_summary()
    );
}

#[test]
fn both_v1_policies_handle_values_near_u64_max_without_overflow() {
    let samples = [u64::MAX - 1, u64::MAX];
    let distribution = Distribution::from_slice(&samples);

    let cli = distribution.cli_v1_summary().expect("CLI summary");
    assert_eq!(cli.mean_ns, u64::MAX - 1);
    assert_eq!(cli.median_ns, u64::MAX - 1);
    assert_eq!(cli.max_ns, u64::MAX);

    let sdk = distribution.sdk_v1_summary();
    assert!(sdk.mean_ns.is_finite());
    assert!(sdk.median_ns.is_finite());
    assert_eq!(sdk.max_ns, u64::MAX);
}

#[test]
fn empty_and_zero_distributions_keep_each_v1_surface_contract() {
    assert_eq!(Distribution::from_slice(&[]).cli_v1_summary(), None);
    assert_eq!(
        Distribution::from_slice(&[]).sdk_v1_summary(),
        SdkV1Summary {
            mean_ns: 0.0,
            median_ns: 0.0,
            std_dev_ns: 0.0,
            min_ns: 0,
            max_ns: 0,
            p95_ns: 0.0,
            p99_ns: 0.0,
        }
    );

    let zeroes = Distribution::from_slice(&[0; 20]);
    assert_eq!(zeroes.cli_v1_summary().expect("CLI zero summary").max_ns, 0);
    assert_eq!(zeroes.sdk_v1_summary().std_dev_ns, 0.0);
}

#[test]
fn sdk_and_cli_percentile_policies_remain_intentionally_distinct() {
    for (len, expected_cli, expected_sdk) in [(12, 12, 11.0), (20, 19, 19.0), (32, 31, 30.0)] {
        let samples = (1..=len as u64).collect::<Vec<_>>();
        let distribution = Distribution::from_slice(&samples);

        assert_eq!(
            distribution.cli_v1_summary().expect("CLI summary").p95_ns,
            expected_cli
        );
        assert_eq!(distribution.sdk_v1_summary().p95_ns, expected_sdk);
    }
}

proptest! {
    #[test]
    fn cli_summary_is_permutation_invariant_and_matches_a_u128_reference(
        samples in prop::collection::vec(any::<u64>(), 1..128)
    ) {
        let direct = Distribution::from_slice(&samples)
            .cli_v1_summary()
            .expect("non-empty direct summary");
        let mut reversed = samples.clone();
        reversed.reverse();
        let permuted = Distribution::from_slice(&reversed)
            .cli_v1_summary()
            .expect("non-empty permuted summary");

        prop_assert_eq!(direct, permuted);
        let expected_mean = (samples.iter().map(|sample| u128::from(*sample)).sum::<u128>()
            / samples.len() as u128) as u64;
        prop_assert_eq!(direct.mean_ns, expected_mean);
        prop_assert!(direct.min_ns <= direct.median_ns);
        prop_assert!(direct.median_ns <= direct.p95_ns);
        prop_assert!(direct.p95_ns <= direct.max_ns);
    }
}

#[test]
fn resource_aggregation_uses_only_present_fields_and_saturates_cpu_total() {
    let mut resources = ResourceAccumulator::new();
    resources.record(ResourceSample {
        cpu_time_ms: Some(u64::MAX),
        peak_memory_growth_kb: None,
        process_peak_memory_kb: Some(100),
    });
    resources.record(ResourceSample {
        cpu_time_ms: Some(2),
        peak_memory_growth_kb: Some(50),
        process_peak_memory_kb: None,
    });
    resources.record(ResourceSample {
        cpu_time_ms: None,
        peak_memory_growth_kb: Some(80),
        process_peak_memory_kb: Some(90),
    });

    assert_eq!(
        resources.finish(),
        ResourceAggregate {
            cpu_total_ms: Some(u64::MAX),
            cpu_median_ms: Some((u64::MAX / 2) + 1),
            peak_memory_growth_kb: Some(80),
            process_peak_memory_kb: Some(100),
        }
    );
}

#[test]
fn safe_totals_and_rounded_percentages_handle_extreme_values() {
    assert_eq!(saturating_sum_u64([u64::MAX, 1]), u64::MAX);
    assert_eq!(rounded_percent_u64(u64::MAX, u64::MAX), Some(100));
    assert_eq!(rounded_percent_u64(1, 3), Some(33));
    assert_eq!(rounded_percent_u64(2, 3), Some(67));
    assert_eq!(rounded_percent_u64(1, 0), None);
    assert_eq!(saturating_u128_to_u64(u128::MAX), u64::MAX);
    assert_eq!(
        saturating_usize_to_u32(usize::MAX),
        u32::try_from(usize::MAX).unwrap_or(u32::MAX)
    );
    assert_eq!(sdk_v1_mean_u64([1, 2]), 1.5);
    assert_eq!(sdk_v1_std_dev_u64([1, 2].into_iter()), 2_f64.sqrt() / 2.0);
}
