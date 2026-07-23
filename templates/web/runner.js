import init, { runBenchmarkJson } from "./mobench_web.js";

const status = document.querySelector("#mobench-status");
const ready = init()
  .then(() => {
    if (status) status.textContent = "Ready";
  })
  .catch((error) => {
    if (status) status.textContent = `Failed to initialize: ${String(error)}`;
    throw error;
  });

window.mobench = Object.freeze({
  ready,
  async run(input) {
    await ready;
    const spec = {
      name: input.name ?? input.function,
      iterations: input.iterations,
      warmup: input.warmup,
    };
    if (typeof spec.name !== "string" || spec.name.length === 0) {
      throw new TypeError("mobench web benchmark requires a function name");
    }
    return JSON.parse(runBenchmarkJson(JSON.stringify(spec)));
  },
});

const form = document.querySelector("#mobench-form");
const result = document.querySelector("#mobench-result");
form?.addEventListener("submit", async (event) => {
  event.preventDefault();
  const data = new FormData(form);
  if (result) {
    result.hidden = false;
    result.textContent = "Running…";
  }
  try {
    const report = await window.mobench.run({
      function: String(data.get("function") ?? ""),
      iterations: Number(data.get("iterations")),
      warmup: Number(data.get("warmup")),
    });
    if (result) result.textContent = JSON.stringify(report, null, 2);
  } catch (error) {
    if (result) result.textContent = `Benchmark failed: ${String(error)}`;
  }
});
