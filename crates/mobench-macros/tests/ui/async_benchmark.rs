use mobench_macros::benchmark;

#[benchmark]
async fn async_work_is_not_a_benchmark() {}

fn main() {}
