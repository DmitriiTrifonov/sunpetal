//! The recompiler's self-test on the generated C: ndskit's own instruction test (no ROM
//! needed) and, when the vectors exist, the game's ARM9 functions (docs/ABI.md).
#![cfg(has_generated)]
use std::path::PathBuf;

use sunpetal_runtime::selftest::run_vectors;

fn gen_dir() -> PathBuf {
    std::env::var_os("SUNPETAL_GEN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../generated/recomp"))
}

fn replay(vectors: PathBuf, required: bool, interp: bool) {
    if !vectors.exists() {
        assert!(!required, "missing {}", vectors.display());
        eprintln!("skipped: {} does not exist", vectors.display());
        return;
    }
    let rep = run_vectors(&vectors, interp).unwrap_or_else(|e| panic!("{}: {}", vectors.display(), e));
    for m in &rep.messages {
        eprintln!("{}", m);
    }
    eprintln!("{}: {} functions, {} vectors, {} failures", vectors.display(), rep.functions, rep.vectors, rep.failures);
    assert!(rep.vectors > 0, "no vectors in {}", vectors.display());
    assert_eq!(rep.failures, 0);
}

#[test]
fn optest_arm9() {
    replay(gen_dir().join("selftest/optest9.txt"), true, false);
}

#[test]
fn optest_arm7() {
    replay(gen_dir().join("selftest/optest7.txt"), true, false);
}

#[test]
fn game_arm9() {
    let v = std::env::var_os("SUNPETAL_ARM9_VECTORS")
        .map(PathBuf::from)
        .unwrap_or_else(|| gen_dir().join("selftest/arm9_vectors.txt"));
    replay(v, false, false);
}

// The same vectors through the interpreter alone: it is checked against armemu as the
// recompiler is.
#[test]
fn interp_optest_arm9() {
    replay(gen_dir().join("selftest/optest9.txt"), true, true);
}

#[test]
fn interp_optest_arm7() {
    replay(gen_dir().join("selftest/optest7.txt"), true, true);
}

#[test]
fn interp_game_arm9() {
    let v = std::env::var_os("SUNPETAL_ARM9_VECTORS")
        .map(PathBuf::from)
        .unwrap_or_else(|| gen_dir().join("selftest/arm9_vectors.txt"));
    replay(v, false, true);
}

// The overlays' functions, with the overlay loaded: calls into its area go through the runtime's
// choice among the overlay modules (the crc of each function's bytes in memory).
fn overlay_vectors() -> Vec<PathBuf> {
    let dir = gen_dir().join("selftest");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    v.retain(|p| {
        p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with("arm9_ov") && n.ends_with("_vectors.txt"))
    });
    v.sort();
    v
}

#[test]
fn overlays_arm9() {
    let files = overlay_vectors();
    if files.is_empty() {
        eprintln!("skipped: no overlay vectors (docs/ABI.md)");
    }
    for f in files {
        replay(f, true, false);
    }
}

#[test]
fn interp_overlays_arm9() {
    for f in overlay_vectors() {
        replay(f, true, true);
    }
}
