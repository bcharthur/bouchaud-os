// V14 policy: become useful on the second sequential page, then grow in a
// bounded way. Sixteen pages are 64 KiB: enough to amortize #PF and ATA work
// without turning one accidental sequential pair into megabytes of I/O.
//
// La fenetre est passee de 8 a 16 pages, mais ce commentaire est reste a
// « huit pages, 32 KiB » -- et le garde-fou qui epinglait la valeur n'a rien
// dit, parce que rien ne l'executait. Les deux sont corriges ensemble : une
// constante dont le commentaire ment est pire qu'une constante sans
// commentaire.

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
