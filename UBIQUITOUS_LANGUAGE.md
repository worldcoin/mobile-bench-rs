# Ubiquitous Language

## Benchmarking

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Benchmark Function** | A Rust function registered for timed execution by mobench. | Test function, bench, routine |
| **Benchmark Crate** | The Rust crate that contains benchmark functions and exports them to generated mobile runners. | Bench mobile crate, workload crate |
| **Benchmark Spec** | The requested benchmark function, iterations, warmup, target, and device selection for one run. | Run config, request payload, spec JSON |
| **Benchmark Report** | The raw timing result emitted by the benchmark runtime for one benchmark function execution context. | Bench output, timing blob, result JSON |
| **Run Summary** | The normalized benchmark output contract written as `summary.json`, `summary.md`, and `results.csv`. | Summary report, CI summary, report |
| **Resource Usage** | CPU and memory measurements attached to a benchmark result. | Metrics, performance data |
| **Regression Finding** | A comparison result where a candidate run exceeds the allowed slowdown threshold. | Failure, performance issue |

## Mobile Execution

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Mobile Target** | The platform family where a benchmark is built or run: Android or iOS. | Platform, target platform |
| **Device Matrix** | A YAML-defined inventory of devices and tags used for deterministic device selection. | Device list, matrix file |
| **Device Profile** | A named selection rule that resolves to one or more devices from built-in defaults or a device matrix. | Device tag, profile, tier |
| **Mobile Artifact** | A build output that can be installed or uploaded for mobile benchmark execution. | App, package, binary |
| **Generated Mobile Runner** | The generated Android or iOS app that loads a benchmark spec and invokes benchmark functions through UniFFI. | Harness app, runner app, scaffold |
| **BrowserStack Run** | A remote mobile execution scheduled through BrowserStack App Automate. | Remote run, cloud run |
| **Recovered Payload** | A benchmark report extracted from BrowserStack logs or fetched session artifacts. | Extracted result, fetched result |

## Project Resolution

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Project Layout** | The resolved project root, benchmark crate, output directory, library name, and configured defaults for a command. | Layout, workspace resolution |
| **mobench Config** | The `mobench.toml` file that defines project, Android, iOS, BrowserStack, and benchmark defaults. | Config file, project config |
| **Run Request** | The programmatic input for running a mobench benchmark flow from library code. | Request, invocation |

## Profiling

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Profile Session** | A profiling lifecycle that plans, captures, processes, and persists native and semantic profiling artifacts for one benchmark function. | Profile run, profiling run |
| **Capture Plan** | The planned artifact contract and metadata for a profile session before execution. | Plan, artifact plan |
| **Native Capture** | Raw platform profiler output collected from Android `simpleperf` or iOS `sample`. | Native profile, profiler output |
| **Semantic Profile** | Phase and harness timeline data emitted by the benchmark runtime. | Phase profile, trace data |
| **Symbolization** | The conversion of native stack addresses into human-readable frames and source locations. | Stack resolution, addr2line |
| **Flamegraph Viewer** | The generated HTML/SVG profiler view that presents full-process and benchmark-focused stacks. | Viewer, flamegraph HTML |
| **Profile Diff** | A comparison of two profile sessions rendered as a differential viewer bundle. | Differential profile, diff bundle |

## Build And Generation

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Artifact Build** | The lifecycle that compiles Rust libraries, refreshes generated mobile runners, embeds metadata, and produces mobile artifacts. | Build flow, mobile build |
| **Generated Project** | The on-disk Android or iOS project produced from SDK templates under `target/mobench`. | Scaffold, generated app |
| **Template Refresh** | The regeneration step that keeps generated mobile runners aligned with embedded SDK templates. | Template sync, scaffold refresh |
| **Package Step** | The iOS-specific step that produces an IPA or XCUITest zip after an artifact build. | Packaging, archive step |

## Relationships

- A **Benchmark Crate** contains one or more **Benchmark Functions**.
- A **Benchmark Spec** names exactly one **Benchmark Function** for a single target-specific execution.
- A **Run Summary** is produced from one or more **Benchmark Reports** and may include **Resource Usage**.
- A **Device Profile** resolves through a **Device Matrix** or built-in defaults into concrete devices.
- A **BrowserStack Run** produces **Recovered Payloads** that become **Benchmark Reports**.
- A **Project Layout** resolves the **Benchmark Crate**, **mobench Config**, and output paths used by an **Artifact Build**.
- A **Profile Session** starts from a **Capture Plan** and may produce a **Native Capture**, **Semantic Profile**, **Flamegraph Viewer**, and **Profile Diff**.
- An **Artifact Build** produces **Mobile Artifacts** from a **Generated Project** and **Benchmark Crate**.

## Example Dialogue

> **Dev:** "When I run a mobile benchmark, should the command take a **Benchmark Function** or a **Benchmark Report**?"
> **Domain expert:** "It takes a **Benchmark Spec** that names the **Benchmark Function**; the **Benchmark Report** is emitted after execution."
> **Dev:** "If the run happens on BrowserStack, is the output still a **Run Summary**?"
> **Domain expert:** "Yes. The **BrowserStack Run** yields **Recovered Payloads**, and those are normalized into the same **Run Summary** contract."
> **Dev:** "For profiling, do we compare the **Native Capture** directly?"
> **Domain expert:** "No. A **Profile Session** processes the **Native Capture** into a **Flamegraph Viewer**, and a **Profile Diff** compares two processed profile sessions."

## Flagged Ambiguities

- "Report" has been used for raw benchmark output, normalized summaries, Markdown output, and profile summaries. Use **Benchmark Report** for raw runtime output and **Run Summary** for normalized CI/reporting output.
- "Profile" has been used for device selection and native profiling. Use **Device Profile** for device selection and **Profile Session** for profiling lifecycle work.
- "Artifact" has been used for APKs/IPAs, BrowserStack fetched files, and profile outputs. Use **Mobile Artifact**, **Recovered Payload**, and **Native Capture** or **Flamegraph Viewer** depending on the lifecycle.
- "Config" has been used for `mobench.toml`, run requests, and benchmark specs. Use **mobench Config**, **Run Request**, and **Benchmark Spec** for those separate concepts.
