//! The cartridge: the ROM's commands over the .nds file, and the backup memory on AUXSPI over a
//! save file.
//!
//! ROMCTRL's start bit runs the command in CARD_COMMAND at once: its data (a block of 0x200 bytes
//! for the main-data read 0xB7, the chip ID for 0xB8) is ready word by word at 0x04100010, read
//! by the CPU or by a DMA channel in the card's start mode; when the last word is taken the
//! transfer ends and, if AUXSPICNT enables it, the card interrupt is raised. A cartridge past its
//! secure area is in KEY2 mode; the commands arrive here plain.
//!
//! The backup memory is an EEPROM (64 Kbit and up: commands with a 2-byte address) or a flash chip
//! (3-byte addresses), its type taken from the save file's size, or `Config::save_size` when
//! there is no file yet (8 KiB by default).
use std::fs;
use std::path::PathBuf;

use super::{note, Nds};

pub(super) struct Cart {
    pub(super) rom: Vec<u8>,
    chip_id: u32,
    auxcnt: u16,
    romctrl: u32,
    cmd: [u8; 8],
    data: Vec<u32>,
    pos: usize,
    /// The CPU that started the transfer.
    owner: i32,
    save: Vec<u8>,
    save_path: PathBuf,
    dirty: bool,
    bk_cmd: u8,
    bk_status: u8,
    bk_out: u8,
    bk_addr: u32,
    bk_n: u32,
    bk_sel: bool,
    /// The first bytes of the command being sent (for the trace).
    bk_head: [u8; 5],
}

impl Cart {
    pub(super) fn new() -> Cart {
        Cart {
            rom: Vec::new(),
            chip_id: 0,
            auxcnt: 0,
            romctrl: 0,
            cmd: [0; 8],
            data: Vec::new(),
            pos: 0,
            owner: 9,
            save: Vec::new(),
            save_path: PathBuf::new(),
            dirty: false,
            bk_cmd: 0,
            bk_status: 0,
            bk_out: 0,
            bk_addr: 0,
            bk_n: 0,
            bk_sel: false,
            bk_head: [0; 5],
        }
    }
}

/// The address mask of the cartridge's chip: a chip is a power of two in size, an image smaller
/// than it (a rebuilt ROM) reads 0xFF past its end.
fn chip_mask(rom_len: usize) -> u32 {
    (rom_len.next_power_of_two() as u32).wrapping_sub(1)
}

impl Nds {
    pub(super) fn cart_open(&mut self) -> Result<(), String> {
        // no ROM file named: the image kept in the data file
        let (path, mut rom) = if self.cfg.rom.as_os_str().is_empty() {
            let data = self.cfg.data.clone().ok_or("no ROM and no data file to start from")?;
            let rom = sunpetal_data::dsrom::read(&data)?.ok_or_else(|| format!("{} holds no ROM, and none was named", data.display()))?;
            (data, rom)
        } else {
            let path = self.cfg.rom.clone();
            let rom = fs::read(&path).map_err(|e| format!("cannot open {}: {}", path.display(), e))?;
            (path, rom)
        };
        if rom.len() < 0x200 {
            return Err(format!("{} is too small to be a cartridge", path.display()));
        }
        let mb = ((rom.len() + 0xF_FFFF) >> 20) as u32;
        // as melonDS makes it; of the ROM itself, not of the image the overlay grows
        let chip_id = 0xC2 | ((if mb != 0 { mb - 1 } else { 0 }) & 0xFF) << 8;
        let files = super::overlay_files(&self.cfg, &rom)?;
        if !files.is_empty() {
            let n = crate::nitrofs::apply_overlay(&mut rom, &files)?;
            note(&format!("overlay: {} files replace the ROM's", n));
        }
        let save_path = match &self.cfg.save {
            Some(p) => p.clone(),
            None => {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.cfg.out.join(format!("{}.sav", stem))
            }
        };
        let mut save = vec![0xFF; self.cfg.save_size];
        if let Ok(s) = fs::read(&save_path) {
            if !s.is_empty() {
                save = s;
            }
        }
        let c = &mut self.cart;
        *c = Cart::new();
        c.rom = rom;
        c.chip_id = chip_id;
        c.save = save;
        c.save_path = save_path;
        Ok(())
    }

    pub(crate) fn rom(&self) -> &[u8] {
        &self.cart.rom
    }

    pub(crate) fn chip_id(&self) -> u32 {
        self.cart.chip_id
    }

    fn write_save(&mut self) -> Result<(), String> {
        if !self.cart.dirty {
            return Ok(());
        }
        let path = self.cart.save_path.clone();
        if let Some(d) = path.parent() {
            let _ = fs::create_dir_all(d);
        }
        // whole, then moved into place: a kill in the middle leaves the old file
        let tmp = path.with_extension("sav.tmp");
        fs::write(&tmp, &self.cart.save).map_err(|e| format!("cannot write {}: {}", tmp.display(), e))?;
        fs::rename(&tmp, &path).map_err(|e| format!("cannot move {} into place: {}", tmp.display(), e))?;
        self.cart.dirty = false;
        Ok(())
    }

    pub(super) fn cart_save(&mut self) {
        if !self.cart.dirty {
            return;
        }
        match self.write_save() {
            Ok(()) => note(&format!("backup memory written to {}", self.cart.save_path.display())),
            Err(e) => note(&e),
        }
    }

    pub(super) fn cart_save_quiet(&mut self) {
        if let Err(e) = self.write_save() {
            note(&e);
        }
    }

    // ---- ROM commands ---------------------------------------------------------------------
    fn card_finish(&mut self) {
        self.cart.romctrl &= !0x8080_0000;
        if self.cart.auxcnt & 0x4000 != 0 {
            let owner = self.cart.owner;
            self.irq_raise(owner, 19);
        }
    }

    fn card_start(&mut self, cpu: i32) {
        let c = &mut self.cart;
        c.owner = cpu;
        let bs = (c.romctrl >> 24) & 7;
        let len: u32 = match bs {
            0 => 0,
            7 => 4,
            _ => 0x100 << bs,
        };
        c.data = vec![0xFFFF_FFFF; (len / 4) as usize];
        c.pos = 0;
        let op = c.cmd[0];
        if op == 0xB7 {
            let mut addr = (c.cmd[1] as u32) << 24 | (c.cmd[2] as u32) << 16 | (c.cmd[3] as u32) << 8 | c.cmd[4] as u32;
            addr &= chip_mask(c.rom.len());
            if addr < 0x8000 {
                addr = 0x8000 + (addr & 0x1FF);
            }
            for i in 0..(len / 4) as usize {
                let a = addr as usize + 4 * i;
                if a + 4 <= c.rom.len() {
                    c.data[i] = u32::from_le_bytes([c.rom[a], c.rom[a + 1], c.rom[a + 2], c.rom[a + 3]]);
                }
            }
            #[cfg(feature = "debug-tools")]
            super::debug::on_card_read(addr, len);
            self.trace(|| format!("card: read {:08X}, {} bytes", addr, len));
        } else if op == 0xB8 {
            let id = c.chip_id;
            c.data.fill(id);
        } else {
            let cmd = c.cmd;
            self.once("card cmd", op as u32, || {
                format!(
                    "card: command {:02X} {:02X}{:02X}{:02X}{:02X} {:02X}{:02X}{:02X} not done (0xFF)",
                    op, cmd[1], cmd[2], cmd[3], cmd[4], cmd[5], cmd[6], cmd[7]
                )
            });
        }
        if self.cart.data.is_empty() {
            return self.card_finish();
        }
        self.cart.romctrl |= 0x8080_0000;
        self.dma_card(cpu);
    }

    pub(super) fn cart_word_ready(&self) -> bool {
        self.cart.pos < self.cart.data.len()
    }

    pub(super) fn cart_data_word(&mut self) -> u32 {
        let c = &mut self.cart;
        if c.pos >= c.data.len() {
            return 0xFFFF_FFFF;
        }
        let w = c.data[c.pos];
        c.pos += 1;
        if c.pos == c.data.len() {
            self.card_finish();
        }
        w
    }

    // ---- the backup memory ----------------------------------------------------------------
    fn addr_bytes(&self) -> u32 {
        let n = self.cart.save.len();
        if n >= 0x40000 {
            3
        } else if n > 512 {
            2
        } else {
            1
        }
    }

    fn backup_xfer(&mut self, v: u8) -> u8 {
        if !self.cart.bk_sel {
            let c = &mut self.cart;
            c.bk_sel = true;
            c.bk_cmd = v;
            c.bk_n = 0;
            c.bk_addr = 0;
        } else {
            self.cart.bk_n += 1;
        }
        let ab = self.addr_bytes();
        let c = &mut self.cart;
        if (c.bk_n as usize) < c.bk_head.len() {
            c.bk_head[c.bk_n as usize] = v;
        }
        let mut out = 0xFF;
        let cmd = if ab == 1 { c.bk_cmd & 0xF7 } else { c.bk_cmd };
        match cmd {
            0x06 => c.bk_status |= 2, // WREN
            0x04 => c.bk_status &= !2, // WRDI
            0x05 => {
                if c.bk_n != 0 {
                    out = c.bk_status; // RDSR
                }
            }
            0x01 => {
                if c.bk_n == 1 {
                    c.bk_status = (c.bk_status & 3) | (v & 0x0C);
                }
            }
            0x03 | 0x02 | 0x0A => {
                // READ, WRITE (page program, flash page write)
                if c.bk_n == 0 {
                    if ab == 1 && c.bk_cmd & 8 != 0 {
                        c.bk_addr = 0x100;
                    }
                } else if c.bk_n <= ab {
                    c.bk_addr = c.bk_addr << 8 | v as u32;
                    if ab == 1 {
                        c.bk_addr |= ((c.bk_cmd & 8) as u32) << 5;
                    }
                } else {
                    let a = c.bk_addr as usize % c.save.len();
                    if cmd == 0x03 {
                        out = c.save[a];
                    } else if c.bk_status & 2 != 0 {
                        c.save[a] = v;
                        c.dirty = true;
                    }
                    c.bk_addr += 1;
                }
            }
            0x9F => {} // ID: none
            _ => self.once("backup cmd", cmd as u32, || format!("backup: command {:02X} not done", cmd)),
        }
        out
    }

    fn backup_deselect(&mut self) {
        let ab = self.addr_bytes();
        if self.cart.bk_sel {
            let (cmd, addr, n, head) = (self.cart.bk_cmd, self.cart.bk_addr, self.cart.bk_n, self.cart.bk_head);
            self.trace(|| format!("backup: command {:02X}, address {:06X} after {} bytes, head {:02X?}", cmd, addr, n, head));
        }
        let c = &mut self.cart;
        if c.bk_sel && (c.bk_cmd == 0x02 || c.bk_cmd == 0x0A || ((c.bk_cmd & 0xF7) == 0x02 && ab == 1)) {
            c.bk_status &= !2; // a write ends: the latch clears
        }
        c.bk_sel = false;
    }

    // ---- registers --------------------------------------------------------------------------
    pub(super) fn cart_write(&mut self, cpu: i32, a: u32, v: u32, size: usize) {
        let off = a & 0xFF;
        if size == 4 {
            self.cart_write(cpu, a, v & 0xFFFF, 2);
            self.cart_write(cpu, a + 2, v >> 16, 2);
            return;
        }
        match off {
            0xA0 => {
                // the chip is released after a byte sent without hold, or when serial mode ends
                if self.cart.auxcnt & 0x2040 == 0x2040 && v & 0x2000 == 0 {
                    self.backup_deselect();
                }
                self.cart.auxcnt = v as u16;
            }
            0xA2 => {
                if self.cart.auxcnt & 0x2000 != 0 {
                    let out = self.backup_xfer(v as u8);
                    self.cart.bk_out = out;
                    if self.cart.auxcnt & 0x40 == 0 {
                        self.backup_deselect();
                    }
                }
            }
            0xA4 => self.cart.romctrl = (self.cart.romctrl & 0xFFFF_0000) | v,
            0xA6 => {
                let c = &mut self.cart;
                c.romctrl = (c.romctrl & 0x0080_FFFF) | ((v & 0xFF7F) << 16) | (c.romctrl & 0x2000_0000);
                if v & 0x8000 != 0 {
                    self.card_start(cpu);
                }
            }
            0xA8..=0xAF => {
                let i = (off - 0xA8) as usize;
                self.cart.cmd[i] = v as u8;
                self.cart.cmd[i + 1] = (v >> 8) as u8;
            }
            _ => {} // the KEY2 seeds (0x1B0-0x1BB): nothing to do with plain commands
        }
    }

    pub(super) fn cart_read(&self, a: u32) -> u32 {
        let c = &self.cart;
        match a & 0xFF {
            0xA0 => c.auxcnt as u32,
            0xA2 => c.bk_out as u32,
            0xA4 => c.romctrl & 0xFFFF,
            0xA6 => c.romctrl >> 16,
            off @ 0xA8..=0xAF => {
                let i = (off - 0xA8) as usize;
                c.cmd[i] as u32 | (c.cmd[i + 1] as u32) << 8
            }
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{chip_mask, Nds};
    use crate::nds::Config;

    /// One chip-select: the bytes go out, the last replies come back.
    fn xfer(n: &mut Nds, bytes: &[u8]) -> Vec<u8> {
        n.cart_write(9, 0x0400_01A0, 0xA000 | 0x40, 2);
        let mut out = Vec::new();
        for (i, b) in bytes.iter().enumerate() {
            if i + 1 == bytes.len() {
                n.cart_write(9, 0x0400_01A0, 0xA000, 2); // the last byte releases the chip
            }
            n.cart_write(9, 0x0400_01A2, *b as u32, 2);
            out.push(n.cart.bk_out);
        }
        out
    }

    #[test]
    fn a_64k_eeprom_keeps_what_is_written_to_it() {
        let mut n = Nds::new(Config::new("none"));
        n.cart.save = vec![0xFF; 65536];
        xfer(&mut n, &[0x06]);
        let mut w = vec![0x02, 0xAB, 0xC0];
        w.extend((0..32u8).map(|i| i ^ 0x5A));
        xfer(&mut n, &w);
        assert_eq!(n.cart.save[0xABC0 + 5], 5 ^ 0x5A);
        assert!(n.cart.dirty);
        // the write latch clears at the end of the write: a second write without WREN is refused
        xfer(&mut n, &[0x02, 0x00, 0x10, 0x11]);
        assert_eq!(n.cart.save[0x10], 0xFF);
        let r = xfer(&mut n, &[0x03, 0xAB, 0xC0, 0, 0, 0]);
        assert_eq!(&r[3..], &[0x5A, 0x5B, 0x58]);
    }

    #[test]
    fn the_mask_follows_the_chip_not_the_image() {
        assert_eq!(chip_mask(64 << 20), 0x03FF_FFFF);
        // a 57 MB image sits on a 64 MB chip: addresses past 32 MB must stay as they are
        assert_eq!(chip_mask(57_066_752), 0x03FF_FFFF);
        assert_eq!(chip_mask(0x200), 0x1FF);
    }
}
