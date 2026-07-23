import init, { runBenchmarkJson } from "./mobench_web.js";

const ready = init();
ready
  .then(() => self.postMessage({ type: "ready" }))
  .catch((error) => {
    self.postMessage({ type: "init-error", error: String(error) });
  });

self.addEventListener("message", async (event) => {
  const message = event.data ?? {};
  if (message.type !== "run") return;

  try {
    await ready;
    const result = JSON.parse(runBenchmarkJson(JSON.stringify(message.spec)));
    self.postMessage({ type: "result", id: message.id, ok: true, result });
  } catch (error) {
    self.postMessage({
      type: "result",
      id: message.id,
      ok: false,
      error: error && error.message ? String(error.message) : String(error),
    });
  }
});
