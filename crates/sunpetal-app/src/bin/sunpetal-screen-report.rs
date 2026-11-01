//! `sunpetal-screen-report DIR`: for the pictures of both screens that `sunpetal-debug --screens-dir`
//! wrote, the frames of dialogue where the one-screen rule would not show the upper screen.
//! Exit code 1 when there are any.
fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: sunpetal-screen-report DIR");
        std::process::exit(2);
    };
    match sunpetal_app::screen_report::report(std::path::Path::new(&dir)) {
        Ok((text, bad)) => {
            print!("{}", text);
            std::process::exit(if bad == 0 { 0 } else { 1 });
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(2);
        }
    }
}
