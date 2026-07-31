import { Proof, initProveKit } from "@worldcoin/provekit";

const FUNCTION_NAME = "provekit_npm::oprf_taceo_prove_and_verify";
const status = document.querySelector("#mobench-status");

async function fetchBytes(path) {
  const response = await fetch(path);
  if (!response.ok) throw new Error(`failed to fetch ${path}: ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

const fixture = Promise.all([
  initProveKit({ threads: false }),
  fetchBytes("/artifacts/oprf_taceo.pkp"),
  fetchBytes("/artifacts/oprf_taceo.pkv"),
  fetch("/artifacts/inputs.json").then(async (response) => {
    if (!response.ok) throw new Error(`failed to fetch inputs: ${response.status}`);
    return response.json();
  }),
]).then(async ([runtime, proverBytes, verifierBytes, inputs]) => {
  const prover = await runtime.loadProver(proverBytes);
  const verifier = await runtime.loadVerifier(verifierBytes);
  return { prover, verifier, inputs };
});

async function proveAndVerify() {
  const { prover, verifier, inputs } = await fixture;
  const proof = await prover.prove(inputs);
  if (!(await verifier.verify(proof))) throw new Error("ProveKit verifier rejected its proof");

  const tampered = proof.bytes.slice();
  tampered[tampered.length - 1] ^= 1;
  try {
    if (await verifier.verify(Proof.fromBytes(tampered))) {
      throw new Error("ProveKit verifier accepted a tampered proof");
    }
  } catch (error) {
    if (error?.message === "ProveKit verifier accepted a tampered proof") throw error;
  }
}

function assertSpec(spec) {
  if (spec?.name !== FUNCTION_NAME) {
    throw new Error(`expected ${FUNCTION_NAME}, received ${String(spec?.name)}`);
  }
  if (!Number.isInteger(spec.iterations) || spec.iterations < 1) {
    throw new Error("iterations must be a positive integer");
  }
  if (!Number.isInteger(spec.warmup) || spec.warmup < 0) {
    throw new Error("warmup must be a non-negative integer");
  }
}

window.mobench = Object.freeze({
  async run(spec) {
    assertSpec(spec);
    for (let index = 0; index < spec.warmup; index += 1) await proveAndVerify();

    const samples = [];
    for (let index = 0; index < spec.iterations; index += 1) {
      const started = performance.now();
      await proveAndVerify();
      samples.push({ duration_ns: Math.max(1, Math.round((performance.now() - started) * 1_000_000)) });
    }
    return { spec, samples };
  },
});

fixture.then(
  () => { if (status) status.textContent = "Ready"; },
  (error) => { if (status) status.textContent = `Failed: ${String(error)}`; },
);
