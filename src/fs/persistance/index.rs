// Contenus confirmes dans chacune des deux demi-zones. La comparaison
// exacte evite qu'une collision ne puisse masquer une ecriture due.
struct SurDisque {
    chemin: String,
    secteur: u64,
    contenu: Vec<u8>,
}

type IndexDemi = Vec<Option<SurDisque>>;
const CACHE_OCTETS_PAR_DEMI: usize = 4 * 1024 * 1024;
static DISQUE: SpinLock<[IndexDemi; 2]> = SpinLock::new([Vec::new(), Vec::new()]);

fn oublie_le_disque() {
    let ancien = core::mem::take(&mut *DISQUE.lock());
    drop(ancien);
}
