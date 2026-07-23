const status = document.querySelector("#mobench-status");
const worker = new Worker(new URL("./worker.js", import.meta.url), {
  type: "module",
});
const pending = new Map();
let nextRunId = 1;
let resolveReady;
let rejectReady;

const ready = new Promise((resolve, reject) => {
  resolveReady = resolve;
  rejectReady = reject;
});

worker.addEventListener("message", (event) => {
  const message = event.data ?? {};
  if (message.type === "ready") {
    if (status) status.textContent = "Ready";
    resolveReady();
    return;
  }
  if (message.type === "init-error") {
    const error = new Error(message.error || "Failed to initialize benchmark worker");
    if (status) status.textContent = error.message;
    rejectReady(error);
    return;
  }
  if (message.type !== "result") return;

  const callback = pending.get(message.id);
  if (!callback) return;
  pending.delete(message.id);
  if (message.ok) {
    callback.resolve(message.result);
  } else {
    callback.reject(new Error(message.error || "Benchmark worker failed"));
  }
});

worker.addEventListener("error", (event) => {
  const error = new Error(event.message || "Benchmark worker crashed");
  rejectReady(error);
  for (const callback of pending.values()) callback.reject(error);
  pending.clear();
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

    const id = nextRunId++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      worker.postMessage({ type: "run", id, spec });
    });
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
