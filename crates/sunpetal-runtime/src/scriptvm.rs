//! The native script interpreter in observer mode (K-162, feature `debug-tools`): it reads a scene's script
//! (`Cnn_yy.txt`) and follows the game's own runner command by command, saying where the game's order differs
//! from what the text says. Nothing here runs the game: the runner stays ARM code.
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::script::{self, Command};

/// A script as the interpreter holds it: the commands in order and the position of each label (`[[name`).
pub struct Script {
    pub cmds: Vec<Command>,
    pub labels: HashMap<String, usize>,
}

/// Reads a script's text: `script::commands` for the tags, and a line `[[name` between tags is a label before the
/// next command.
pub fn parse(src: &str) -> Script {
    let cmds = script::commands(src);
    let mut labels = HashMap::new();
    let mut n = 0; // commands met so far, by their opening tags
    let mut open = false;
    for line in src.lines().map(|l| l.split("//").next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
        match line.strip_prefix('<').and_then(|l| l.strip_suffix('>')) {
            Some(t) if t.starts_with('/') => open = false,
            Some(_) => {
                n += 1;
                open = true;
            }
            None => {
                if let (false, Some(name)) = (open, line.strip_prefix("[[")) {
                    labels.insert(name.trim().to_string(), n);
                }
            }
        }
    }
    Script { cmds, labels }
}

/// Where the interpreter says the game is, and what it has seen.
#[derive(Clone)]
pub struct Follower {
    script: Rc<Script>,
    /// Tags the game has a handler for: others (a tag the game does not know) are skipped, the game skips them too.
    known: HashSet<String>,
    pub pc: usize,
    /// The variables as the game's `SETVALUE`/`ADDVALUE` have set them since the frame began (over the game's own).
    vars: HashMap<usize, i32>,
    pub seen: usize,
    /// How many times the script has run again from its start after its end.
    pub runs: usize,
}

fn num(s: Option<&String>) -> Option<i32> {
    s?.split_whitespace().next()?.parse().ok()
}

impl Follower {
    pub fn new(script: Script, known: HashSet<String>) -> Follower {
        Follower { script: Rc::new(script), known, pc: 0, vars: HashMap::new(), seen: 0, runs: 0 }
    }

    /// The script's commands.
    pub fn commands(&self) -> usize {
        self.script.cmds.len()
    }

    /// The followers that could be where the game is, given the commands it ran in a row (`run`): one at each place
    /// of the script whose commands go as `run` does, or at each command of the first tag if none does. A game that
    /// came into a script by a label (a branch before the follower looked) could be at any of them.
    pub fn candidates(&self, run: &[&str]) -> Vec<Follower> {
        let known: Vec<(usize, &str)> = self.script.cmds.iter().enumerate().filter(|(_, c)| self.known.contains(&c.tag)).map(|(i, c)| (i, c.tag.as_str())).collect();
        let mut at: Vec<usize> = (0..known.len()).filter(|&k| run.iter().enumerate().all(|(j, t)| known.get(k + j).is_some_and(|(_, x)| x == t))).map(|k| known[k].0).collect();
        if at.is_empty() {
            at = self.script.cmds.iter().enumerate().filter(|(_, c)| run.first() == Some(&c.tag.as_str())).map(|(i, _)| i).collect();
        }
        at.into_iter()
            .map(|pc| {
                let mut f = self.clone();
                f.pc = pc;
                f
            })
            .collect()
    }

    /// A new frame: the game's variables are read afresh.
    pub fn frame(&mut self) {
        self.vars.clear();
    }

    /// The tag the interpreter expects the game to run next, if the script has not ended.
    pub fn expected(&self) -> Option<&str> {
        self.script.cmds.get(self.pc).map(|c| c.tag.as_str())
    }

    /// The game entered the handler of `tag`: `vars` reads the game's variable `n`. `Err` says how the game's order
    /// differs from the script's.
    pub fn observe(&mut self, tag: &str, vars: &dyn Fn(usize) -> i32) -> Result<(), String> {
        while self.script.cmds.get(self.pc).is_some_and(|c| !self.known.contains(&c.tag) && c.tag != tag) {
            self.pc += 1;
        }
        if self.pc >= self.script.cmds.len() && !self.script.cmds.is_empty() {
            // an event's script runs again from its start: the file is not read again
            self.pc = 0;
            self.runs += 1;
        }
        let Some(cmd) = self.script.cmds.get(self.pc).cloned() else {
            return Err(format!("the script is empty, the game runs <{}>", tag));
        };
        if cmd.tag != tag {
            return Err(format!("at {}: the script says <{}>, the game runs <{}>", self.pc, cmd.tag, tag));
        }
        self.seen += 1;
        let var = |me: &Self, n: usize| me.vars.get(&n).copied().unwrap_or_else(|| vars(n));
        let next = match tag {
            "JUMP" => cmd.params.first().and_then(|l| self.script.labels.get(l.trim())).copied(),
            "CHANGEBYVALUE" => {
                let (n, count) = (num(cmd.params.first()).unwrap_or(0) as usize, num(cmd.params.get(1)).unwrap_or(0) as usize);
                let v = var(self, n);
                let branch = cmd.params.iter().skip(2).take(count).find_map(|p| {
                    let mut w = p.split_whitespace();
                    let (max, label) = (w.next()?.parse::<i32>().ok()?, w.next()?);
                    (v <= max).then(|| label.to_string())
                });
                // a value above every limit takes no branch: the script goes on with the next command
                match branch {
                    Some(l) => self.script.labels.get(&l).copied(),
                    None => Some(self.pc + 1),
                }
            }
            "SETVALUE" => {
                if let (Some(n), Some(v)) = (num(cmd.params.first()), num(cmd.params.get(1))) {
                    self.vars.insert(n as usize, v);
                }
                Some(self.pc + 1)
            }
            "ADDVALUE" => {
                if let (Some(n), Some(d)) = (num(cmd.params.first()), num(cmd.params.get(1))) {
                    let v = var(self, n as usize);
                    self.vars.insert(n as usize, v.wrapping_add(d));
                }
                Some(self.pc + 1)
            }
            "END" => Some(self.script.cmds.len()),
            _ => Some(self.pc + 1),
        };
        self.pc = next.ok_or_else(|| format!("at {}: <{}> leads nowhere the script knows", self.pc, tag))?;
        Ok(())
    }
}

/// The handlers of the script step by key (the address with the Thumb bit set) from the table of
/// `docs/native-script-dispatch.csv`: `code,name,case_label,handler,lines,callees`, the name as `<TAG>`.
pub fn handlers(csv: &str) -> HashMap<u32, String> {
    csv.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            Some((u32::from_str_radix(f.get(3)?, 16).ok()? | 1, f.get(1)?.trim_matches(|c| c == '<' || c == '>').to_string()))
        })
        .collect()
}

/// The observer that goes with the game frame by frame: the commands the game ran in the frame (`run`), the
/// game's variables, and the scene whose script the game read last. It holds every follower that could be where
/// the game is, takes the next scene's script when the game reads one, and keeps the mismatches it finds.
pub struct Observer {
    tags: HashSet<String>,
    scene: Option<String>,
    base: Option<Follower>,
    fs: Vec<Follower>,
    joining: bool,
    /// Commands that agreed in the scripts that are over, and the scripts followed.
    pub agreed: usize,
    pub scripts: usize,
    /// The commands of the scripts followed, in all.
    pub total: usize,
    /// Mismatches found: the frame and what differs (the first twenty), and how many in all.
    pub mismatches: Vec<(u64, String)>,
    pub mismatch_count: usize,
}

impl Observer {
    /// `tags`: the tags the game has a handler for.
    pub fn new(tags: HashSet<String>) -> Observer {
        Observer { tags, scene: None, base: None, fs: Vec::new(), joining: true, agreed: 0, scripts: 0, total: 0, mismatches: Vec::new(), mismatch_count: 0 }
    }

    /// One frame. `run`: (vblank, tag) of the handlers the game entered in it, in order. `vars`: the game's
    /// variables at its start. `scene`: the scene the game read last. `load`: the script text of a scene.
    pub fn frame(&mut self, run: &[(u64, &str)], vars: &[i32], scene: Option<&str>, load: &dyn Fn(&str) -> Result<String, String>) {
        if let Some(now) = scene.filter(|s| self.scene.as_deref() != Some(*s)) {
            let first = self.scene.is_none();
            self.agreed += self.fs.first().map_or(0, |f| f.seen);
            match load(now) {
                Ok(text) => {
                    let f = Follower::new(parse(&text), self.tags.clone());
                    self.total += f.commands();
                    self.scripts += 1;
                    self.fs = vec![f.clone()];
                    self.base = Some(f);
                    // the game is already inside the first script it is seen in; a later scene starts at its beginning
                    self.joining = first;
                }
                Err(_) => {
                    self.fs.clear();
                    self.base = None;
                }
            }
            self.scene = Some(now.to_string());
        }
        if self.base.is_none() {
            return;
        }
        if self.joining && !run.is_empty() {
            let tags: Vec<&str> = run.iter().map(|r| r.1).collect();
            self.fs = self.base.as_ref().map(|b| b.candidates(&tags)).unwrap_or_default();
            self.joining = false;
            if self.fs.is_empty() {
                self.note(run[0].0, "the game's first commands are nowhere in the script".to_string());
                self.joining = true;
                return;
            }
        }
        for f in &mut self.fs {
            f.frame();
        }
        let at = |n: usize| vars.get(n).copied().unwrap_or(0);
        for (vblank, tag) in run {
            if self.joining || self.fs.is_empty() {
                break;
            }
            let mut first_error = None;
            self.fs.retain_mut(|f| match f.observe(tag, &at) {
                Ok(()) => true,
                Err(e) => {
                    first_error.get_or_insert(e);
                    false
                }
            });
            if self.fs.is_empty() {
                self.note(*vblank, first_error.unwrap_or_default());
                // look for the game's place again from its next commands
                self.joining = true;
            }
        }
    }

    fn note(&mut self, vblank: u64, what: String) {
        self.mismatch_count += 1;
        if self.mismatches.len() < 20 {
            self.mismatches.push((vblank, format!("{}: {}", self.scene.as_deref().unwrap_or("?"), what)));
        }
    }

    /// The scene followed and the place in its script (the command the game runs next), if one place is left.
    pub fn position(&self) -> Option<(&str, usize)> {
        Some((self.scene.as_deref()?, self.fs.first()?.pc))
    }

    /// Commands that agreed, over all the scripts, the one being followed included.
    pub fn agreed_now(&self) -> usize {
        self.agreed + self.fs.first().map_or(0, |f| f.seen)
    }

    /// Whether the game got to the end of the script being followed.
    pub fn ended(&self) -> bool {
        self.fs.first().is_some_and(|f| f.expected().is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(tags: &[&str]) -> HashSet<String> {
        tags.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn the_handlers_come_from_the_table_with_the_thumb_bit() {
        let h = handlers("code,name,case_label,handler,lines,callees\n0,<R>,02046168,,,\n1,<TEXT>,02045CF6,0204802C,16,\n");
        assert_eq!(h.len(), 1);
        assert_eq!(h[&0x0204_802D], "TEXT");
    }

    #[test]
    fn follows_flow_labels_and_a_branch_on_a_variable() {
        let src = "<SETVALUE>\n3\n5\n</SETVALUE>\n<CHANGEBYVALUE>\n3\n2\n4\tlow\n9\thigh\n</CHANGEBYVALUE>\n<WAIT>\n</WAIT>\n<JUMP>\nend\n</JUMP>\n[[low\n<PLAYSE>\n1\n</PLAYSE>\n<JUMP>\nend\n</JUMP>\n[[high\n<FADE_IN>\n</FADE_IN>\n[[end\n<END>\n</END>\n";
        let mut f = Follower::new(parse(src), known(&["SETVALUE", "CHANGEBYVALUE", "WAIT", "JUMP", "PLAYSE", "FADE_IN", "END"]));
        let vars = |_| 0;
        f.observe("SETVALUE", &vars).unwrap();
        f.observe("CHANGEBYVALUE", &vars).unwrap(); // variable 3 is 5 now: the second branch
        assert_eq!(f.expected(), Some("FADE_IN"));
        f.observe("FADE_IN", &vars).unwrap();
        f.observe("END", &vars).unwrap();
        assert_eq!(f.expected(), None);
    }

    #[test]
    fn candidates_are_every_place_the_run_could_be_and_the_game_rules_them_out() {
        let f = Follower::new(
            parse(
                "<PLAYSE>
</PLAYSE>
<WAIT>
</WAIT>
<FADE_IN>
</FADE_IN>
<PLAYSE>
</PLAYSE>
<WAIT>
</WAIT>
<END>
</END>
",
            ),
            known(&["PLAYSE", "WAIT", "FADE_IN", "END"]),
        );
        let mut c = f.candidates(&["PLAYSE", "WAIT"]);
        assert_eq!(c.len(), 2);
        c.retain_mut(|f| f.observe("PLAYSE", &|_| 0).is_ok());
        c.retain_mut(|f| f.observe("WAIT", &|_| 0).is_ok());
        c.retain_mut(|f| f.observe("END", &|_| 0).is_ok());
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn a_script_that_ends_runs_again_from_its_start() {
        let mut f = Follower::new(
            parse(
                "<PLAYSE>
</PLAYSE>
<END>
</END>
",
            ),
            known(&["PLAYSE", "END"]),
        );
        for t in ["PLAYSE", "END", "PLAYSE", "END"] {
            f.observe(t, &|_| 0).unwrap();
        }
        assert_eq!(f.runs, 1);
    }

    #[test]
    fn says_where_the_game_goes_another_way() {
        let mut f = Follower::new(parse("<PLAYSE>\n1\n</PLAYSE>\n<WAIT>\n</WAIT>\n"), known(&["PLAYSE", "WAIT", "FADE_IN"]));
        f.observe("PLAYSE", &|_| 0).unwrap();
        assert!(f.observe("FADE_IN", &|_| 0).unwrap_err().contains("the script says <WAIT>"));
    }

    #[test]
    fn a_tag_the_game_does_not_know_is_skipped() {
        let mut f = Follower::new(parse("<ON_AIR>\n</ON_AIR>\n<WAIT>\n</WAIT>\n"), known(&["WAIT"]));
        f.observe("WAIT", &|_| 0).unwrap();
    }
}
