//! The parity harness of the GPU backend plan (`docs/GPU-BACKEND.md`). Этап 0: a dump of a real
//! frame must re-render with the software renderer byte-exact. Этап 1: the GPU renderer
//! (`nds::gpu_parity_check`, feature `gpu`) is scored against the software renderer on the same
//! filtered job, within the stage's tolerances. The dumps are not in the repository:
//! `SUNPETAL_JOB_DUMPS` points at a directory of `*.spjob` files made with `SUNPETAL_JOB_DUMP`
//! (+ `SUNPETAL_JOB_FRAMES`); the GPU test writes its difference maps into `SUNPETAL_JOB_DIFFS`.

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

/// Этап 4 acceptance (`docs/GPU-BACKEND.md`): the full picture (opaque, translucent and shadow
/// polygons) within the tolerances — mean |d| per channel at most 1/63, fewer than 0.5% of pixels
/// off by more than 2 steps, and the difference map empty along the polygon edges.
#[cfg(feature = "gpu")]
#[test]
fn gpu_parity_on_real_frames() {
    use sunpetal_runtime::nds::gpu_parity_check;
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
    let mut any = false;
    let mut opaque = 0usize;
    for f in files {
        let gp = match gpu_parity_check(&f) {
            Err(e) if e.contains("no GPU") => {
                println!("{}: skipped ({e})", f.display());
                continue;
            }
            Err(e) => panic!("{}: {e}", f.display()),
            Ok(gp) => gp,
        };
        let rep = &gp.report;
        any = true;
        println!(
            "{} (frame {}): exact {:.4}% ({} of {}), mean |d| {:.4}/{:.4}/{:.4}, over2 {:.4}%, edge diffs {}",
            rep.path,
            rep.vblank,
            100.0 * rep.exact as f64 / rep.pixels.max(1) as f64,
            rep.exact,
            rep.pixels,
            rep.mean_abs[0],
            rep.mean_abs[1],
            rep.mean_abs[2],
            100.0 * rep.over2 as f64 / rep.pixels.max(1) as f64,
            gp.edge_diffs
        );
        if rep.exact == rep.pixels {
            opaque += 1;
        }
        // the difference map, if asked: red where the words differ (written before the checks,
        // so a failing frame still leaves its map behind)
        if let Some(dir) = std::env::var_os("SUNPETAL_JOB_DIFFS") {
            let name = f.file_stem().unwrap().to_string_lossy();
            let dir = PathBuf::from(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{name}.png"));
            let file = std::fs::File::create(&path).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), gp.w as u32, gp.h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&gp.diff).unwrap();
            println!("  diff map: {}", path.display());
        }
        // the acceptance of этап 4
        for c in rep.mean_abs {
            assert!(c <= 1.0 / 63.0, "{}: mean |d| {c} > 1/63", rep.path);
        }
        assert!(rep.over2 * 1000 <= rep.pixels * 5, "{}: {} of {} pixels off by more than 2 steps", rep.path, rep.over2, rep.pixels);
        assert!(rep.notes_equal, "{}: notes differ", rep.path);
        assert_eq!(gp.edge_diffs, 0, "{}: differences along polygon edges", rep.path);
    }
    if any {
        println!("{} of the dumps came out exact", opaque);
    } else {
        println!("no GPU device: the GPU parity test did not run");
    }
}

/// Этап 7.0 acceptance (`docs/GPU-BACKEND.md`): the auxiliary maps (`oid`, `cover`, `trans`) the
/// outline and the shading read match the software engine's byte-exact on every dump.
#[cfg(feature = "gpu")]
#[test]
fn gpu_aux_on_real_frames() {
    use sunpetal_runtime::nds::gpu_aux_check;
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
    let mut any = false;
    for f in files {
        let gp = match gpu_aux_check(&f) {
            Err(e) if e.contains("no GPU") => {
                println!("{}: skipped ({e})", f.display());
                continue;
            }
            Err(e) => panic!("{}: {e}", f.display()),
            Ok(gp) => gp,
        };
        any = true;
        println!(
            "{}: oid {}/{} cover {}/{} trans {}/{}, depth exact {}/{} mean {:.3} max {:.3} steps, over tol {}, quantized-to-15-bit would change {} (max {:.3} steps)",
            gp.path,
            gp.oid_exact,
            gp.pixels,
            gp.cover_exact,
            gp.pixels,
            gp.trans_exact,
            gp.pixels,
            gp.depth_exact,
            gp.pixels,
            gp.depth_mean_steps,
            gp.depth_max_steps,
            gp.depth_over_tol,
            gp.depth_quantized_diffs,
            gp.depth_quantized_max_steps
        );
        for (name, first) in [("oid", gp.oid_first), ("cover", gp.cover_first), ("trans", gp.trans_first)] {
            if let Some((i, g, s)) = first {
                println!("  first {name} mismatch at pixel {i}: gpu {g} soft {s}");
            }
        }
        if let Some((i, g, s)) = gp.depth_first {
            println!("  first depth mismatch at pixel {i}: gpu {:.9} soft {:.9}", g, s);
        }
        // the depth difference map (этап 5.1), if asked: gray by the magnitude of the difference
        if let Some(dir) = std::env::var_os("SUNPETAL_JOB_DIFFS") {
            let name = f.file_stem().unwrap().to_string_lossy();
            let dir = PathBuf::from(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{name}.depth.png"));
            let file = std::fs::File::create(&path).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), gp.w as u32, gp.h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&gp.depth_diff).unwrap();
            println!("  depth diff map: {}", path.display());
        }
        assert_eq!(gp.oid_exact, gp.pixels, "{}: oid map differs", gp.path);
        assert_eq!(gp.cover_exact, gp.pixels, "{}: cover map differs", gp.path);
        assert_eq!(gp.trans_exact, gp.pixels, "{}: trans map differs", gp.path);
    }
    if any {
        println!("auxiliary maps matched on all the dumps");
    } else {
        println!("no GPU device: the auxiliary-map test did not run");
    }
}

/// Этап 7.1 acceptance (`docs/GPU-BACKEND.md`): the outline bits (24-27) match the software
/// engine's on ≥ 95 % of the pixels of every dump.
#[cfg(feature = "gpu")]
#[test]
fn gpu_outline_on_real_frames() {
    use sunpetal_runtime::nds::gpu_outline_check;
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
    let mut any = false;
    for f in files {
        let gp = match gpu_outline_check(&f) {
            Err(e) if e.contains("no GPU") => {
                println!("{}: skipped ({e})", f.display());
                continue;
            }
            Err(e) => panic!("{}: {e}", f.display()),
            Ok(gp) => gp,
        };
        any = true;
        println!(
            "{}: outline bits {}/{} ({:.4}%), on {} pixels, words exact {}/{}",
            gp.path,
            gp.outline_exact,
            gp.pixels,
            100.0 * gp.outline_exact as f64 / gp.pixels.max(1) as f64,
            gp.outline_on,
            gp.exact,
            gp.pixels
        );
        if let Some((i, g, s)) = gp.first {
            println!("  first word mismatch at pixel {i}: gpu {g:08x} soft {s:08x}");
        }
        if let Some(dir) = std::env::var_os("SUNPETAL_JOB_DIFFS") {
            let name = f.file_stem().unwrap().to_string_lossy();
            let dir = PathBuf::from(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{name}.outline.png"));
            let file = std::fs::File::create(&path).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), gp.w as u32, gp.h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&gp.diff).unwrap();
            println!("  outline diff map: {}", path.display());
        }
        assert!(
            gp.outline_exact * 100 >= gp.pixels * 95,
            "{}: outline bits match on {} of {} pixels",
            gp.path,
            gp.outline_exact,
            gp.pixels
        );
    }
    if any {
        println!("outline bits matched on all the dumps");
    } else {
        println!("no GPU device: the outline test did not run");
    }
}

/// Этап 7.2 acceptance (`docs/GPU-BACKEND.md`): the shade bits (28-31) match the software
/// engine's on ≥ 95 % of the pixels of every dump.
#[cfg(feature = "gpu")]
#[test]
fn gpu_shade_on_real_frames() {
    use sunpetal_runtime::nds::gpu_shade_check;
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
    let mut any = false;
    for f in files {
        let gp = match gpu_shade_check(&f) {
            Err(e) if e.contains("no GPU") => {
                println!("{}: skipped ({e})", f.display());
                continue;
            }
            Err(e) => panic!("{}: {e}", f.display()),
            Ok(gp) => gp,
        };
        any = true;
        println!(
            "{}: shade bits {}/{} ({:.4}%), on {} pixels, words exact {}/{}",
            gp.path,
            gp.shade_exact,
            gp.pixels,
            100.0 * gp.shade_exact as f64 / gp.pixels.max(1) as f64,
            gp.shade_on,
            gp.exact,
            gp.pixels
        );
        if let Some((i, g, s)) = gp.first {
            println!("  first word mismatch at pixel {i}: gpu {g:08x} soft {s:08x}");
        }
        assert!(
            gp.shade_exact * 100 >= gp.pixels * 95,
            "{}: shade bits match on {} of {} pixels",
            gp.path,
            gp.shade_exact,
            gp.pixels
        );
    }
    if any {
        println!("shade bits matched on all the dumps");
    } else {
        println!("no GPU device: the shading test did not run");
    }
}

/// Этап 7.3 acceptance (`docs/GPU-BACKEND.md`): the edge marking (DISP3DCNT bit 5) matches the
/// software engine's byte-exact on every dump.
#[cfg(feature = "gpu")]
#[test]
fn gpu_edge_on_real_frames() {
    use sunpetal_runtime::nds::gpu_edge_check;
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
    let mut any = false;
    for f in files {
        let gp = match gpu_edge_check(&f) {
            Err(e) if e.contains("no GPU") => {
                println!("{}: skipped ({e})", f.display());
                continue;
            }
            Err(e) => panic!("{}: {e}", f.display()),
            Ok(gp) => gp,
        };
        any = true;
        println!("{}: words exact {}/{} ({} edge-marked pixels in the reference)", gp.path, gp.exact, gp.pixels, gp.edge_pixels);
        if let Some((i, g, s)) = gp.first {
            println!("  first word mismatch at pixel {i}: gpu {g:08x} soft {s:08x}");
        }
        assert_eq!(gp.exact, gp.pixels, "{}: the edge marking differs", gp.path);
    }
    if any {
        println!("edge marking matched on all the dumps");
    } else {
        println!("no GPU device: the edge test did not run");
    }
}
