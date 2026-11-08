//! The native model of the dialogue's order and state (K-177, `docs/NATIVE-PILOT.md`, variant A): what the game's
//! script step does to the dialogue words of the guest each logic step, from the commands that ran (`TEXT_EX`,
//! `WAIT`, `SHOWWINDOW`, `HIDEWINDOW`) and the A button. The numbers are data (`docs/native-dialog-params.csv`:
//! value and where it came from), the model is only the mechanism. The words and the chain they follow:
//! `docs/NATIVE-DIALOG.md` §2в.

/// The numbers of the dialogue's cycle (see `docs/native-dialog-params.csv`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Params {
    /// The mark `TEXT_EX` sets (counts down one per step to 0).
    pub mark_start: i32,
    /// The mark `WAIT` sets: the step waits for A.
    pub wait_mark: i32,
    /// The most the window's counter `w2` counts to.
    pub w2_cap: u32,
    /// `f48` goes to 0 when the mark is this or less.
    pub f48_clear_at: i32,
    /// `f4c` starts to move when the mark is this or less: up by `f4c_rise` to `f4c_peak`, then down by `f4c_fall`.
    pub f4c_rise_at: i32,
    pub f4c_rise: u32,
    pub f4c_peak: u32,
    pub f4c_fall: u32,
}

impl Params {
    /// Reads the table (`key,value,source` lines, the head first); every key must be there.
    pub fn parse(csv: &str) -> Result<Params, String> {
        let get = |key: &str| -> Result<i64, String> {
            let line = csv.lines().skip(1).find(|l| l.split(',').next() == Some(key)).ok_or(format!("native-dialog-params: no {}", key))?;
            line.split(',').nth(1).and_then(|v| v.trim().parse().ok()).ok_or(format!("native-dialog-params: {} is not a number", key))
        };
        Ok(Params {
            mark_start: get("mark_start")? as i32,
            wait_mark: get("wait_mark")? as i32,
            w2_cap: get("w2_cap")?.try_into().map_err(|_| "native-dialog-params: w2_cap < 0")?,
            f48_clear_at: get("f48_clear_at")? as i32,
            f4c_rise_at: get("f4c_rise_at")? as i32,
            f4c_rise: get("f4c_rise")?.try_into().map_err(|_| "native-dialog-params: f4c_rise < 0")?,
            f4c_peak: get("f4c_peak")?.try_into().map_err(|_| "native-dialog-params: f4c_peak < 0")?,
            f4c_fall: get("f4c_fall")?.try_into().map_err(|_| "native-dialog-params: f4c_fall < 0")?,
        })
    }

    /// The table the repository carries.
    pub fn builtin() -> Params {
        Params::parse(include_str!("../../../docs/native-dialog-params.csv")).expect("docs/native-dialog-params.csv")
    }
}

/// The dialogue words the model keeps (the guest's `[0x0206577C]`, `[0x0207596C+0x48]`, `[0x0207596C+0x4C]`,
/// `[0x020758EC+2]`, the window flag `[0x02078598]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct State {
    pub mark: i32,
    pub f48: u32,
    pub f4c: u32,
    pub w2: u32,
    pub window: bool,
}

#[derive(Clone, Debug)]
pub struct Model {
    p: Params,
    pub state: State,
    rising: bool,
    /// Whether a `TEXT_EX` has run: before the first one `f48`, `f4c` and `w2` hold what the scene's start left in
    /// them (the game sets them elsewhere), and the model does not follow them.
    armed: bool,
    /// How many `TEXT_EX` ran: the text pointer changes that many times.
    pub texts: u64,
}

impl Model {
    pub fn new(p: Params, state: State) -> Model {
        Model { p, state, rising: false, armed: false, texts: 0 }
    }

    /// The mark taken from the game: other commands (a wait of so many frames, a fade) set it, and the model
    /// does not know them yet.
    pub fn set_mark(&mut self, mark: i32) {
        self.state.mark = mark;
    }

    /// Whether the cycle's words (`f48`, `f4c`, `w2`) are the model's yet (after the first `TEXT_EX`).
    pub fn armed(&self) -> bool {
        self.armed
    }

    /// One logic step of the game's script step: `tags` are the commands that ran in it (`TEXT_EX`…, as `scriptvm::handlers` names them), in order,
    /// `a` is whether A was pressed since the last step.
    pub fn step(&mut self, tags: &[&str], a: bool) {
        let p = &self.p;
        let s = &mut self.state;
        let mut fresh = false;
        if s.mark == p.wait_mark && a {
            // the step sees A: the wait is over, the icon goes out
            s.mark = 0;
            if self.armed {
                s.f4c = 0;
            }
            self.rising = false;
            return;
        }
        if s.mark == 0 {
            for t in tags {
                match *t {
                    "TEXT_EX" => {
                        s.mark = p.mark_start;
                        s.f48 = 1;
                        s.f4c = 0;
                        s.w2 = 0;
                        self.rising = true;
                        self.texts += 1;
                        self.armed = true;
                        fresh = true;
                    }
                    "WAIT" => s.mark = p.wait_mark,
                    "SHOWWINDOW" => s.window = true,
                    "HIDEWINDOW" => s.window = false,
                    _ => {}
                }
            }
        }
        if s.mark > 0 {
            s.mark -= 1;
        }
        if !self.armed {
            return;
        }
        if !fresh {
            s.w2 = (s.w2 + 1).min(p.w2_cap);
        }
        if s.mark <= p.f48_clear_at {
            s.f48 = 0;
        }
        if s.mark <= p.f4c_rise_at {
            if self.rising {
                s.f4c += p.f4c_rise;
                if s.f4c >= p.f4c_peak {
                    s.f4c = p.f4c_peak;
                    self.rising = false;
                }
            } else {
                s.f4c = s.f4c.saturating_sub(p.f4c_fall);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model::new(Params::builtin(), State { w2: 10, window: true, ..State::default() })
    }

    #[test]
    fn the_table_reads_and_a_missing_key_is_named() {
        assert_eq!(Params::builtin().mark_start, 20);
        assert!(Params::parse("key,value,source\nmark_start,20,x\n").unwrap_err().contains("wait_mark"));
    }

    #[test]
    fn text_ex_starts_the_cycle_and_the_mark_counts_down_to_the_wait() {
        let mut m = model();
        m.step(&["TEXT_EX"], false);
        assert_eq!((m.state.mark, m.state.f48, m.state.w2, m.state.f4c), (19, 1, 0, 0));
        for _ in 0..11 {
            m.step(&[], false);
        }
        // mark 8: f48 is out; 7: f4c starts
        assert_eq!((m.state.mark, m.state.f48, m.state.f4c), (8, 0, 0));
        m.step(&[], false);
        assert_eq!((m.state.mark, m.state.f4c), (7, 7));
        for _ in 0..7 {
            m.step(&[], false);
        }
        assert_eq!((m.state.mark, m.state.f4c), (0, 40));
        m.step(&["WAIT"], false);
        assert_eq!((m.state.mark, m.state.f4c), (-1, 39));
        m.step(&[], false);
        assert_eq!(m.state.f4c, 38);
    }

    #[test]
    fn a_ends_the_wait_only_when_waiting() {
        let mut m = model();
        m.step(&[], true);
        assert_eq!(m.state.mark, 0);
        m.step(&["WAIT"], false);
        assert_eq!(m.state.mark, -1);
        m.step(&[], true);
        assert_eq!((m.state.mark, m.state.f4c), (0, 0));
        assert_eq!(m.texts, 0);
    }

    #[test]
    fn the_window_flag_follows_show_and_hide() {
        let mut m = model();
        m.step(&["HIDEWINDOW"], false);
        assert!(!m.state.window);
        m.step(&["SHOWWINDOW"], false);
        assert!(m.state.window);
    }
}
