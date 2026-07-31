//! Characterizes the v0.1.43 editable/embedded template boundary.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn relative_files(root: &Path) -> BTreeSet<PathBuf> {
    fn walk(root: &Path, current: &Path, files: &mut BTreeSet<PathBuf>) {
        let mut entries = fs::read_dir(current)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", current.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", current.display()));
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, files);
            } else if path.is_file() {
                files.insert(path.strip_prefix(root).expect("descendant").to_path_buf());
            }
        }
    }

    let mut files = BTreeSet::new();
    walk(root, root, &mut files);
    files
}

#[test]
fn template_mirrors_have_only_the_characterized_ios_header_drift() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let editable_root = workspace.join("templates");
    let embedded_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");

    let editable_files = relative_files(&editable_root);
    let embedded_files = relative_files(&embedded_root);
    assert_eq!(
        editable_files, embedded_files,
        "v0.1.43 editable and embedded template file inventories drifted"
    );

    let mismatches = editable_files
        .iter()
        .filter(|relative| {
            fs::read(editable_root.join(relative)).expect("read editable template")
                != fs::read(embedded_root.join(relative)).expect("read embedded template")
        })
        .cloned()
        .collect::<BTreeSet<_>>();

    assert_eq!(
        mismatches,
        [PathBuf::from(
            "ios/BenchRunner/BenchRunner/BenchRunner-Bridging-Header.h.template",
        )]
        .into_iter()
        .collect(),
        "update the template ownership receipt when mirror drift changes"
    );
}

#[test]
fn web_templates_are_synchronized_byte_for_byte() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let editable_root = workspace.join("templates/web");
    let embedded_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates/web");

    let expected = [
        PathBuf::from("index.html"),
        PathBuf::from("runner.js"),
        PathBuf::from("worker.js"),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(relative_files(&editable_root), expected);
    assert_eq!(relative_files(&embedded_root), expected);

    for relative in expected {
        assert_eq!(
            fs::read(editable_root.join(&relative)).expect("read editable web template"),
            fs::read(embedded_root.join(&relative)).expect("read embedded web template"),
            "web template mirror drifted: {}",
            relative.display()
        );
    }
}
