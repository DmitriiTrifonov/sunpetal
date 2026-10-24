//! The ARM7's serial devices: the SPI bus (the power manager, the firmware's flash, the touch
//! screen's controller) and the RTC.
//!
//! The firmware is made up, as melonDS makes it when it has no dump (the oracle's settings): 128
//! KiB, the user settings twice at its end with their CRC (nickname "melonDS", English, birthday
//! 1/1, the touch screen's calibration). The RTC keeps a fixed date (2009-04-20, a Monday), in
//! 24-hour mode: a run is the same run every time. The touch screen reports the pen a script puts down (`input.rs`).
use super::{now, Nds};

const USER_SETTINGS: usize = 0x1FE00;
const FW_SIZE: usize = 0x20000;

fn crc16(mut crc: u32, p: &[u8]) -> u16 {
    for &b in p {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc as u16
}

fn put16(p: &mut [u8], v: u16) {
    p[0] = v as u8;
    p[1] = (v >> 8) as u8;
}

fn make_firmware() -> Vec<u8> {
    let mut fw = vec![0xFFu8; FW_SIZE];
    fw[..0x200].fill(0);
    put16(&mut fw[0x20..], (USER_SETTINGS >> 3) as u16); // where the user settings are, / 8
    fw[0x1D] = 0xFF; // console type: DS
    // the wireless settings the ARM7 copies to 0x027FFCF4 at boot: melonDS's MAC address
    // (00:09:BF:11:22:33) and enabled channels (1-13)
    fw[0x36..0x3C].copy_from_slice(&[0x00, 0x09, 0xBF, 0x11, 0x22, 0x33]);
    put16(&mut fw[0x3C..], 0x3FFE);
    put16(&mut fw[0x2C..], 0x0138); // the wireless block's length, and its CRC
    let c = crc16(0, &fw[0x2C..0x2C + 0x138]);
    put16(&mut fw[0x2A..], c);
    let mut u = [0u8; 0x100];
    put16(&mut u[0x00..], 5); // version
    u[0x03] = 1; // birthday: month, day
    u[0x04] = 1;
    for (i, ch) in "melonDS".chars().enumerate() {
        put16(&mut u[0x06 + 2 * i..], ch as u16);
    }
    put16(&mut u[0x1A..], 7);
    // touch calibration: ADC (0, 0) at pixel (0, 0), (0xFF0, 0xBF0) at (255, 191)
    put16(&mut u[0x58..], 0);
    put16(&mut u[0x5A..], 0);
    put16(&mut u[0x5E..], 0x0FF0);
    put16(&mut u[0x60..], 0x0BF0);
    u[0x62] = 0xFF;
    u[0x63] = 0xBF;
    put16(&mut u[0x64..], 0x0031); // English, backlight at its highest
    put16(&mut u[0x70..], 0); // update counter
    let c = crc16(0xFFFF, &u[..0x70]);
    put16(&mut u[0x72..], c);
    fw[USER_SETTINGS..USER_SETTINGS + 0x100].copy_from_slice(&u);
    fw[USER_SETTINGS + 0x100..USER_SETTINGS + 0x200].copy_from_slice(&u);
    fw
}

pub(super) struct Spi {
    fw: Vec<u8>,
    spicnt: u16,
    spidata: u8,
    /// A device is selected (a command is running).
    sel: bool,
    dev: u32,
    /// Bytes of this command so far.
    n: u32,
    cmd: u8,
    addr: u32,
    /// The power manager's registers.
    pm: [u8; 8],
    fw_status: u8,
    tsc_val: u16,
    tsc_left: u32,
    pen: bool,
    pen_x: u16,
    pen_y: u16,
    // the RTC
    rtc_io: u16,
    r_in: u8,
    r_in_bit: u8,
    r_in_pos: u8,
    r_out: [u8; 8],
    r_out_bit: u8,
    r_out_pos: usize,
    /// Command number (0-7) and read bit (0x80).
    rcmd: u8,
    stat1: u8,
    stat2: u8,
    alarm1: [u8; 3],
    alarm2: [u8; 3],
    adjust: u8,
    free: u8,
}

impl Spi {
    pub(super) fn new() -> Spi {
        Spi {
            fw: Vec::new(),
            spicnt: 0,
            spidata: 0,
            sel: false,
            dev: 0,
            n: 0,
            cmd: 0,
            addr: 0,
            pm: [0x0D, 0x00, 0x01, 0x00, 0x40, 0, 0, 0],
            fw_status: 0,
            tsc_val: 0,
            tsc_left: 0,
            pen: false,
            pen_x: 0,
            pen_y: 0,
            rtc_io: 0,
            r_in: 0,
            r_in_bit: 0,
            r_in_pos: 0,
            r_out: [0; 8],
            r_out_bit: 0,
            r_out_pos: 0,
            rcmd: 0,
            stat1: 0x02,
            stat2: 0,
            alarm1: [0; 3],
            alarm2: [0; 3],
            adjust: 0,
            free: 0,
        }
    }
}

fn bcd(v: u64) -> u8 {
    (((v / 10) << 4) | (v % 10)) as u8
}

fn bitrev(v: u8) -> u8 {
    v.reverse_bits()
}

impl Nds {
    pub(super) fn spi_reset(&mut self) {
        self.spi = Spi::new();
        self.spi.fw = make_firmware();
    }

    /// The stylus down at (x, y) of the lower screen, or up.
    pub(super) fn spi_set_pen(&mut self, down: bool, x: i32, y: i32) {
        self.spi.pen = down;
        self.spi.pen_x = (x << 4) as u16;
        self.spi.pen_y = (y << 4) as u16;
    }

    /// The 0x70 bytes of user settings, as the firmware holds them.
    pub(crate) fn firmware_user_settings(&self) -> Vec<u8> {
        self.spi.fw[USER_SETTINGS..USER_SETTINGS + 0x70].to_vec()
    }

    // ---- the SPI bus ----------------------------------------------------------------------
    fn power(&mut self, v: u8) -> u8 {
        let s = &mut self.spi;
        if s.n == 0 {
            s.cmd = v;
            return 0;
        }
        let reg = (s.cmd & 7) as usize;
        if s.cmd & 0x80 != 0 {
            return s.pm[reg];
        }
        s.pm[reg] = v;
        0
    }

    fn firmware(&mut self, v: u8) -> u8 {
        if self.spi.n == 0 {
            self.spi.cmd = v;
            self.spi.addr = 0;
            return 0;
        }
        let (cmd, n) = (self.spi.cmd, self.spi.n);
        let s = &mut self.spi;
        match cmd {
            0x03 | 0x0B => {
                // READ, FAST READ (a dummy byte first)
                if n <= 3 {
                    s.addr = s.addr << 8 | v as u32;
                    return 0;
                }
                if cmd == 0x0B && n == 4 {
                    return 0;
                }
                let a = s.addr as usize % FW_SIZE;
                s.addr += 1;
                s.fw[a]
            }
            0x05 => s.fw_status,
            0x06 => {
                s.fw_status |= 2;
                0
            }
            0x04 => {
                s.fw_status &= !2;
                0
            }
            0x9F => {
                const ID: [u8; 3] = [0x20, 0x40, 0x12];
                if n <= 3 { ID[n as usize - 1] } else { 0xFF }
            }
            0x0A | 0x02 => {
                // page write, page program
                if n <= 3 {
                    s.addr = s.addr << 8 | v as u32;
                    return 0;
                }
                if s.fw_status & 2 != 0 {
                    let a = s.addr as usize % FW_SIZE;
                    s.addr += 1;
                    s.fw[a] = if cmd == 0x02 { s.fw[a] & v } else { v };
                }
                0
            }
            _ => {
                self.once("firmware cmd", cmd as u32, || format!("firmware: command {:02X} not done", cmd));
                0xFF
            }
        }
    }

    fn touch(&mut self, v: u8) -> u8 {
        let s = &mut self.spi;
        let mut out = 0;
        if s.tsc_left == 2 {
            out = (s.tsc_val >> 5) as u8;
        } else if s.tsc_left == 1 {
            out = (s.tsc_val << 3) as u8;
        }
        if s.tsc_left != 0 {
            s.tsc_left -= 1;
        }
        if v & 0x80 != 0 {
            // no pen: Y reads full scale, X and the pressure none
            let pen = s.pen;
            s.tsc_val = match (v >> 4) & 7 {
                1 => if pen { s.pen_y } else { 0xFFF }, // Y
                5 => if pen { s.pen_x } else { 0 },     // X
                3 => if pen { 0x200 } else { 0 },       // Z1
                4 => if pen { 0xE00 } else { 0xFFF },   // Z2
                0 => 0x2D4, // temperature 0
                7 => 0x358, // temperature 1
                _ => 0,
            };
            if v & 0x08 != 0 {
                s.tsc_val &= 0xFF0; // 8-bit mode
            }
            s.tsc_left = 2;
        }
        out
    }

    pub(super) fn spi_write(&mut self, addr: u32, v: u32) {
        if addr & 0xFF == 0xC0 {
            self.spi.spicnt = (v & 0xCF03) as u16;
            if v & 0x8000 == 0 {
                self.spi.sel = false;
            }
            return;
        }
        if self.spi.spicnt & 0x8000 == 0 {
            return;
        }
        let dev = ((self.spi.spicnt >> 8) & 3) as u32;
        if !self.spi.sel || dev != self.spi.dev {
            self.spi.sel = true;
            self.spi.dev = dev;
            self.spi.n = 0;
        }
        let b = v as u8;
        let out = match dev {
            0 => self.power(b),
            1 => self.firmware(b),
            2 => self.touch(b),
            _ => 0,
        };
        self.spi.spidata = out;
        self.spi.n += 1;
        if self.spi.spicnt & 0x800 == 0 {
            self.spi.sel = false;
        }
        if self.spi.spicnt & 0x4000 != 0 {
            self.irq_raise(7, 23);
        }
    }

    pub(super) fn spi_read(&self, addr: u32) -> u32 {
        if addr & 0xFF == 0xC0 { self.spi.spicnt as u32 } else { self.spi.spidata as u32 }
    }

    // ---- the RTC ----------------------------------------------------------------------------
    /// Seconds of DS time since the run began, onto 2009-04-20 (a Monday) 12:00:00.
    fn datetime() -> [u8; 7] {
        let s = now() / 67_027_964;
        let (sec, min, hour) = (s % 60, s / 60 % 60, (12 + s / 3600 % 24) % 24);
        [bcd(9), bcd(4), bcd(20), 1, bcd(hour) | if hour >= 12 { 0x40 } else { 0 }, bcd(min), bcd(sec)]
    }

    fn rtc_byte(&mut self, b: u8) {
        if self.spi.r_in_pos == 0 {
            let c = if b & 0x0F == 0x06 { b } else { bitrev(b) };
            let s = &mut self.spi;
            s.rcmd = ((c >> 4) & 7) | (c & 0x80);
            s.r_out = [0; 8];
            if s.rcmd & 0x80 != 0 {
                match s.rcmd & 7 {
                    0 => {
                        s.r_out[0] = s.stat1;
                        s.stat1 &= 0x0F; // reading clears the power and interrupt flags
                    }
                    1 => s.r_out[..3].copy_from_slice(&s.alarm1),
                    2 => s.r_out[..7].copy_from_slice(&Self::datetime()),
                    3 => s.r_out[0] = s.adjust,
                    4 => s.r_out[0] = s.stat2,
                    5 => s.r_out[..3].copy_from_slice(&s.alarm2),
                    6 => s.r_out[..3].copy_from_slice(&Self::datetime()[4..]),
                    _ => s.r_out[0] = s.free,
                }
            }
            return;
        }
        let s = &mut self.spi;
        let i = s.r_in_pos as usize - 1;
        match s.rcmd & 7 {
            0 => {
                if i == 0 {
                    s.stat1 = (s.stat1 & 0xF1) | (b & 0x0E);
                }
            }
            1 => {
                if i < 3 {
                    s.alarm1[i] = b;
                }
            }
            3 => {
                if i == 0 {
                    s.adjust = b;
                }
            }
            4 => {
                if i == 0 {
                    s.stat2 = b;
                }
            }
            5 => {
                if i < 3 {
                    s.alarm2[i] = b;
                }
            }
            7 => {
                if i == 0 {
                    s.free = b;
                }
            }
            _ => {} // the date is fixed: writes to it are dropped
        }
    }

    pub(super) fn rtc_write(&mut self, v: u16) {
        if v & 4 != 0 {
            if self.spi.rtc_io & 4 == 0 {
                // chip select: a new command
                let s = &mut self.spi;
                s.r_in = 0;
                s.r_in_bit = 0;
                s.r_in_pos = 0;
                s.r_out_bit = 0;
                s.r_out_pos = 0;
            } else if v & 2 == 0 {
                // clock low: a bit moves
                if v & 0x10 != 0 {
                    let s = &mut self.spi;
                    s.r_in |= ((v & 1) as u8) << s.r_in_bit;
                    s.r_in_bit += 1;
                    if s.r_in_bit == 8 {
                        let b = s.r_in;
                        s.r_in = 0;
                        s.r_in_bit = 0;
                        self.rtc_byte(b);
                        self.spi.r_in_pos += 1;
                    }
                } else {
                    let s = &mut self.spi;
                    s.rtc_io = (s.rtc_io & !1) | ((s.r_out[s.r_out_pos] >> s.r_out_bit) & 1) as u16;
                    s.r_out_bit += 1;
                    if s.r_out_bit == 8 {
                        s.r_out_bit = 0;
                        if s.r_out_pos < 7 {
                            s.r_out_pos += 1;
                        }
                    }
                }
            }
        }
        let s = &mut self.spi;
        s.rtc_io = if v & 0x10 != 0 { v } else { (s.rtc_io & 1) | (v & 0xFFFE) };
    }

    pub(super) fn rtc_read(&self) -> u16 {
        self.spi.rtc_io
    }
}
