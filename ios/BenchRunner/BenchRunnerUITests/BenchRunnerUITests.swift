import Foundation
import XCTest

final class BenchRunnerUITests: XCTestCase {

    /// Maximum time to wait for benchmark completion (5 minutes for long benchmarks)
    private let defaultBenchmarkTimeout: TimeInterval = 300.0
    private let expectedBenchmarkFunction = "sample_fns::fibonacci"

    private var benchmarkTimeout: TimeInterval {
        if let configuredTimeout =
            ProcessInfo.processInfo.environment["MOBENCH_IOS_BENCHMARK_TIMEOUT_SECS"],
            let parsedTimeout = TimeInterval(configuredTimeout),
            parsedTimeout > 0
        {
            return parsedTimeout
        }

        return defaultBenchmarkTimeout
    }

    private func validateBenchmarkReport(_ jsonString: String) {
        XCTAssertFalse(jsonString.isEmpty, "Benchmark report JSON should not be empty")

        guard let data = jsonString.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            XCTFail("Benchmark report should be a valid JSON object: \(jsonString)")
            return
        }

        XCTAssertNil(payload["error"], "Benchmark report should not be an error payload: \(jsonString)")

        let spec = payload["spec"] as? [String: Any]
        let reportedFunction = payload["function"] as? String ?? spec?["name"] as? String
        XCTAssertEqual(
            reportedFunction,
            expectedBenchmarkFunction,
            "Benchmark report function must match the requested function"
        )

        let samples = payload["samples_ns"] as? [Any] ?? payload["samples"] as? [Any]
        XCTAssertNotNil(samples, "Benchmark report should include measured samples")
        XCTAssertFalse(samples?.isEmpty ?? true, "Benchmark report samples should not be empty")

        if let schemaVersion = payload["schema_version"] {
            XCTAssertTrue(
                schemaVersion is String || schemaVersion is NSNumber,
                "Benchmark report schema_version should be a string or number"
            )
        }
    }

    func testLaunchAndCaptureBenchmarkReport() {
        let app = XCUIApplication()
        app.launch()

        // Wait for the benchmark to actually COMPLETE, not just start
        // The app sets a "benchmarkCompleted" element when done
        let completedIndicator = app.staticTexts["benchmarkCompleted"]
        let completed = completedIndicator.waitForExistence(timeout: benchmarkTimeout)
        XCTAssertTrue(completed, "Benchmark should complete within \(benchmarkTimeout) seconds")

        // Wait 5 seconds so BrowserStack video captures the results
        // This delay is critical for video evidence of benchmark completion
        Thread.sleep(forTimeInterval: 5.0)

        // Extract the benchmark report JSON from the hidden element
        let reportElement = app.staticTexts["benchmarkReportJSON"]
        XCTAssertTrue(reportElement.exists, "Benchmark report JSON element should exist after completion")

        // The JSON is stored in the element's label property
        let jsonString = reportElement.label

        // Log with markers that mobench fetch can parse from instrumentation logs
        // Using NSLog to ensure it goes to device logs that BrowserStack captures
        NSLog("BENCH_REPORT_JSON_START")
        NSLog("%@", jsonString)
        NSLog("BENCH_REPORT_JSON_END")

        // Also print to stdout for local testing visibility
        print("BENCH_REPORT_JSON_START")
        print(jsonString)
        print("BENCH_REPORT_JSON_END")

        validateBenchmarkReport(jsonString)
    }

    // Keep the old test name for backward compatibility
    func testLaunchShowsBenchmarkReport() {
        testLaunchAndCaptureBenchmarkReport()
    }
}
