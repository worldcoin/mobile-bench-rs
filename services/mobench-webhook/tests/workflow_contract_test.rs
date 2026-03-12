use std::{fs, path::PathBuf};

use serde_yaml::Value;

fn workflow_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".github")
        .join("workflows")
        .join(name)
}

fn load_workflow(name: &str) -> Value {
    let path = workflow_path(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_yaml::from_str(&raw)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

fn job<'a>(workflow: &'a Value, name: &str) -> &'a Value {
    &workflow["jobs"][name]
}

fn install_mobench_scripts(workflow: &Value) -> Vec<&str> {
    workflow["jobs"]
        .as_mapping()
        .unwrap()
        .values()
        .filter_map(|job| job["steps"].as_sequence())
        .flat_map(|steps| steps.iter())
        .filter(|step| step["name"].as_str() == Some("Install mobench"))
        .map(|step| {
            step["run"]
                .as_str()
                .expect("Install mobench step should define a shell script")
        })
        .collect()
}

#[test]
fn mobile_bench_workflow_keeps_browserstack_secret_gate() {
    let workflow = load_workflow("mobile-bench.yml");
    let condition = job(&workflow, "browserstack")["if"]
        .as_str()
        .expect("browserstack job should define an if condition");

    assert!(
        condition.contains("secrets.BROWSERSTACK_USERNAME")
            && condition.contains("secrets.BROWSERSTACK_ACCESS_KEY")
            && condition.contains("!="),
        "browserstack job should skip cleanly when BrowserStack secrets are absent, got: {condition}"
    );
}

#[test]
fn reusable_workflow_install_handles_tags_and_shas() {
    let workflow = load_workflow("reusable-bench.yml");
    let scripts = install_mobench_scripts(&workflow);

    assert_eq!(scripts.len(), 3, "expected install step in each reusable job");
    for script in scripts {
        assert!(
            script.contains("refs/tags/"),
            "install script should recognize tag refs, got: {script}"
        );
        assert!(
            script.contains("--tag"),
            "install script should install tag refs with --tag, got: {script}"
        );
        assert!(
            script.contains("--rev"),
            "install script should install SHA refs with --rev, got: {script}"
        );
        assert!(
            script.contains("--branch"),
            "install script should still support branch refs, got: {script}"
        );
    }
}
