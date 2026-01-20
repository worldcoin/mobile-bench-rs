import Foundation

private let defaultFunction = "sample_fns::fibonacci"
private let defaultIterations: UInt32 = 20
private let defaultWarmup: UInt32 = 3

struct BenchParams {
    let function: String
    let iterations: UInt32
    let warmup: UInt32

    private struct EncodedBenchSpec: Decodable {
        let function: String
        let iterations: UInt32
        let warmup: UInt32
    }

    static func fromBundle() -> BenchParams? {
        guard let url = Bundle.main.url(forResource: "bench_spec", withExtension: "json") else {
            return nil
        }
        do {
            let data = try Data(contentsOf: url)
            let decoded = try JSONDecoder().decode(EncodedBenchSpec.self, from: data)
            return BenchParams(function: decoded.function, iterations: decoded.iterations, warmup: decoded.warmup)
        } catch {
            return nil
        }
    }

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

    static func resolved() -> BenchParams {
        if let bundled = fromBundle() {
            return bundled
        }
        return fromProcessInfo()
    }
}

enum BenchRunnerFFI {
    static func runCurrentBenchmark() async -> String {
        let params = BenchParams.resolved()
        return run(params: params)
    }

    static func run(params: BenchParams) -> String {
        let spec = BenchSpec(
            name: params.function,
            iterations: params.iterations,
            warmup: params.warmup
        )

        do {
            let report = try runBenchmark(spec: spec)
            return formatBenchReport(report)
        } catch let error as BenchError {
            return formatBenchError(error)
        } catch {
            return "Unexpected error: \(error.localizedDescription)"
        }
    }

    /// Formats a duration in nanoseconds to a human-readable string.
    /// Uses milliseconds (ms) by default, switches to seconds (s) if >= 1000ms.
    private static func formatDuration(_ ns: UInt64) -> String {
        let ms = Double(ns) / 1_000_000.0
        if ms >= 1000.0 {
            let secs = ms / 1000.0
            return String(format: "%.3fs", secs)
        } else {
            return String(format: "%.3fms", ms)
        }
    }

    private static func formatBenchReport(_ report: BenchReport) -> String {
        var output = "=== Benchmark Results ===\n\n"
        output += "Function: \(report.spec.name)\n"
        output += "Iterations: \(report.spec.iterations)\n"
        output += "Warmup: \(report.spec.warmup)\n\n"

        output += "Samples (\(report.samples.count)):\n"
        for (index, sample) in report.samples.enumerated() {
            output += "  \(index + 1). \(formatDuration(sample.durationNs))\n"
        }

        if !report.samples.isEmpty {
            let durations = report.samples.map { $0.durationNs }
            let min = durations.min() ?? 0
            let max = durations.max() ?? 0
            let avg = durations.reduce(0, +) / UInt64(durations.count)

            output += "\nStatistics:\n"
            output += "  Min: \(formatDuration(min))\n"
            output += "  Max: \(formatDuration(max))\n"
            output += "  Avg: \(formatDuration(avg))\n"
        }

        return output
    }

    private static func formatBenchError(_ error: BenchError) -> String {
        switch error {
        case .InvalidIterations(let message):
            return "Error (InvalidIterations): \(message)"
        case .UnknownFunction(let message):
            return "Error (UnknownFunction): \(message)"
        case .ExecutionFailed(let message):
            return "Error (ExecutionFailed): \(message)"
        }
    }
}
