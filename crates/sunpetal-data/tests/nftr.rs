//! NFTR fonts of the ROM go through `Nftr::parse` and `write` unchanged. Needs SUNPETAL_ROM; skipped without it.
use sunpetal_data::nftr::{Font, Nftr};
use sunpetal_data::nitrofs::NitroFs;

#[test]
fn every_font_of_the_rom_is_written_back_byte_for_byte() {
    let Some(rom) = std::env::var_os("SUNPETAL_ROM") else {
        eprintln!("skipped: no SUNPETAL_ROM");
        return;
    };
    let rom = std::fs::read(rom).unwrap();
    let fs = NitroFs::parse(&rom).unwrap();
    let mut n = 0;
    for p in fs.paths() {
        if !p.to_lowercase().ends_with(".nftr") {
            continue;
        }
        let bytes = fs.file(&rom, p).unwrap();
        let font = Nftr::parse(bytes).unwrap_or_else(|e| panic!("{}: {}", p, e));
        assert!(font.write() == bytes, "{} is not the same after write", p);
        // the measuring parser sees the same characters
        let m = Font::parse(bytes).unwrap();
        assert!(m.has('A'), "{}", p);
        n += 1;
    }
    assert!(n >= 4, "fonts found: {}", n);
}
