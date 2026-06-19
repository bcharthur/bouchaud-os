//! Horloge temps reel (RTC) via la CMOS (ports 0x70/0x71).
//!
//! Fournit la date et l'heure courantes. QEMU expose une RTC standard ; les
//! valeurs sont souvent en BCD (selon le registre d'etat B) et converties ici.

use crate::arch::x86_64::ports::{inb, outb};

/// Decalage horaire local applique a l'affichage de l'OS.
///
/// La RTC de QEMU/PC est lue en UTC dans cette implementation. Bouchaud OS
/// affiche l'heure francaise d'ete (UTC+2), ce qui corrige l'ecart observe de
/// 07h30 affichees pour 09h30 locales.
pub const LOCAL_UTC_OFFSET_HOURS: i8 = 2;

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

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 31,
    }
}

fn add_hours(mut dt: DateTime, offset_hours: i8) -> DateTime {
    let mut hour = dt.hour as i16 + offset_hours as i16;
    while hour >= 24 {
        hour -= 24;
        dt.day += 1;
        if dt.day > days_in_month(dt.year, dt.month) {
            dt.day = 1;
            dt.month += 1;
            if dt.month > 12 {
                dt.month = 1;
                dt.year += 1;
            }
        }
    }
    while hour < 0 {
        hour += 24;
        if dt.day > 1 {
            dt.day -= 1;
        } else {
            if dt.month > 1 {
                dt.month -= 1;
            } else {
                dt.month = 12;
                dt.year -= 1;
            }
            dt.day = days_in_month(dt.year, dt.month);
        }
    }
    dt.hour = hour as u8;
    dt
}

/// Lit la date et l'heure courantes depuis la RTC (UTC).
pub fn now_utc() -> DateTime {
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

/// Lit la date et l'heure locales affichees par l'OS (UTC+2).
pub fn now() -> DateTime {
    add_hours(now_utc(), LOCAL_UTC_OFFSET_HOURS)
}
