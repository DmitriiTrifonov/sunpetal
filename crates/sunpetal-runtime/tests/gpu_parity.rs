//! Этап 0 of the GPU backend plan (`docs/GPU-BACKEND.md`): a dump of a real frame must re-render
//! with the software renderer byte-exact. Later the GPU renderer is scored against the same
//! dumps. The dumps are not in the repository: `SUNPETAL_JOB_DUMPS` points at a directory of
//! `*.spjob` files made with `SUNPETAL_JOB_DUMP` (+ `SUNPETAL_JOB_FRAMES`).

use std::path::PathBuf;
use sunpetal_runtime::nds::parity_check;

#[test]
fn software_parity_on_real_frames() {
    let Some(dir) = std::env::var_os("SUNPETAL_JOB_DUMPS").map(PathBuf::from) else {
        println!("SUNPETAL_JOB_DUMPS is not set: no dumps to check");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("dump dir")
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension()?.to_str()? == "spjob").then_some(p)
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .spjob files in {}", dir.display());
    for f in files {
        let rep = parity_check(&f).unwrap_or_else(|x| panic!("{x}"));
        println!(
            "{} (frame {}): exact {:.4}% ({} of {}), mean |d| {:.3}/{:.3}/{:.3}, over2 {:.4}%, fx off {}, notes {}",
            rep.path,
            rep.vblank,
            100.0 * rep.exact as f64 / rep.pixels.max(1) as f64,
            rep.exact,
            rep.pixels,
            rep.mean_abs[0],
            rep.mean_abs[1],
            rep.mean_abs[2],
            100.0 * rep.over2 as f64 / rep.pixels.max(1) as f64,
            rep.fx_off,
            if rep.notes_equal { "equal" } else { "DIFFER" }
        );
        assert_eq!(rep.exact, rep.pixels, "{}: not byte-exact", rep.path);
        assert!(rep.notes_equal, "{}: notes differ", rep.path);
    }
}
