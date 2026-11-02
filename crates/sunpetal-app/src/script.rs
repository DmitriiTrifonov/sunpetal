//! `sunpetal-app ROM --script FILE|-`: the window's picture with no window (K-090). The app runs as it does in a
//! window (layouts, the crisp dialogue text, effects, the menu), drawn into a texture off screen; commands come one
//! a line, each answered with a line of JSON on stdout. The settings are the player's but never saved; the save is a
//! copy (`desktop_main`), so a game the player has open is not touched.
//!
//!     size W H [PPP]       the window, in pixels (PPP: egui's pixels per point, 2 on a Retina screen; default 1)
//!     wait N               N frames of the machine (the app draws each)
//!     until dialogue|idle [N]  frames until a dialogue line is shown (A pressed each second, as the debugger's
//!                          `advance`) | the player has the control (at most N, 3000)
//!     press BUTTON [N]     a DS button (A B X Y L R START SELECT UP DOWN LEFT RIGHT) held N frames (default 8)
//!     tap X Y [N]          the stylus at window pixel X Y for N frames (through the layout, as the mouse)
//!     touch X Y [N]        the stylus at lower-screen DS pixel X Y for N frames (no layout)
//!     swap | menu          the screen swap (F8) | the settings menu (F1)
//!     set KEY VALUE        a setting by its path in config.toml (video.layout single, video.dialogue_font nunito)
//!     fast on|off          run without waiting for the clock (on at the start)
//!     shot FILE            the window's picture to FILE (PNG)
//!     status               where the machine and the app are
//!     quit
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use crate::{bit, config, gfx, App, Options, Settings};

/// Runs the script from `input`; the exit code of the program.
pub fn run(o: Options, input: impl BufRead) -> i32 {
    let (mut app, shared, _stream) = crate::prepare(o, true);
    let [w, h] = app.settings.video.window;
    if let Err(e) = make_gfx(&mut app, w, h, 1.0) {
        println!("{{\"error\":{:?}}}", e);
        crate::stop_machine(&shared);
        return 1;
    }
    shared.fast.store(true, Ordering::Relaxed);
    shared.paused.store(false, Ordering::Relaxed);
    // the first picture: the machine has booted (and played to the `--goto` scene), its frame count is the real one
    let since = std::time::Instant::now();
    while shared.frame.lock().unwrap().is_none() {
        if shared.stopped.lock().unwrap().is_some() || since.elapsed().as_secs() > 900 {
            println!("{{\"error\":{:?}}}", shared.stopped.lock().unwrap().clone().unwrap_or_else(|| "no picture in 15 minutes".into()));
            crate::stop_machine(&shared);
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    println!("{{\"ready\":true,\"vblank\":{}}}", shared.vblank.load(Ordering::Relaxed));
    let mut code = 0;
    for line in input.lines() {
        let Ok(line) = line else { break };
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() || words[0].starts_with('#') {
            continue;
        }
        if words[0] == "quit" {
            break;
        }
        match command(&mut app, &words) {
            Ok(said) => println!("{{\"ok\":{:?},{}}}", words[0], said),
            Err(e) => {
                println!("{{\"error\":{:?},\"line\":{:?}}}", e, line);
                code = 1;
            }
        }
        if app.shared.stopped.lock().unwrap().is_some() {
            println!("{{\"error\":\"the machine stopped\"}}");
            code = 1;
            break;
        }
    }
    crate::stop_machine(&shared);
    code
}

/// The picture off screen at `w` by `h` pixels, and the settings put into effect on it (as `resumed` does).
fn make_gfx(app: &mut App, w: u32, h: u32, ppp: f32) -> Result<(), String> {
    app.gfx = Some(gfx::Gfx::offscreen(w, h, ppp)?);
    app.win_size = (w, h);
    app.applied = Settings { video: config::Video { res: 0, fullscreen: !app.settings.video.fullscreen, ..app.settings.video.clone() }, ..app.settings.clone() };
    app.applied.input.keys.clear();
    app.apply();
    Ok(())
}

fn num<T: std::str::FromStr>(words: &[&str], i: usize, default: Option<T>) -> Result<T, String> {
    match words.get(i) {
        Some(w) => w.parse().map_err(|_| format!("{}: not a number", w)),
        None => default.ok_or_else(|| format!("{}: argument {} is missing", words[0], i)),
    }
}

/// `n` frames of the machine, the app drawing each picture it sends.
fn wait(app: &mut App, n: u64) -> Result<(), String> {
    let start = app.shared.vblank.load(Ordering::Relaxed);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60 + n / 30);
    while app.shared.vblank.load(Ordering::Relaxed) < start + n {
        if app.shared.stopped.lock().unwrap().is_some() {
            return Err("the machine stopped".into());
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("the machine did not get through {} frames in time", n));
        }
        if app.shared.frame.lock().unwrap().is_some() {
            app.redraw();
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    // the last picture, drawn
    app.redraw();
    Ok(())
}

fn command(app: &mut App, words: &[&str]) -> Result<String, String> {
    match words[0] {
        "size" => {
            let (w, h, ppp) = (num::<u32>(words, 1, None)?, num::<u32>(words, 2, None)?, num::<f32>(words, 3, Some(1.0))?);
            make_gfx(app, w.max(16), h.max(16), ppp)?;
            Ok(format!("\"size\":[{},{}]", w, h))
        }
        "wait" => {
            wait(app, num(words, 1, None)?)?;
            Ok(format!("\"vblank\":{}", app.shared.vblank.load(Ordering::Relaxed)))
        }
        "until" => {
            let what = *words.get(1).ok_or("until dialogue|idle [N]")?;
            let most: u64 = num(words, 2, Some(3000))?;
            let start = app.shared.vblank.load(Ordering::Relaxed);
            loop {
                let met = match what {
                    // the box, whatever draws its text (the crisp line is only there when the window draws it)
                    "dialogue" => app.shared.line_drawn.load(Ordering::Relaxed),
                    "idle" => app.shared.idle.load(Ordering::Relaxed),
                    other => return Err(format!("until {}: dialogue or idle", other)),
                };
                if met {
                    break;
                }
                let now = app.shared.vblank.load(Ordering::Relaxed);
                if now >= start + most {
                    return Err(format!("no {} in {} frames", what, most));
                }
                // the scene waits for A between its lines (a line still being written is finished by it)
                if what == "dialogue" && (now - start) % 60 == 59 {
                    app.press("A");
                }
                wait(app, 1)?;
            }
            Ok(format!("\"vblank\":{}", app.shared.vblank.load(Ordering::Relaxed)))
        }
        "press" => {
            let b = bit(words.get(1).copied().unwrap_or(""));
            if b == 0 {
                return Err(format!("no button {:?}", words.get(1).copied().unwrap_or("")));
            }
            app.kb |= b;
            app.send_keys();
            let held = wait(app, num(words, 2, Some(8))?);
            app.kb &= !b;
            app.send_keys();
            held.map(|_| String::from("\"held\":true"))
        }
        "tap" | "touch" => {
            let (x, y, n) = (num::<f32>(words, 1, None)?, num::<f32>(words, 2, None)?, num::<u64>(words, 3, Some(6))?);
            let on = if words[0] == "tap" {
                app.mouse = (x, y);
                app.pen_down = true;
                let g = app.gfx.as_ref().ok_or("no picture")?;
                if app.layout().touch(g.size(), app.mouse, false).is_none() {
                    app.pen_down = false;
                    return Err(format!("{} {} is not on a screen of the layout {:?}", x, y, app.layout().name));
                }
                app.update_pen();
                app.shared.input.lock().unwrap().pen
            } else {
                let p = Some((x.clamp(0.0, 255.0) as i32, y.clamp(0.0, 191.0) as i32));
                app.shared.input.lock().unwrap().pen = p;
                p
            };
            let held = wait(app, n);
            app.pen_down = false;
            app.shared.input.lock().unwrap().pen = None;
            held.map(|_| format!("\"pen\":{}", on.map_or("null".to_string(), |(x, y)| format!("[{},{}]", x, y))))
        }
        "swap" => {
            app.swap_screen();
            Ok(format!("\"layout\":{:?}", app.layout().name))
        }
        "menu" => {
            app.toggle_menu();
            Ok(format!("\"open\":{}", app.menu.open))
        }
        "set" => {
            let (key, value) = (words.get(1).ok_or("set KEY VALUE")?, words[2..].join(" "));
            app.settings = set(&app.settings, key, &value)?;
            // put into effect now, so that a shot taken next sees it
            app.redraw();
            Ok(format!("\"set\":{:?}", key))
        }
        "fast" => {
            app.shared.fast.store(words.get(1) != Some(&"off"), Ordering::Relaxed);
            Ok(String::from("\"fast\":true"))
        }
        "shot" => {
            let path = PathBuf::from(words.get(1).ok_or("shot FILE")?);
            app.pending_capture.set(Some(path.clone()));
            app.redraw();
            if !path.is_file() {
                return Err(format!("{}: not written", path.display()));
            }
            Ok(format!("\"file\":{:?}", path.display().to_string()))
        }
        "status" => {
            let text = app.shared.dialogue_text.lock().unwrap().as_ref().map_or("null".to_string(), |t| format!("{:?}", t.0));
            let (w, h) = app.gfx.as_ref().map_or((0.0, 0.0), |g| g.window_size());
            Ok(format!(
                "\"vblank\":{},\"layout\":{:?},\"lower\":{},\"window\":[{},{}],\"dialogue\":{},\"idle\":{},\"crisp\":{}",
                app.shared.vblank.load(Ordering::Relaxed),
                app.layout().name,
                app.auto_lower || app.hold.is_some(),
                w,
                h,
                text,
                app.shared.idle.load(Ordering::Relaxed),
                app.shared.crisp_text.load(Ordering::Relaxed)
            ))
        }
        other => Err(format!("no command {:?}", other)),
    }
}

/// The settings with one value changed by its path (`video.layout`), the value read as TOML (a number, true,
/// "text") or else taken as text.
fn set(s: &Settings, key: &str, value: &str) -> Result<Settings, String> {
    let mut v = toml::Value::try_from(s).map_err(|e| e.to_string())?;
    let parsed = toml::from_str::<toml::Table>(&format!("v = {}", value)).ok().and_then(|mut t| t.remove("v")).unwrap_or_else(|| toml::Value::String(value.to_string()));
    let mut at = &mut v;
    let parts: Vec<&str> = key.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        let table = at.as_table_mut().ok_or_else(|| format!("{}: not a table", parts[..i].join(".")))?;
        if i + 1 == parts.len() {
            if !table.contains_key(*part) {
                return Err(format!("no setting {}", key));
            }
            table.insert(part.to_string(), parsed.clone());
            break;
        }
        at = table.get_mut(*part).ok_or_else(|| format!("no setting {}", key))?;
    }
    let mut out: Settings = v.try_into().map_err(|e: toml::de::Error| format!("{} = {}: {}", key, value, e))?;
    out.sanitize();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_is_changed_by_its_path() {
        let s = Settings::default();
        let t = set(&s, "video.layout", "single").unwrap();
        assert_eq!(t.video.layout, "single");
        let t = set(&t, "video.res", "3").unwrap();
        assert_eq!(t.video.res, 3);
        let t = set(&t, "video.wide", "false").unwrap();
        assert!(!t.video.wide);
        assert!(set(&s, "video.nothing", "1").is_err());
        assert!(set(&s, "video.res", "\"many\"").is_err());
    }
}
