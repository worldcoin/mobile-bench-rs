import Foundation

private let defaultFunction = "sample_fns::fibonacci"
private let defaultIterations: UInt32 = 20
private let defaultWarmup: UInt32 = 3

struct BenchParams {
    let function: String
    let iterations: UInt32
    let warmup: UInt32

    static func fromProcessInfo() -> BenchParams {
        let info = ProcessInfo.processInfo
        var function = info.environment["BENCH_FUNCTION"] ?? defaultFunction
        var iterations = UInt32(info.environment["BENCH_ITERATIONS"] ?? "") ?? defaultIterations
        var warmup = UInt32(info.environment["BENCH_WARMUP"] ?? "") ?? defaultWarmup

        for arg in info.arguments {
            if arg.hasPrefix("--bench-function=") {
                function = String(arg.split(separator: "=", maxSplits: 1).last ?? Substring(function))
            } else if arg.hasPrefix("--bench-iterations=") {
                iterations = UInt32(arg.split(separator: "=", maxSplits: 1).last ?? "") ?? iterations
            } else if arg.hasPrefix("--bench-warmup=") {
                warmup = UInt32(arg.split(separator: "=", maxSplits: 1).last ?? "") ?? warmup
            }
        }

        return BenchParams(function: function, iterations: iterations, warmup: warmup)
    }
}

enum BenchRunnerFFI {
    static func runCurrentBenchmark() async -> String {
        let params = BenchParams.fromProcessInfo()
        return run(params: params)
    }

    static func run(params: BenchParams) -> String {
        let rawPtr = params.function.withCString { fname -> UnsafeMutablePointer<CChar>? in
            bench_run_json(fname, params.iterations, params.warmup)
        }
        guard let rawPtr else {
            return "run error: received null pointer from bench_run_json"
        }

        defer { bench_free_string(rawPtr) }
        return String(cString: rawPtr)
    }
}
