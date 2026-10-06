// NOTE : commentaires `//` et non `//!` -- ce fichier est inclus tel quel par
// le test hote `tools/fs/test_cache_jetable.rs` (`#[path]`), comme
// `fs/renommage.rs`.
//
// Ce que la persistance peut ABANDONNER quand `/persist` ne tient plus.
//
// BOUCHAUD_PERSIST_CACHE_JETABLE_V1
//
// La zone persistante a deux bornes dures : `ENTREES_MAX` fichiers, et la
// capacite d'une demi-zone. Au-dela, `synchronise_snapshot` rendait -1 et
// n'ecrivait RIEN -- ni le cache, ni les temoins de connexion, ni les
// reglages, ni un fichier telecharge. Tant que Ladybird tournait avec
// `--disable-http-disk-cache`, rien sous `/persist` ne grossissait seul. Le
// cache HTTP, lui, ecrit un fichier par reponse, autant que l'espace annonce
// le permet : il aurait franchi la borne en une soiree, et la persistance
// entiere se serait arretee sans que personne n'ait rien demande d'autre que
// de naviguer.
//
// Un cache est jetable PAR DEFINITION : le perdre coute un telechargement, pas
// une donnee. La convention qui le dit est celle de « Cache Directory Tagging
// Specification » : un fichier `CACHEDIR.TAG` dont le contenu commence par une
// signature fixe marque le dossier qui le porte, et tout son sous-arbre, comme
// regenerable. `tar --exclude-caches`, borg, restic et rsync la respectent.
// Le navigateur pose cette etiquette sur son dossier de cache.
//
// La regle :
//
//   * si tout tient, tout est ecrit -- un cache n'est jamais ecarte sans
//     raison ;
//   * sinon, les arbres etiquetes sont ecartes ENTIERS, le plus gros d'abord,
//     jusqu'a ce que le reste tienne. Jamais un fichier isole : l'index du
//     cache (une base SQLite) vit dans le meme arbre que les reponses qu'il
//     decrit, et en garder l'un sans les autres ferait relire au redemarrage
//     un index qui designe des fichiers absents ;
//   * si le reste ne tient toujours pas, rien n'est ecarte et la
//     synchronisation echoue comme avant : ce qui deborde alors n'est pas
//     jetable, et le taire serait pire.

pub const NOM_ETIQUETTE: &str = "CACHEDIR.TAG";

// Les 43 premiers octets imposes par la specification.
pub const SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

// Le contenu complet que le navigateur ecrit (la specification recommande un
// commentaire apres la signature).
pub const CONTENU_ETIQUETTE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55\n\
# Cache HTTP de Ladybird (Bouchaud OS). Regenerable : la persistance l'ecarte\n\
# en entier quand /persist ne tient plus dans sa zone.\n";

// Un fichier `CACHEDIR.TAG` etiquette-t-il son dossier ?
pub fn est_etiquette(nom: &str, contenu: &[u8]) -> bool {
    nom == NOM_ETIQUETTE && contenu.starts_with(SIGNATURE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fichier {
    pub octets: usize,
    // L'arbre etiquete qui le contient (le plus externe), ou `None`.
    pub groupe: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limites {
    pub entrees_max: usize,
    pub secteurs_max: u64,
    pub taille_secteur: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    // Tout tient : rien n'est ecarte.
    ToutTient,
    // Des arbres etiquetes ont ete ecartes (`ecartes[g]`) et le reste tient.
    Ecarte { arbres: u32, fichiers: usize, octets: u64 },
    // Meme sans aucun cache, le contenu ne tient pas. Rien n'est ecarte.
    NeTientPas,
}

#[inline]
pub fn secteurs(octets: usize, taille_secteur: usize) -> u64 {
    ((octets + taille_secteur - 1) / taille_secteur) as u64
}

// Decide quels arbres etiquetes ecarter.
//
// `ecartes` est indexe par numero de groupe et doit couvrir tous les groupes
// presents dans `fichiers` ; il est remis a `false` puis rempli. Aucune
// allocation : le noyau l'appelle sous le verrou de transaction.
pub fn selectionne(fichiers: &[Fichier], limites: &Limites, ecartes: &mut [bool]) -> Verdict {
    for e in ecartes.iter_mut() {
        *e = false;
    }
    let mut entrees = fichiers.len();
    let mut total: u64 = 0;
    for f in fichiers {
        total += secteurs(f.octets, limites.taille_secteur);
    }
    let tient = |entrees: usize, total: u64| entrees <= limites.entrees_max && total <= limites.secteurs_max;
    if tient(entrees, total) {
        return Verdict::ToutTient;
    }

    // Le plus gros arbre restant d'abord : c'est celui dont l'abandon libere
    // le plus pour une seule perte. Quadratique en nombre d'arbres -- il y en
    // a un par navigateur.
    let (mut arbres, mut fichiers_ecartes, mut octets_ecartes) = (0u32, 0usize, 0u64);
    loop {
        let mut meilleur: Option<(usize, u64, usize, u64)> = None; // (groupe, secteurs, fichiers, octets)
        for g in 0..ecartes.len() {
            if ecartes[g] {
                continue;
            }
            let (mut s, mut n, mut o) = (0u64, 0usize, 0u64);
            for f in fichiers {
                if f.groupe == Some(g as u32) {
                    s += secteurs(f.octets, limites.taille_secteur);
                    n += 1;
                    o += f.octets as u64;
                }
            }
            if n == 0 {
                continue;
            }
            // A secteurs egaux, celui qui libere le plus d'entrees.
            let mieux = match meilleur {
                None => true,
                Some((_, ms, mn, _)) => s > ms || (s == ms && n > mn),
            };
            if mieux {
                meilleur = Some((g, s, n, o));
            }
        }
        let Some((g, s, n, o)) = meilleur else {
            // Plus rien de jetable et ca ne tient toujours pas.
            for e in ecartes.iter_mut() {
                *e = false;
            }
            return Verdict::NeTientPas;
        };
        ecartes[g] = true;
        arbres += 1;
        fichiers_ecartes += n;
        octets_ecartes += o;
        entrees -= n;
        total -= s;
        if tient(entrees, total) {
            return Verdict::Ecarte { arbres, fichiers: fichiers_ecartes, octets: octets_ecartes };
        }
    }
}
