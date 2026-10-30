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

/// A pack with a made font: it survives the container and, applied to the ROM, gives the font the
/// measuring parser reads (the width check counts by it). Needs SUNPETAL_ROM.
#[test]
fn a_font_in_the_pack_replaces_the_roms_dialogue_font() {
    use sunpetal_data::fontgen::{self, Params};
    use sunpetal_data::translate::{Table, Translation, DIALOGUE_FONT};
    let Some(rom) = std::env::var_os("SUNPETAL_ROM") else {
        eprintln!("skipped: no SUNPETAL_ROM");
        return;
    };
    let rom = std::fs::read(rom).unwrap();
    let fs = NitroFs::parse(&rom).unwrap();
    let base = Nftr::parse(fs.file(&rom, DIALOGUE_FONT).unwrap()).unwrap();
    let ubuntu = epaint_default_fonts::UBUNTU_LIGHT;
    let chars = fontgen::charset(["Привет, мир"]);
    let font = fontgen::build(&base, ubuntu, &chars, &Params::default()).unwrap().write();
    assert!(!Font::parse(fs.file(&rom, DIALOGUE_FONT).unwrap()).unwrap().has('П'));

    let (mut tr, _) = Translation::make(&rom, &rom, Table::default(), "ru", "ue").unwrap();
    assert!(tr.deltas.is_empty());
    tr.set_font(&rom, DIALOGUE_FONT, &font).unwrap();
    tr.set_font(&rom, DIALOGUE_FONT, &font).unwrap(); // twice: replaced, not added
    assert_eq!(tr.deltas.len(), 1);
    let back = Translation::from_bytes(&tr.to_bytes().unwrap()).unwrap();
    let files = back.overlay(&rom).unwrap();
    let made = &files.iter().find(|f| f.0 == DIALOGUE_FONT).expect("the font is in the overlay").1;
    assert_eq!(made, &font);
    assert!(Font::parse(made).unwrap().has('П'));
    assert!(tr.set_font(&rom, DIALOGUE_FONT, b"not a font").is_err());
    assert!(tr.set_font(&rom, "Chapter/other.nftr", &font).is_err());
}
