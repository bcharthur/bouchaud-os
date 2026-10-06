// Immutable RAMFS snapshot used by one persistence transaction.

struct SnapshotEntree {
    chemin: String,
    contenu: Vec<u8>,
}

/// Synchronisations qui ont du ecarter un cache pour tenir dans la zone.
static TX_CACHES_ECARTES: AtomicU64 = AtomicU64::new(0);

/// Ce que la zone peut porter : `ENTREES_MAX` fichiers, et le contenu d'une
/// demi-zone (sa table reservee deduite).
const LIMITES_ZONE: crate::fs::cache_jetable::Limites = crate::fs::cache_jetable::Limites {
    entrees_max: ENTREES_MAX,
    secteurs_max: SECTEURS_DEMI - SECTEURS_TABLE,
    taille_secteur: SECTOR_SIZE,
};

/// Retire de la collecte les arbres `CACHEDIR.TAG` qu'il faut abandonner pour
/// que le reste tienne. BOUCHAUD_PERSIST_CACHE_JETABLE_V1.
///
/// Sans zone, rien n'est ecarte : il n'y a pas de borne a respecter, et
/// `synchronise_snapshot` rendra `SANS_ZONE` de toute facon.
fn ecarte_les_caches_qui_debordent(meta: Vec<Entree>) -> Vec<Entree> {
    use crate::fs::cache_jetable::{selectionne, Fichier, Verdict};
    if debut().is_none() {
        return meta;
    }
    let groupes = meta.iter().filter_map(|e| e.groupe).max().map_or(0, |g| g as usize + 1);
    let fichiers: Vec<Fichier> = meta.iter().map(|e| Fichier { octets: e.longueur, groupe: e.groupe }).collect();
    let mut ecartes = vec![false; groupes];
    match selectionne(&fichiers, &LIMITES_ZONE, &mut ecartes) {
        Verdict::ToutTient => meta,
        Verdict::NeTientPas => {
            // Pas de cache a abandonner, ou pas assez : l'echec qui suit est
            // celui d'un contenu NON jetable, et il doit se voir.
            crate::serial_println!(
                "BOUCHAUD_PERSIST_DEBORDE fichiers={} max={} secteurs_max={} jetable=insuffisant",
                meta.len(), ENTREES_MAX, LIMITES_ZONE.secteurs_max
            );
            meta
        }
        Verdict::Ecarte { arbres, fichiers, octets } => {
            // Une ligne a la premiere fois, puis une sur 64 : un navigateur
            // qui deborde le fait a chaque `fsync`.
            let n = TX_CACHES_ECARTES.fetch_add(1, Ordering::Relaxed);
            if n % 64 == 0 {
                crate::serial_println!(
                    "BOUCHAUD_PERSIST_CACHE_ECARTE arbres={} fichiers={} octets={} total_fichiers={} fois={}",
                    arbres, fichiers, octets, meta.len(), n + 1
                );
            }
            meta.into_iter()
                .filter(|e| !e.groupe.map_or(false, |g| ecartes[g as usize]))
                .collect()
        }
    }
}

fn rassemble_snapshot() -> Vec<SnapshotEntree> {
    let meta = ecarte_les_caches_qui_debordent(rassemble());
    let systeme = fs();
    let mut out = Vec::with_capacity(meta.len());
    for entree in meta {
        let mut contenu = vec![0u8; entree.longueur];
        contenu.copy_from_slice(&systeme.nodes[entree.noeud].content[..entree.longueur]);
        out.push(SnapshotEntree { chemin: entree.chemin, contenu });
    }
    out
}

/// Ce que `/persist` occupe dans sa zone, et ce que la zone peut porter :
/// `(secteurs_utilises, secteurs_max, fichiers, fichiers_max)`, ou `None` sans
/// zone. Sert `statfs` : l'espace libre d'un chemin persistant est celui de
/// la ZONE, pas celui de la memoire vive.
pub fn occupation() -> Option<(u64, u64, usize, usize)> {
    debut()?;
    let meta = rassemble();
    let secteurs: u64 = meta
        .iter()
        .map(|e| crate::fs::cache_jetable::secteurs(e.longueur, SECTOR_SIZE))
        .sum();
    Some((secteurs, LIMITES_ZONE.secteurs_max, meta.len(), ENTREES_MAX))
}
