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

/// Result of running a benchmark, containing both display text and JSON report
struct BenchmarkResult {
    let displayText: String
    let jsonReport: String
}

enum BenchRunnerFFI {
    static func runCurrentBenchmark() async -> BenchmarkResult {
        let params = BenchParams.resolved()
        return run(params: params)
    }

    static func run(params: BenchParams) -> BenchmarkResult {
        let spec = BenchSpec(
            name: params.function,
            iterations: params.iterations,
            warmup: params.warmup
        )

        do {
            let report = try runBenchmark(spec: spec)
            let displayText = formatBenchReport(report)
            let jsonReport = generateJSONReport(report)
            return BenchmarkResult(displayText: displayText, jsonReport: jsonReport)
        } catch let error as BenchError {
            let errorText = formatBenchError(error)
            let errorJSON = generateErrorJSON(error)
            return BenchmarkResult(displayText: errorText, jsonReport: errorJSON)
        } catch {
            let errorText = "Unexpected error: \(error.localizedDescription)"
            let errorJSON = "{\"error\": \"Unexpected error: \(error.localizedDescription)\"}"
            return BenchmarkResult(displayText: errorText, jsonReport: errorJSON)
        }
    }

    /// Generates a JSON report matching the Android BENCH_JSON format for consistency
    private static func generateJSONReport(_ report: BenchReport) -> String {
        var json: [String: Any] = [:]

        // Spec section
        let specDict: [String: Any] = [
            "name": report.spec.name,
            "iterations": report.spec.iterations,
            "warmup": report.spec.warmup
        ]
        json["spec"] = specDict

        // Function name at top level (for compatibility with existing parsers)
        json["function"] = report.spec.name

        // Samples as array of duration_ns values
        let samplesNs = report.samples.map { $0.durationNs }
        json["samples_ns"] = samplesNs

        // Also include samples in object format for compatibility
        let samplesArray = report.samples.map { ["duration_ns": $0.durationNs] }
        json["samples"] = samplesArray

        // Statistics
        if !report.samples.isEmpty {
            let durations = report.samples.map { $0.durationNs }
            let minNs = durations.min() ?? 0
            let maxNs = durations.max() ?? 0
            let sumNs = durations.reduce(0, +)
            let avgNs = Double(sumNs) / Double(durations.count)

            // Compute median
            let sorted = durations.sorted()
            let medianNs: UInt64
            if sorted.count % 2 == 0 {
                medianNs = (sorted[sorted.count / 2 - 1] + sorted[sorted.count / 2]) / 2
            } else {
                medianNs = sorted[sorted.count / 2]
            }

            let stats: [String: Any] = [
                "min_ns": minNs,
                "max_ns": maxNs,
                "avg_ns": avgNs,
                "mean_ns": UInt64(avgNs),
                "median_ns": medianNs
            ]
            json["stats"] = stats

            // Also add top-level stats for compatibility with different parsers
            json["mean_ns"] = UInt64(avgNs)
            json["min_ns"] = minNs
            json["max_ns"] = maxNs
        }

        // Resource metrics (iOS-specific)
        let resources: [String: Any] = [
            "platform": "ios",
            "timestamp_ms": Int64(Date().timeIntervalSince1970 * 1000)
        ]
        json["resources"] = resources

        // Serialize to JSON string
        do {
            let data = try JSONSerialization.data(withJSONObject: json, options: [.sortedKeys])
            return String(data: data, encoding: .utf8) ?? "{}"
        } catch {
            print("[BenchRunner] ERROR: Failed to serialize JSON report: \(error)")
            return "{}"
        }
    }

    /// Generates a JSON error report
    private static func generateErrorJSON(_ error: BenchError) -> String {
        let errorDict: [String: Any] = [
            "error": true,
            "message": error.localizedDescription
        ]

        do {
            let data = try JSONSerialization.data(withJSONObject: errorDict, options: [.sortedKeys])
            return String(data: data, encoding: .utf8) ?? "{\"error\": true}"
        } catch {
            return "{\"error\": true, \"message\": \"Failed to serialize error\"}"
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
