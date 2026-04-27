#[test]
fn benchmark_macro_rejects_async_functions() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/async_benchmark.rs");
}
