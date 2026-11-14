//! The debug console (K-232): a line that drops from the top of the window, as in Quake. Off unless
//! `SUNPETAL_DEBUG_MENU=1`; opened by the backquote key. The commands are `sunpetal_runtime::console`'s (run on the
//! machine's thread between frames) and the two that need the app: `goto` (a restart that plays to the scene) and `shot`.
use crate::emu::Shared;

pub struct Console {
    pub enabled: bool,
    pub open: bool,
    input: String,
    lines: Vec<String>,
    history: Vec<String>,
    /// Where in the history the arrow keys are (`history.len()`: the line being typed).
    at: usize,
    focus: bool,
}

impl Default for Console {
    fn default() -> Self {
        Console {
            enabled: std::env::var("SUNPETAL_DEBUG_MENU").is_ok_and(|v| !v.is_empty() && v != "0"),
            open: false,
            input: String::new(),
            lines: vec!["console: `help` lists the commands".into()],
            history: Vec::new(),
            at: 0,
            focus: false,
        }
    }
}

/// What the window has to do for a line.
#[derive(Debug, PartialEq, Eq)]
pub enum Ask {
    Goto(String),
    Shot,
}

/// A scene as `goto` takes it: `Cnn_yy`.
pub fn valid_scene(s: &str) -> bool {
    s.strip_prefix('C').and_then(|r| r.split_once('_')).is_some_and(|(c, y)| c.len() == 2 && c.bytes().all(|b| b.is_ascii_digit()) && !y.is_empty())
}

/// The chapter whose start the scene is (`C05_01` is 5): the game starts it in place (`chapter N`); a later scene of
/// the chapter is none (its event needs the hero walking: the window restarts to play to it).
pub fn chapter_start(s: &str) -> Option<u32> {
    s.strip_prefix('C')?.strip_suffix("_01")?.parse().ok().filter(|c| (1..=19).contains(c))
}

impl Console {
    pub fn toggle(&mut self) {
        if self.enabled {
            self.open = !self.open;
            self.focus = self.open;
        }
    }

    /// How many lines there are, and the lines after the first `n` (for the script's answer).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn since(&self, n: usize) -> Vec<String> {
        self.lines.iter().skip(n).cloned().collect()
    }

    pub fn say(&mut self, text: &str) {
        for l in text.lines() {
            self.lines.push(l.to_string());
        }
        let n = self.lines.len();
        if n > 400 {
            self.lines.drain(..n - 400);
        }
    }

    /// The answers the machine has for the commands sent.
    pub fn pump(&mut self, shared: &Shared) {
        let out = std::mem::take(&mut *shared.console_out.lock().unwrap());
        for l in out {
            self.say(&l);
        }
    }

    /// One line typed: answered here, sent to the machine, or handed back as something for the window.
    pub fn submit(&mut self, shared: &Shared, line: &str) -> Option<Ask> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        self.say(&format!("> {}", line));
        if self.history.last().map(String::as_str) != Some(line) {
            self.history.push(line.to_string());
        }
        self.at = self.history.len();
        let mut w = line.split_whitespace();
        match w.next() {
            Some("goto") => match w.next() {
                Some(s) if chapter_start(s).is_some() => shared.console_in.lock().unwrap().push(format!("chapter {}", chapter_start(s).unwrap_or(0))),
                Some(s) if valid_scene(s) => return Some(Ask::Goto(s.to_string())),
                _ => self.say("goto Cnn_yy, as C04_16"),
            },
            Some("shot") => return Some(Ask::Shot),
            _ => shared.console_in.lock().unwrap().push(line.to_string()),
        }
        None
    }

    /// Completes the command word typed.
    fn complete(&mut self) {
        if self.input.contains(' ') {
            return;
        }
        let hits: Vec<&str> = sunpetal_runtime::console::COMMANDS.iter().map(|(n, _)| *n).filter(|n| n.starts_with(self.input.as_str())).collect();
        match hits.as_slice() {
            [one] => self.input = format!("{} ", one),
            [] => {}
            many => {
                let all = many.join(" ");
                self.say(&all);
            }
        }
    }

    /// Draws the console; the line to run when Enter was pressed in it.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut run = None;
        let frame = egui::Frame::new().fill(egui::Color32::from_black_alpha(215)).inner_margin(6.0);
        let width = ctx.content_rect().width();
        egui::Area::new(egui::Id::new("sunpetal-console")).order(egui::Order::Foreground).fixed_pos(egui::pos2(0.0, 0.0)).show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(width - 12.0);
                let h = (ctx.content_rect().height() * 0.4).max(80.0);
                egui::ScrollArea::vertical().max_height(h).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                    for l in &self.lines {
                        ui.label(egui::RichText::new(l).monospace().color(egui::Color32::from_gray(225)));
                    }
                });
                let edit =
                    ui.add(egui::TextEdit::singleline(&mut self.input).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).hint_text("goto C04_16, map 46, cam 20 0 …"));
                if self.focus {
                    edit.request_focus();
                    self.focus = false;
                }
                // Enter ends the edit; Tab completes, the arrows walk the history
                let (up, down, tab) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Tab)));
                if edit.has_focus() || edit.lost_focus() {
                    if up && self.at > 0 {
                        self.at -= 1;
                        self.input = self.history[self.at].clone();
                    } else if down && self.at < self.history.len() {
                        self.at += 1;
                        self.input = self.history.get(self.at).cloned().unwrap_or_default();
                    }
                    if tab {
                        self.complete();
                    }
                }
                if edit.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    run = Some(std::mem::take(&mut self.input));
                    self.focus = true;
                }
            });
        });
        run
    }
}

#[cfg(test)]
mod tests {
    use super::{chapter_start, valid_scene};

    #[test]
    fn the_start_of_a_chapter_is_started_in_place() {
        assert_eq!(chapter_start("C05_01"), Some(5));
        assert_eq!(chapter_start("C19_01"), Some(19));
        assert_eq!(chapter_start("C05_02"), None);
        assert_eq!(chapter_start("C00_01"), None);
        assert_eq!(chapter_start("C05_LIST"), None);
    }

    #[test]
    fn scene_names() {
        assert!(valid_scene("C04_16"));
        assert!(valid_scene("C04_LIST"));
        assert!(!valid_scene("A04_16"));
        assert!(!valid_scene("C4_16"));
        assert!(!valid_scene("C04"));
    }
}
