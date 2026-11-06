//! The policy of START by scene: the game's, or the app's short menu. A scene named here is decided
//! by the table; any other (or none: the scene is unknown) falls back to the picture's rules
//! (`start_to_game` in `lib.rs`), as before. Esc and F1 always open the app's menu.

/// What START does on a scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// START goes to the game.
    Game,
    /// START opens the app's short menu.
    #[allow(dead_code)] // for scenes the author marks after seeing them; the table starts with the title
    App,
}

/// Scenes (the game's script files read last, `script_scene`) with the policy for each: sorted, no
/// duplicates. Add a line only for a scene that has been seen in the game.
pub const SCENES: &[(&str, Policy, &str)] = &[("C19_01", Policy::Game, "the title: Press START, the title's menu (B-18)")];

/// The table's policy for `scene`, if it has one.
pub fn policy(scene: Option<&str>) -> Option<Policy> {
    let name = scene?;
    SCENES.binary_search_by(|(n, _, _)| n.cmp(&name)).ok().map(|i| SCENES[i].1)
}

/// START is the game's: by the table when the scene is in it, otherwise `by_picture` (the old rules).
pub fn start_to_game(scene: Option<&str>, by_picture: bool) -> bool {
    match policy(scene) {
        Some(Policy::Game) => true,
        Some(Policy::App) => false,
        None => by_picture,
    }
}

/// Защёлка титула: поднимается плашкой «Press START» или сценой с политикой `Game`, сбрасывается
/// любой другой прочитанной сценой; без сцены (`None`) держится как была.
pub fn latch(on: bool, prompt: bool, scene: Option<&str>) -> bool {
    match scene {
        Some(_) if policy(scene) == Some(Policy::Game) => true,
        Some(_) => false,
        None => on || prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_has_no_duplicates() {
        assert!(SCENES.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn a_scene_in_the_table_decides_and_the_others_follow_the_picture() {
        // the title (B-18): the picture's rules said no on frame 1263, the table says START is the game's
        assert!(start_to_game(Some("C19_01"), false));
        // a scene not in the table, and no scene at all: the picture decides
        assert!(!start_to_game(Some("C00_03"), false));
        assert!(start_to_game(Some("C00_03"), true));
        assert!(!start_to_game(None, false));
        assert!(start_to_game(None, true));
    }

    #[test]
    fn the_title_latch_holds_until_another_scene_is_read() {
        assert!(latch(false, true, None));
        assert!(latch(true, false, None));
        assert!(!latch(false, false, None));
        assert!(!latch(true, true, Some("C02_01")));
        assert!(latch(false, false, Some("C19_01")));
    }
}
