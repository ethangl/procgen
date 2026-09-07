use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

fn main() {
    let workspace = Path::new("../..");
    let mut files = Vec::new();
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "apps/viewer/src/model.rs",
        "crates/procgen-core",
        "crates/procgen-sphere",
        "crates/procgen-sphere-mesh",
        "crates/procgen-planet",
        "crates/procgen-tectonics",
        "crates/procgen-geology",
        "crates/procgen-climate",
    ] {
        collect_files(&workspace.join(path), &mut files);
    }
    files.sort();

    let mut hasher = DefaultHasher::new();
    for path in files {
        println!("cargo::rerun-if-changed={}", path.display());
        path.strip_prefix(workspace)
            .unwrap_or(&path)
            .to_string_lossy()
            .hash(&mut hasher);
        fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
            .hash(&mut hasher);
    }
    println!(
        "cargo::rustc-env=PROCGEN_GENERATOR_BUILD_ID={:016x}",
        hasher.finish()
    );
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        files.push(path.to_owned());
        return;
    }
    let mut entries: Vec<_> = fs::read_dir(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .map(|entry| entry.expect("directory entry must be readable").path())
        .collect();
    entries.sort();
    for entry in entries {
        collect_files(&entry, files);
    }
}
