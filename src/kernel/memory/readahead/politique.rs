// V14 policy: become useful on the second sequential page, then grow in a
// bounded way. Eight pages are 32 KiB: enough to amortize #PF and ATA work
// without turning one accidental sequential pair into megabytes of I/O.

const RA_START_AFTER: u64 = 2;
const RA_MAX_PAGES: u64 = 16;

#[inline]
fn ra_window(run: u64) -> u64 {
    if run < RA_START_AFTER { 0 }
    else if run < 4 { 2 }
    else if run < 8 { 4 }
    else if run < 16 { 8 }
    else { RA_MAX_PAGES }
}
