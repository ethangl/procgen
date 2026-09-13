use std::{
    env, fs,
    path::{Path, PathBuf},
};
fn sources(path: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            sources(&p, files);
        } else if p.extension().is_some_and(|e| e == "rs" || e == "wgsl") {
            files.push(p);
        }
    }
}
fn main() {
    let rustc = std::process::Command::new(env::var("RUSTC").unwrap())
        .arg("--version")
        .output()
        .unwrap();
    assert!(rustc.status.success());
    println!(
        "cargo:rustc-env=PILOT_RUSTC={}",
        String::from_utf8(rustc.stdout).unwrap().trim()
    );
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut files = vec![
        root.join("Cargo.toml"),
        root.join("build.rs"),
        root.join("../../Cargo.lock"),
    ];
    for path in [
        "src",
        "../../crates/procgen-core/src",
        "../../crates/procgen-noise/src",
        "../../crates/procgen-cubesphere/src",
    ] {
        sources(&root.join(path), &mut files);
    }
    let mut hash = 0xcbf29ce484222325_u64;
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
        for byte in fs::read(file).unwrap() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    println!("cargo:rustc-env=PILOT_BUILD_ID={hash:016x}");
}
