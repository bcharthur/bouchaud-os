//! Horloge temps reel (RTC) via la CMOS (ports 0x70/0x71).
//!
//! Fournit la date et l'heure courantes. QEMU expose une RTC standard ; les
//! valeurs sont souvent en BCD (selon le registre d'etat B) et converties ici.

use crate::arch::x86_64::ports::{inb, outb};

#[derive(Clone, Copy)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        outb(0x70, reg);
        inb(0x71)
    }
}

fn update_in_progress() -> bool {
    cmos_read(0x0A) & 0x80 != 0
}

fn bcd_to_bin(v: u8) -> u8 {
    (v & 0x0F) + ((v >> 4) * 10)
}

/// Lit la date et l'heure courantes depuis la RTC.
pub fn now() -> DateTime {
    // Attend la fin d'une eventuelle mise a jour, puis lit deux fois jusqu'a
    // obtenir des valeurs stables (evite les lectures pendant un tick).
    while update_in_progress() {}
    let read_raw = || DateTime {
        second: cmos_read(0x00),
        minute: cmos_read(0x02),
        hour: cmos_read(0x04),
        day: cmos_read(0x07),
        month: cmos_read(0x08),
        year: cmos_read(0x09) as u16,
    };

    let mut prev = read_raw();
    loop {
        while update_in_progress() {}
        let cur = read_raw();
        if cur.second == prev.second && cur.minute == prev.minute
            && cur.hour == prev.hour && cur.day == prev.day
            && cur.month == prev.month && cur.year == prev.year
        {
            prev = cur;
            break;
        }
        prev = cur;
    }

    let status_b = cmos_read(0x0B);
    let mut dt = prev;
    // Conversion BCD -> binaire si necessaire (bit 2 de B a 0 => BCD).
    if status_b & 0x04 == 0 {
        dt.second = bcd_to_bin(dt.second);
        dt.minute = bcd_to_bin(dt.minute);
        dt.hour = bcd_to_bin(dt.hour & 0x7F) | (dt.hour & 0x80);
        dt.day = bcd_to_bin(dt.day);
        dt.month = bcd_to_bin(dt.month);
        dt.year = bcd_to_bin(dt.year as u8) as u16;
    }
    // Format 12h -> 24h si necessaire (bit 1 de B a 0 => 12h).
    if status_b & 0x02 == 0 && (dt.hour & 0x80) != 0 {
        dt.hour = ((dt.hour & 0x7F) + 12) % 24;
    }
    dt.year += 2000; // RTC ne donne que les 2 derniers chiffres
    dt
}
