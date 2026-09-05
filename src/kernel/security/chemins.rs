// NOTE: commentaires `//` et non `//!`.
//
// `tools/security/test_bac_a_sable_navigateur.rs` inclut ce fichier dans un
// `mod chemins { include!(...) }`, et un attribut INTERNE -- ce qu'est `//!` --
// ne peut pas provenir d'une expansion de macro. C'est la meme contrainte qui
// s'applique deja a `profile.rs`, inclus de la meme facon.
//
// Ou un role sandboxe a le droit de lire, et ou il a le droit d'ecrire.
//
// # Pourquoi ces predicats vivent seuls
//
// Ils etaient dans `filesystem.rs`, entre le ramfs, la table de descripteurs
// et la tache courante. Aucun test hote ne pouvait donc les exercer : le
// fichier ne se compile qu'avec le noyau. Or ce sont des fonctions du seul
// texte d'un chemin, et ce qu'elles decident -- ce qu'un moteur de rendu
// compromis peut ecrire -- est precisement ce qu'on veut pouvoir affirmer.
//
// `tools/security/test_bac_a_sable_navigateur.rs` les inclut telles quelles,
// comme il inclut deja `profile.rs`.
//
// # Ce que le decoupage par PROFIL corrige
//
// BOUCHAUD_C19_PROFIL_PERSISTANT_DU_NAVIGATEUR
//
// La liste etait unique : tous les roles sandboxes lisaient les memes
// repertoires et ecrivaient dans les memes. `/persist` n'y figurait pas, et
// la couche plateforme du portage Ladybird demandait pourtant un profil
// PERSISTANT :
//
//     BROWSER_HOST_PATHS profile=/persist/ladybird/profile downloads=/persist/Downloads
//
// RequestServer -- le seul role qui ecrit ce profil -- se le voyait refuser :
//
//     [SECURITY-DENY] pid=6 op=open-path path=/persist/ladybird/profile/cache/alt-svc-cache.txt
//     Unable to create disk cache: mkdir: Permission denied (errno=13)
//
// soixante-cinq refus par session, aucun cache HTTP sur disque, aucun
// cookie et aucun HSTS conserves d'un demarrage a l'autre.
//
// Refuser cette ecriture n'etait pas « plus sur ». HSTS ne protege que s'il
// est RETENU : un magasin qu'on ne peut pas ecrire ramene chaque demarrage a
// la premiere visite, celle ou l'attaque par retrogradation est possible. Ce
// que l'on gagnait en confinement, on le perdait en protection reelle.
//
// Le droit est donc accorde a `BrowserNetwork` -- RequestServer -- et a lui
// seul. Les roles de RENDU (WebContent, WebWorker, ImageDecoder), ceux qui
// executent le script d'un site et decodent ses images, gardent zero acces au
// stockage persistant : un rendu compromis ne doit pas pouvoir survivre a un
// redemarrage. C'est la frontiere qui compte, et elle ne bouge pas ici.

use super::profile::SecurityProfile;

/// Le profil persistant du navigateur, tel que la couche plateforme du portage
/// Ladybird le place (`tools/ladybird/prepare-platform-complete.py`).
///
/// Un seul prefixe couvre tout ce que RequestServer y ecrit : `--profile-path`
/// y met le cache HTTP et le cache alt-svc, `XDG_DATA_HOME` les magasins SQL,
/// `XDG_CACHE_HOME` le reste. Les enumerer separement ferait deux sources de
/// verite pour une seule arborescence.
pub const PROFIL_NAVIGATEUR: &str = "/persist/ladybird";

/// `path` est-il `root` lui-meme, ou un descendant ?
///
/// La comparaison va jusqu'au SEPARATEUR : sans cela, `/tmpfoo` passerait pour
/// un descendant de `/tmp`, et un prefixe accorde ouvrirait ses voisins de nom.
pub fn sous_arbre(path: &str, root: &str) -> bool {
    path == root
        || (path.starts_with(root)
            && path.as_bytes().get(root.len()).copied() == Some(b'/'))
}

/// Ce que TOUT role sandboxe peut lire.
fn lecture_commune(path: &str) -> bool {
    sous_arbre(path, "/usr")
        || sous_arbre(path, "/lib")
        || sous_arbre(path, "/etc")
        || sous_arbre(path, "/tmp")
        || sous_arbre(path, "/var/tmp")
        || sous_arbre(path, "/proc/self")
        || sous_arbre(path, "/dev/shm")
        || path == "/dev/null"
        || path == "/dev/zero"
        || path == "/dev/urandom"
}

/// Ce que TOUT role sandboxe peut ecrire.
fn ecriture_commune(path: &str) -> bool {
    sous_arbre(path, "/tmp")
        || sous_arbre(path, "/var/tmp")
        || sous_arbre(path, "/dev/shm")
        || path == "/dev/null"
}

/// Ce role possede-t-il le profil persistant du navigateur ?
const fn possede_le_profil(profile: SecurityProfile) -> bool {
    matches!(profile, SecurityProfile::BrowserNetwork)
}

pub fn lecture_permise(profile: SecurityProfile, path: &str) -> bool {
    if lecture_commune(path) {
        return true;
    }
    if !possede_le_profil(profile) {
        return false;
    }
    // Le repertoire `/persist` LUI-MEME est lisible pour ce role : un magasin
    // qui commence par verifier que son volume existe echouerait sinon avant
    // d'atteindre son propre sous-arbre. Sa racine reste en revanche
    // inscriptible par personne -- c'est la couche plateforme, non sandboxee,
    // qui la peuple au demarrage.
    path == "/persist" || sous_arbre(path, PROFIL_NAVIGATEUR)
}

pub fn ecriture_permise(profile: SecurityProfile, path: &str) -> bool {
    if ecriture_commune(path) {
        return true;
    }
    possede_le_profil(profile) && sous_arbre(path, PROFIL_NAVIGATEUR)
}
