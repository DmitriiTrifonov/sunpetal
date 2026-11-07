//! The native script interpreter in observer mode (K-162, feature `debug-tools`): it reads a scene's script
//! (`Cnn_yy.txt`) and follows the game's own runner command by command, saying where the game's order differs
//! from what the text says. Nothing here runs the game: the runner stays ARM code.
use std::collections::{HashMap, HashSet};

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
pub struct Follower {
    script: Script,
    /// Tags the game has a handler for: others (a tag the game does not know) are skipped, the game skips them too.
    known: HashSet<String>,
    pub pc: usize,
    /// The variables as the game's `SETVALUE`/`ADDVALUE` have set them since the frame began (over the game's own).
    vars: HashMap<usize, i32>,
    pub seen: usize,
}

fn num(s: Option<&String>) -> Option<i32> {
    s?.split_whitespace().next()?.parse().ok()
}

impl Follower {
    pub fn new(script: Script, known: HashSet<String>) -> Follower {
        Follower { script, known, pc: 0, vars: HashMap::new(), seen: 0 }
    }

    /// The script's commands.
    pub fn commands(&self) -> usize {
        self.script.cmds.len()
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
        let Some(cmd) = self.script.cmds.get(self.pc).cloned() else {
            return Err(format!("the script has ended at {}, the game runs <{}>", self.pc, tag));
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
                branch.and_then(|l| self.script.labels.get(&l).copied())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn known(tags: &[&str]) -> HashSet<String> {
        tags.iter().map(|t| t.to_string()).collect()
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
