// Builds the recompiled C (the output of `python -m ndskit.recomp`, see docs/ABI.md)
// into a static library and links it. The generated code comes from the user's own
// ROM and is never in the repository: it is looked for in $SUNPETAL_GEN_DIR, else in
// <workspace>/generated/recomp. Without it the crate builds with no modules.
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_generated)");
    println!("cargo:rerun-if-env-changed=SUNPETAL_GEN_DIR");
    println!("cargo:rerun-if-changed=include/sunpetal_abi.h");

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dir = env::var_os("SUNPETAL_GEN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../generated/recomp"));
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut sources: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map_or(false, |x| x == "c"))
            .collect(),
        Err(_) => Vec::new(),
    };
    if !sources.iter().any(|p| p.file_name().map_or(false, |n| n == "modules.c")) {
        println!(
            "cargo:warning=no generated C in {} (run the recompiler, docs/ABI.md): building without modules",
            dir.display()
        );
        return;
    }
    sources.sort();
    for s in &sources {
        println!("cargo:rerun-if-changed={}", s.display());
    }
    cc::Build::new()
        .files(&sources)
        .include(manifest.join("include"))
        .include(&dir)
        .std("gnu11")
        .opt_level(2)
        .warnings(false)
        .compile("sunpetal_generated");
    println!("cargo:rustc-cfg=has_generated");
}
