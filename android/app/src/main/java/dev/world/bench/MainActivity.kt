package dev.world.bench

import android.os.Bundle
import android.os.Debug
import android.os.Process
import android.os.SystemClock
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import org.json.JSONArray
import org.json.JSONObject
import uniffi.sample_fns.BenchException
import uniffi.sample_fns.BenchReport
import uniffi.sample_fns.BenchSpec
import uniffi.sample_fns.runBenchmark

class MainActivity : AppCompatActivity() {

    companion object {
        private const val DEFAULT_FUNCTION = "sample_fns::fibonacci"
        private const val DEFAULT_ITERATIONS = 20u
        private const val DEFAULT_WARMUP = 3u
        private const val FUNCTION_EXTRA = "bench_function"
        private const val ITERATIONS_EXTRA = "bench_iterations"
        private const val WARMUP_EXTRA = "bench_warmup"
        private const val SPEC_ASSET = "bench_spec.json"

        init {
            System.loadLibrary("sample_fns")
        }
    }

    private data class BenchParams(
        val function: String,
        val iterations: UInt,
        val warmup: UInt,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        val params = resolveBenchParams()
        val display = try {
            val spec = BenchSpec(
                name = params.function,
                iterations = params.iterations,
                warmup = params.warmup
            )
            val report = runBenchmark(spec)
            // Debug: Log first sample's raw nanoseconds
            if (report.samples.isNotEmpty()) {
                android.util.Log.d("MainActivity", "First sample duration_ns: ${report.samples[0].durationNs}")
            }
            logBenchReport(report)
            formatBenchReport(report)
        } catch (e: BenchException.InvalidIterations) {
            "Error: ${e.message}"
        } catch (e: BenchException.UnknownFunction) {
            "Error: ${e.message}"
        } catch (e: BenchException.ExecutionFailed) {
            "Error: ${e.message}"
        } catch (e: Exception) {
            "Unexpected error: ${e.message}"
        }

        findViewById<TextView>(R.id.result_text)?.text = display

        // Keep the report on screen for at least 5 seconds so BrowserStack video captures it
        android.util.Log.i("BenchRunner", "Displaying results for 5 seconds for video capture...")
        Thread.sleep(5000)
        android.util.Log.i("BenchRunner", "Display hold complete")
    }

    /**
     * Formats a duration in nanoseconds to a human-readable string.
     * Uses milliseconds (ms) by default, switches to seconds (s) if >= 1000ms.
     */
    private fun formatDuration(ns: Long): String {
        val ms = ns.toDouble() / 1_000_000.0
        return if (ms >= 1000.0) {
            val secs = ms / 1000.0
            String.format("%.3fs", secs)
        } else {
            String.format("%.3fms", ms)
        }
    }

    private fun formatBenchReport(report: BenchReport): String = buildString {
        appendLine("=== Benchmark Results ===")
        appendLine()
        appendLine("Function: ${report.spec.name}")
        appendLine("Iterations: ${report.spec.iterations}")
        appendLine("Warmup: ${report.spec.warmup}")
        appendLine()
        appendLine("Samples (${report.samples.size}):")
        report.samples.forEachIndexed { index, sample ->
            appendLine("  ${index + 1}. ${formatDuration(sample.durationNs.toLong())}")
        }

        if (report.samples.isNotEmpty()) {
            val durations = report.samples.map { it.durationNs.toLong() }
            val min = durations.minOrNull() ?: 0L
            val max = durations.maxOrNull() ?: 0L
            val avg = durations.sum().toDouble() / durations.size.toDouble()
            appendLine()
            appendLine("Statistics:")
            appendLine("  Min: ${formatDuration(min)}")
            appendLine("  Max: ${formatDuration(max)}")
            appendLine("  Avg: ${formatDuration(avg.toLong())}")
        }
    }

    private fun logBenchReport(report: BenchReport) {
        val json = JSONObject()
        val spec = JSONObject()
        spec.put("name", report.spec.name)
        spec.put("iterations", report.spec.iterations.toInt())
        spec.put("warmup", report.spec.warmup.toInt())
        json.put("spec", spec)

        val samples = report.samples.map { it.durationNs.toLong() }
        val sampleArray = JSONArray()
        samples.forEach { sampleArray.put(it) }
        json.put("samples_ns", sampleArray)

        if (samples.isNotEmpty()) {
            val min = samples.minOrNull() ?: 0L
            val max = samples.maxOrNull() ?: 0L
            val avg = samples.sum().toDouble() / samples.size.toDouble()
            val stats = JSONObject()
            stats.put("min_ns", min)
            stats.put("max_ns", max)
            stats.put("avg_ns", avg.toDouble())
            json.put("stats", stats)
        }

        val memInfo = Debug.MemoryInfo()
        Debug.getMemoryInfo(memInfo)
        val resources = JSONObject()
        resources.put("elapsed_cpu_ms", Process.getElapsedCpuTime())
        resources.put("uptime_ms", SystemClock.elapsedRealtime())
        resources.put("total_pss_kb", memInfo.totalPss)
        resources.put("private_dirty_kb", memInfo.totalPrivateDirty)
        resources.put("native_heap_kb", Debug.getNativeHeapAllocatedSize() / 1024)
        val usedHeap = Runtime.getRuntime().totalMemory() - Runtime.getRuntime().freeMemory()
        resources.put("java_heap_kb", usedHeap / 1024)
        json.put("resources", resources)

        android.util.Log.i("BenchRunner", "BENCH_JSON ${json}")
    }

    private fun resolveBenchParams(): BenchParams {
        val defaults = loadBenchParamsFromAssets() ?: BenchParams(
            DEFAULT_FUNCTION,
            DEFAULT_ITERATIONS,
            DEFAULT_WARMUP
        )
        val fn = intent?.getStringExtra(FUNCTION_EXTRA)
            ?.takeUnless { it.isBlank() }
            ?: defaults.function
        val iterations = intent?.getIntExtra(ITERATIONS_EXTRA, defaults.iterations.toInt())?.toUInt()
            ?: defaults.iterations
        val warmup = intent?.getIntExtra(WARMUP_EXTRA, defaults.warmup.toInt())?.toUInt()
            ?: defaults.warmup
        return BenchParams(fn, iterations, warmup)
    }

    private fun loadBenchParamsFromAssets(): BenchParams? {
        return try {
            val raw = assets.open(SPEC_ASSET).bufferedReader().use { it.readText() }
            if (raw.isBlank()) {
                null
            } else {
                val json = JSONObject(raw)
                BenchParams(
                    json.optString("function", DEFAULT_FUNCTION),
                    json.optInt("iterations", DEFAULT_ITERATIONS.toInt()).toUInt(),
                    json.optInt("warmup", DEFAULT_WARMUP.toInt()).toUInt(),
                )
            }
        } catch (_: Exception) {
            null
        }
    }
}
