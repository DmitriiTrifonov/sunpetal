//! The switches of the native modules (docs/NATIVE-ENGINE-PLAN.md). `SUNPETAL_NATIVE=1` turns them all on, together; a
//! module's own variable overrides it (`SUNPETAL_NATIVE_LOAD=0` leaves that one to the game, `=1` takes it alone).
//! Off by default: the ARM code is the reference until the author says otherwise.

/// Whether the module whose variable is `name` is on.
pub fn on(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) if v == "1" => true,
        Ok(v) if v == "0" => false,
        _ => all(),
    }
}

/// `SUNPETAL_NATIVE=1`: every module on.
pub fn all() -> bool {
    std::env::var("SUNPETAL_NATIVE").is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_modules_follow_the_umbrella_unless_set() {
        // one test body: the environment is the process's
        std::env::remove_var("SUNPETAL_NATIVE");
        std::env::remove_var("SUNPETAL_NATIVE_T1");
        assert!(!super::on("SUNPETAL_NATIVE_T1"));
        std::env::set_var("SUNPETAL_NATIVE", "1");
        assert!(super::on("SUNPETAL_NATIVE_T1"));
        std::env::set_var("SUNPETAL_NATIVE_T1", "0");
        assert!(!super::on("SUNPETAL_NATIVE_T1"));
        std::env::remove_var("SUNPETAL_NATIVE");
        std::env::set_var("SUNPETAL_NATIVE_T1", "1");
        assert!(super::on("SUNPETAL_NATIVE_T1"));
        std::env::remove_var("SUNPETAL_NATIVE_T1");
    }
}
