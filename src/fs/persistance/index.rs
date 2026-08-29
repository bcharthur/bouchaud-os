// Local index of the last fully committed persistent image.

struct SurDisque {
    chemin: String,
    secteur: u64,
    longueur: usize,
    sceau: (u64, u64),
}

static DISQUE: SpinLock<Vec<SurDisque>> = SpinLock::new(Vec::new());

fn sceau(contenu: &[u8]) -> (u64, u64) {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x9e37_79b9_7f4a_7c15;
    for &octet in contenu {
        a = (a ^ octet as u64).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b ^ octet as u64).wrapping_mul(0x8864_0000_0000_003d);
    }
    (a, b)
}

fn deja_ecrite(index: usize, chemin: &str, secteur: u64, longueur: usize,
    sceau_courant: (u64, u64)) -> bool {
    let disque = DISQUE.lock();
    match disque.get(index) {
        Some(connu) => connu.chemin == chemin
            && connu.secteur == secteur
            && connu.longueur == longueur
            && connu.sceau == sceau_courant,
        None => false,
    }
}

fn oublie_le_disque() { DISQUE.lock().clear(); }
