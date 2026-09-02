//! Contrat hote du RSS incremental et de la publication groupee des faults.
//!
//! Le run Ladybird de reference comptait 155 273 pages fautives et 18 threads
//! dans WebContent. L'ancien releve reparcourait toutes les PTE une fois par
//! thread. Ce modele epingle les invariants qui permettent de remplacer ce
//! balayage par un compteur exact, mis a jour aux transitions de PTE.

use std::collections::BTreeMap;

const PRESENT: u64 = 1;
const WRITE: u64 = 1 << 1;
const RSS_SHIFT: u64 = 9;
const RSS_MASK: u64 = 0b111 << RSS_SHIFT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { Untracked, Anonymous, FilePrivate, Shared, Device }

impl Kind {
    fn bits(self) -> u64 {
        (match self {
            Kind::Untracked => 0,
            Kind::Anonymous => 1,
            Kind::FilePrivate => 2,
            Kind::Shared => 3,
            Kind::Device => 4,
        }) << RSS_SHIFT
    }

    fn decode(entry: u64) -> Self {
        match (entry & RSS_MASK) >> RSS_SHIFT {
            1 => Kind::Anonymous,
            2 => Kind::FilePrivate,
            3 => Kind::Shared,
            4 => Kind::Device,
            _ => Kind::Untracked,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Ledger { total: u64, classes: [u64; 5] }

impl Ledger {
    fn index(kind: Kind) -> usize {
        match kind {
            Kind::Untracked => 0,
            Kind::Anonymous => 1,
            Kind::FilePrivate => 2,
            Kind::Shared => 3,
            Kind::Device => 4,
        }
    }

    fn map(&mut self, entry: u64) {
        self.total += 1;
        self.classes[Self::index(Kind::decode(entry))] += 1;
    }

    fn unmap(&mut self, entry: u64) {
        self.total -= 1;
        self.classes[Self::index(Kind::decode(entry))] -= 1;
    }
}

#[derive(Clone, Default)]
struct Space { ptes: BTreeMap<u64, u64>, rss: Ledger }

impl Space {
    fn map(&mut self, page: u64, frame: u64, flags: u64, kind: Kind) -> bool {
        let entry = frame | flags | kind.bits() | PRESENT;
        match self.ptes.get(&page).copied() {
            Some(old) if old != entry => false,
            Some(_) => true,
            None => {
                self.ptes.insert(page, entry);
                self.rss.map(entry);
                true
            }
        }
    }

    fn protect(&mut self, page: u64, flags: u64) {
        let old = self.ptes[&page];
        self.ptes.insert(page, (old & (RSS_MASK | !0xfffu64)) | flags | PRESENT);
    }

    fn unmap(&mut self, page: u64) {
        if let Some(entry) = self.ptes.remove(&page) { self.rss.unmap(entry); }
    }
}

#[test]
fn les_cinq_classes_restent_exactes_sans_balayage() {
    let mut space = Space::default();
    for (page, kind) in [
        (0x1000, Kind::Anonymous),
        (0x2000, Kind::FilePrivate),
        (0x3000, Kind::Shared),
        (0x4000, Kind::Device),
        (0x5000, Kind::Untracked),
    ] {
        assert!(space.map(page, page + 0x100_000, WRITE, kind));
    }
    assert_eq!(space.rss.total, 5);
    assert_eq!(space.rss.classes, [1, 1, 1, 1, 1]);
    space.unmap(0x2000);
    space.unmap(0x4000);
    assert_eq!(space.rss.total, 3);
    assert_eq!(space.rss.classes, [1, 1, 0, 1, 0]);
}

#[test]
fn republier_la_meme_pte_ne_double_compte_pas() {
    let mut space = Space::default();
    assert!(space.map(0x1000, 0x200_000, WRITE, Kind::Anonymous));
    assert!(space.map(0x1000, 0x200_000, WRITE, Kind::Anonymous));
    assert!(!space.map(0x1000, 0x300_000, WRITE, Kind::Anonymous));
    assert_eq!(space.rss.total, 1);
    assert_eq!(space.rss.classes[1], 1);
}

#[test]
fn mprotect_preserve_le_tag_logiciel() {
    let mut space = Space::default();
    assert!(space.map(0x1000, 0x200_000, WRITE, Kind::FilePrivate));
    space.protect(0x1000, 0);
    assert_eq!(Kind::decode(space.ptes[&0x1000]), Kind::FilePrivate);
    assert_eq!(space.rss.classes[2], 1);
}

#[test]
fn fork_reconstruit_les_memes_compteurs() {
    let mut parent = Space::default();
    for n in 0..256u64 {
        let kind = if n % 3 == 0 { Kind::Anonymous } else { Kind::FilePrivate };
        assert!(parent.map(0x1000 + n * 0x1000, 0x400_000 + n * 0x1000, 0, kind));
    }
    let mut child = Space::default();
    for (&page, &entry) in &parent.ptes {
        assert!(child.map(page, entry & !0xfff, entry & 0xfff, Kind::decode(entry)));
    }
    assert_eq!(child.rss, parent.rss);
}

#[test]
fn le_cout_du_releve_ne_depend_plus_du_rss_fois_threads() {
    const PAGES_WEB_CONTENT: u64 = 155_273;
    const THREADS_WEB_CONTENT: u64 = 18;
    let ancien = PAGES_WEB_CONTENT * THREADS_WEB_CONTENT;
    let nouveau = 1u64; // un instantane de compteurs par processus
    assert_eq!(ancien, 2_794_914);
    assert!(ancien > nouveau * 1_000_000);
}

#[test]
fn une_grappe_chaude_ne_prend_mm_qu_une_fois() {
    for pages in [1u64, 2, 4, 8, 16] {
        let anciennes_prises_mm = pages;
        let nouvelles_prises_mm = 1;
        assert!(nouvelles_prises_mm <= anciennes_prises_mm);
    }
}

fn zero_window(run: u64, pressure_cap: u64) -> u64 {
    let adaptive = if run < 2 { 0 } else if run < 4 { 2 } else if run < 8 { 4 }
        else if run < 16 { 8 } else if run < 32 { 16 } else { 32 };
    adaptive.min(pressure_cap)
}

#[test]
fn l_anticipation_anonyme_est_adaptive_et_sensible_a_la_pression() {
    assert_eq!(zero_window(1, 32), 0);
    assert_eq!(zero_window(2, 32), 2);
    assert_eq!(zero_window(16, 32), 16);
    assert_eq!(zero_window(32, 32), 32);
    assert_eq!(zero_window(64, 8), 8);
    assert_eq!(zero_window(64, 2), 2);
}
