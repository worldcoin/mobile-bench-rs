//! Browser-only adapter for ProveKit's complete passport age-check benchmark.
//!
//! The release workflow copies this file into the pinned ProveKit checkout.
//! It uses the same compiled Noir program and inputs as `bench-mobile`, but
//! consumes a witness generated during the secretless preparation job because
//! ProveKit intentionally excludes ACVM witness generation from wasm32 builds.

use {
    acir::native_types::WitnessMap,
    mobench_sdk::{benchmark, profile_phase},
    noirc_artifacts::program::ProgramArtifact,
    provekit_common::{NoirElement, NoirProofScheme, Prover},
    provekit_prover::Prove,
    provekit_r1cs_compiler::NoirProofSchemeBuilder,
    std::hint::black_box,
};

const COMPLETE_AGE_CHECK_PROGRAM: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/bench_mobile_fixtures/complete_age_check.json"
));
const COMPLETE_AGE_CHECK_WITNESS: &[u8] =
    include_bytes!("../generated/complete_age_check.witness.postcard");

struct PreparedAgeCheck {
    prover: Prover,
    witness: WitnessMap<NoirElement>,
}

fn setup_complete_age_check() -> PreparedAgeCheck {
    let program: ProgramArtifact = serde_json::from_str(COMPLETE_AGE_CHECK_PROGRAM)
        .expect("deserialize complete age-check Noir artifact");
    let scheme =
        NoirProofScheme::from_program(program).expect("prepare complete age-check proof scheme");
    let witness = postcard::from_bytes(COMPLETE_AGE_CHECK_WITNESS)
        .expect("deserialize complete age-check witness");

    PreparedAgeCheck {
        prover: Prover::from_noir_proof_scheme(scheme),
        witness,
    }
}

#[benchmark(setup = setup_complete_age_check, per_iteration)]
pub fn bench_passport_complete_age_check_prove(prepared: PreparedAgeCheck) {
    let proof = profile_phase("prove", || {
        prepared
            .prover
            .prove_with_witness(prepared.witness)
            .expect("prove complete age-check fixture")
    });
    black_box(proof);
}
