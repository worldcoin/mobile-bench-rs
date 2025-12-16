package dev.world.bench

import android.os.Bundle
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity

class MainActivity : AppCompatActivity() {

    companion object {
        private const val DEFAULT_FUNCTION = "sample_fns::fibonacci"
        private const val DEFAULT_ITERATIONS = 20
        private const val DEFAULT_WARMUP = 3
        private const val FUNCTION_EXTRA = "bench_function"
        private const val ITERATIONS_EXTRA = "bench_iterations"
        private const val WARMUP_EXTRA = "bench_warmup"

        init {
            System.loadLibrary("sample_fns")
        }
    }

    private data class BenchSpec(
        val function: String,
        val iterations: Int,
        val warmup: Int,
    )

    private external fun runBench(function: String, iterations: Int, warmup: Int): String

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        val spec = resolveBenchSpec()
        val report = runBench(spec.function, spec.iterations, spec.warmup)
        val display = buildString {
            appendLine("Function: ${spec.function}")
            appendLine("Iterations: ${spec.iterations}")
            appendLine("Warmup: ${spec.warmup}")
            appendLine()
            append(report)
        }

        findViewById<TextView>(R.id.result_text)?.text = display
    }

    private fun resolveBenchSpec(): BenchSpec {
        val fn = intent?.getStringExtra(FUNCTION_EXTRA)
            ?.takeUnless { it.isBlank() }
            ?: DEFAULT_FUNCTION
        val iterations = intent?.getIntExtra(ITERATIONS_EXTRA, DEFAULT_ITERATIONS)
            ?: DEFAULT_ITERATIONS
        val warmup = intent?.getIntExtra(WARMUP_EXTRA, DEFAULT_WARMUP) ?: DEFAULT_WARMUP
        return BenchSpec(fn, iterations, warmup)
    }
}
