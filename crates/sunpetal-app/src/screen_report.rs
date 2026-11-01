//! The report of the one-screen walk (docs/SINGLE-SCREEN.md, G.1): for the pictures of both screens
//! that `sunpetal-debug --autoplay --screens-dir` took at every line of a scene, which screen the
//! rule of `layout::lower_wanted` picks. A line of dialogue is read on the upper screen, so a frame
//! where the rule picks the lower one (or cannot tell) is a frame without the screen it needs.
use std::collections::BTreeMap;
use std::path::Path;

use crate::layout;

/// What the rule says of the frames of one scene.
#[derive(Default, Debug, PartialEq)]
pub struct Scene {
    pub frames: usize,
    /// Frames where the rule picks the lower screen during dialogue.
    pub wrong: Vec<String>,
    /// Frames the rule cannot judge (a picture too short).
    pub unknown: Vec<String>,
}

/// The rule on frames given as (name, scale, RGBA of both screens, upper over lower); the scene is
/// the part of the name before the last `_`.
pub fn judge(frames: &[(String, usize, Vec<u8>)]) -> BTreeMap<String, Scene> {
    let mut out: BTreeMap<String, Scene> = BTreeMap::new();
    for (name, scale, rgba) in frames {
        let scene = name.rsplit_once('_').map_or(name.as_str(), |(s, _)| s).to_string();
        let s = out.entry(scene).or_default();
        s.frames += 1;
        match layout::lower_wanted(*scale, rgba) {
            Some(false) => {}
            Some(true) => s.wrong.push(name.clone()),
            None => s.unknown.push(name.clone()),
        }
    }
    out
}

fn read(path: &Path) -> Result<(usize, Vec<u8>), String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info().map_err(|e| format!("{}: {}", path.display(), e))?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).map_err(|e| format!("{}: {}", path.display(), e))?;
    if info.width % 256 != 0 || info.height != info.width / 256 * 384 {
        return Err(format!("{}: not both screens ({}x{})", path.display(), info.width, info.height));
    }
    let rgba = match info.color_type {
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        c => return Err(format!("{}: colour type {:?}", path.display(), c)),
    };
    Ok(((info.width / 256) as usize, rgba))
}

/// The report of the PNGs in `dir`: a line per scene, the bad frames named; and how many there are.
pub fn report(dir: &Path) -> Result<(String, usize), String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    names.sort();
    let mut frames = Vec::new();
    for p in names {
        let (scale, rgba) = read(&p)?;
        frames.push((p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), scale, rgba));
    }
    let scenes = judge(&frames);
    let mut text = String::new();
    let mut bad = 0;
    for (name, s) in &scenes {
        bad += s.wrong.len() + s.unknown.len();
        text += &format!("{}: {} frames, {} without the upper screen", name, s.frames, s.wrong.len() + s.unknown.len());
        if !s.wrong.is_empty() || !s.unknown.is_empty() {
            text += &format!(" ({})", s.wrong.iter().chain(&s.unknown).cloned().collect::<Vec<_>>().join(", "));
        }
        text += "\n";
    }
    text += &format!("{} scenes, {} frames, {} without the screen they need\n", scenes.len(), scenes.values().map(|s| s.frames).sum::<usize>(), bad);
    Ok((text, bad))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(upper: u8, lower: u8) -> Vec<u8> {
        let mut v = vec![upper; 256 * 192 * 4];
        v.extend(std::iter::repeat(lower).take(256 * 192 * 4));
        v
    }

    /// A lit upper screen over a dark lower one is read on the upper; a lit lower one with a dark
    /// upper (a menu) is a frame the walk reports; the frames are counted by scene.
    #[test]
    fn the_report_counts_frames_by_scene_and_names_those_without_the_upper_screen() {
        let frames = vec![
            ("C02_01_01001".to_string(), 1, frame(200, 0)),
            ("C02_01_01002".to_string(), 1, frame(200, 0)),
            ("C02_01_01003".to_string(), 1, frame(0, 200)),
            ("C04_01_01001".to_string(), 1, frame(200, 0)),
            ("C04_01_01002".to_string(), 1, vec![0; 16]),
        ];
        let r = judge(&frames);
        assert_eq!(r["C02_01"].frames, 3);
        assert_eq!(r["C02_01"].wrong, vec!["C02_01_01003".to_string()]);
        assert!(r["C04_01"].wrong.is_empty());
        assert_eq!(r["C04_01"].unknown, vec!["C04_01_01002".to_string()]);
    }
}
