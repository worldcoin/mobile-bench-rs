package dev.world.bench

import android.os.Bundle
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
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
            System.loadLibrary("uniffi_sample_fns")
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
            val durationUs = sample.durationNs.toDouble() / 1_000.0
            appendLine("  ${index + 1}. ${String.format("%.3f", durationUs)} μs (${sample.durationNs} ns)")
        }

        if (report.samples.isNotEmpty()) {
            val durations = report.samples.map { it.durationNs.toDouble() / 1_000.0 }
            val min = durations.minOrNull() ?: 0.0
            val max = durations.maxOrNull() ?: 0.0
            val avg = durations.average()
            appendLine()
            appendLine("Statistics:")
            appendLine("  Min: ${String.format("%.3f", min)} μs")
            appendLine("  Max: ${String.format("%.3f", max)} μs")
            appendLine("  Avg: ${String.format("%.3f", avg)} μs")
        }
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
