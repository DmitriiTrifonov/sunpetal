//! Checks a data file and says what is in it.
//!
//!     sunpetal-check sunpetal.data
//!
//! The same check the game makes at start: the version, every part as long as recorded, every
//! checksum. Exit code 0 if the file holds, 1 if it is damaged or not a data file.
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: sunpetal-check FILE");
        return ExitCode::FAILURE;
    };
    match sunpetal_data::report::report(&path) {
        Ok(r) => {
            print!("{}", r);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::FAILURE
        }
    }
}
