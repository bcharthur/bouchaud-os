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

/// Le profil persistant du navigateur, tel que le processus navigateur le place
/// (`tools/ladybird/ui-bouchaud/main.cpp`).
///
/// Le profil `default` d'upstream sous les racines XDG (BOUCHAUD_PROFIL_XDG_V1) :
/// `XDG_CACHE_HOME=/persist/ladybird/cache` (cache HTTP, alt-svc : a
/// RequestServer), `XDG_DATA_HOME=/persist/ladybird/data` (magasins SQL : au
/// processus navigateur), `XDG_CONFIG_HOME=/persist/ladybird/config`
/// (reglages : au processus navigateur). BOUCHAUD_PROFIL_PAR_ROLE_V1 : aucun
/// role sandboxe ne possede plus l'arborescence entiere.
pub const PROFIL_NAVIGATEUR: &str = "/persist/ladybird";

/// Ou le navigateur depose ce que l'utilisateur telecharge.
///
/// BOUCHAUD_C20_TELECHARGEMENTS, BOUCHAUD_UI_V1
///
/// Aucun role SANDBOXE n'y a acces. Le fichier est ecrit par le processus
/// navigateur (`BouchaudBrowserHost`, profil `BrowserBroker`, non sandboxe),
/// par `WebView::FileDownloader` upstream : il reprend la requete a
/// RequestServer et ecrit lui-meme. Le droit que WebContent recevait ici
/// existait parce que le chrome vivait DANS WebContent ; il est parti avec lui
/// (`docs/ladybird/UI_BOUCHAUD.md`).
///
/// Ce qui y atterrit reste `Untrusted` a l'execution : `security::profile`
/// classe ainsi tout binaire lance depuis un chemin contenant `/Downloads/`.
pub const DOSSIER_TELECHARGEMENTS: &str = "/persist/Downloads";

/// Ou le chrome du navigateur garde l'historique et les favoris.
///
/// BOUCHAUD_C21_HISTORIQUE_ET_FAVORIS, BOUCHAUD_UI_V1
///
/// Voisin de nom du profil -- `/persist/ladybird-chrome` a cote de
/// `/persist/ladybird` -- et deliberement : la comparaison de `sous_arbre` va
/// jusqu'au separateur, donc l'un n'ouvre pas l'autre.
///
/// Aucun role SANDBOXE n'y a acces. L'historique est une donnee PRIVEE et un
/// favori une cible de navigation que l'utilisateur a choisie : un rendu
/// compromis qui pourrait les lire ou les reecrire ferait de l'espionnage ou
/// de l'hameconnage durable. Le chrome vit maintenant dans le processus
/// navigateur (UI/Bouchaud), qui n'est pas sandboxe ; le droit qu'avait
/// WebContent est retire.
pub const MAGASIN_DU_CHROME: &str = "/persist/ladybird-chrome";

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
        // `/proc/sys/vm` et `/proc/sys/kernel` : le noyau les PUBLIE pour les
        // allocateurs (`sysroot.rs` les cree en 0444), puis le bac a sable en
        // refusait la lecture. Le releve physique du 12 septembre montre le
        // resultat : trois `SECURITY-DENY ... path=/proc/sys/vm/overcommit_memory`
        // a chaque lancement du navigateur, un par processus.
        //
        // Ce sont des valeurs de configuration fixes -- `0`, `65530`,
        // `6.1.0-bouchaud` -- que la moindre machine Linux rend a tout le
        // monde. Les deux sous-arbres sont NOMMES plutot que `/proc/sys`
        // entier : un `/proc/sys` futur qui porterait quelque chose de
        // sensible ne serait pas accorde par accident.
        || sous_arbre(path, "/proc/sys/vm")
        || sous_arbre(path, "/proc/sys/kernel")
        // BOUCHAUD_C24_TOPOLOGIE_CPU
        //
        // La topologie des processeurs. Le releve physique montre
        // `/sys/devices/system/cpu/online` refuse, et ce refus coute cher :
        // une bibliotheque a qui l'on ferme `/sys` retombe sur une
        // heuristique, et cette heuristique vaut souvent UN. Sur une machine
        // qui ordonnance seize processeurs, chaque pool de threads du
        // navigateur naissait alors a un fil.
        //
        // `/proc/stat` pour la meme raison : c'est la seconde source que les
        // compteurs de processeurs consultent quand la premiere leur est
        // fermee.
        //
        // Ce sont des valeurs de TOPOLOGIE -- combien de processeurs, combien
        // de coeurs --, que la moindre machine Linux rend a tout le monde, et
        // que le processus apprendrait de toute facon en comptant ses propres
        // migrations. Les chemins sont NOMMES et non `/sys` entier : un `/sys`
        // futur qui porterait de la donnee de peripherique ne serait pas
        // accorde par accident.
        || sous_arbre(path, "/sys/devices/system/cpu")
        || path == "/proc/stat"
        || path == "/proc/cpuinfo"
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

// BOUCHAUD_PROFIL_PAR_ROLE_V1 (convergence P2)
//
// RequestServer possedait TOUT `/persist/ladybird` : configuration, donnees
// (la base SQL ou le processus navigateur range cookies et stockage local)
// et cache. Or il n'utilise que son `cache_path` (Services/RequestServer/
// main.cpp : cache disque et `alt-svc-cache.txt`) -- le bac a sable upstream
// le restreint d'ailleurs a ce seul chemin. Il ne possede plus que le CACHE.
//
// Et en mode EPHEMERE (bureau live, `BOUCHAUD_LADYBIRD_EPHEMERAL`), le profil
// vit sous `/tmp/ladybird-*` -- que `lecture_commune`/`ecriture_commune`
// ouvraient a TOUS les roles sandboxes : un WebContent compromis lisait la
// base des cookies et le cache HTTP d'un autre site. Ces racines sont
// desormais PRIVEES, quel que soit le mode : seul le processus navigateur
// (non sandboxe) y a acces, et RequestServer a son cache.

/// Les racines du profil du navigateur, persistant puis ephemere
/// (`tools/ladybird/ui-bouchaud/main.cpp`). Le runtime
/// (`/tmp/ladybird-runtime`, sockets) n'en fait pas partie.
const RACINES_PRIVEES: [&str; 5] = [
    PROFIL_NAVIGATEUR,
    "/tmp/ladybird",
    "/tmp/ladybird-config",
    "/tmp/ladybird-data",
    "/tmp/ladybird-cache",
];

/// Le cache HTTP (`XDG_CACHE_HOME`), persistant puis ephemere : le seul
/// morceau du profil qui appartient a RequestServer.
pub const CACHES_RESEAU: [&str; 2] = ["/persist/ladybird/cache", "/tmp/ladybird-cache"];

fn dans_le_profil_prive(path: &str) -> bool {
    RACINES_PRIVEES.iter().any(|racine| sous_arbre(path, racine))
}

fn dans_le_cache_reseau(path: &str) -> bool {
    CACHES_RESEAU.iter().any(|racine| sous_arbre(path, racine))
}

/// `path` est-il un ancetre STRICT d'un cache reseau (`/persist`,
/// `/persist/ladybird`) ? `Core::Directory::ensure_directory` cree chaque
/// ancetre en ignorant EEXIST : le role reseau doit les VOIR
/// (BOUCHAUD_MKDIR_EEXIST_AVANT_EACCES_V1), sans pouvoir y ecrire ni lire ce
/// qu'ils contiennent d'autre.
fn ancetre_du_cache(path: &str) -> bool {
    CACHES_RESEAU.iter().any(|racine| {
        racine.len() > path.len()
            && racine.starts_with(path)
            && (path == "/" || racine.as_bytes()[path.len()] == b'/')
    })
}

/// Ce role possede-t-il le cache HTTP du navigateur ?
const fn possede_le_cache(profile: SecurityProfile) -> bool {
    matches!(profile, SecurityProfile::BrowserNetwork)
}

pub fn lecture_permise(profile: SecurityProfile, path: &str) -> bool {
    if path == "/" {
        return false;
    }
    if dans_le_profil_prive(path) {
        return possede_le_cache(profile) && (dans_le_cache_reseau(path) || ancetre_du_cache(path));
    }
    if lecture_commune(path) {
        return true;
    }
    // Le repertoire `/persist` LUI-MEME est lisible pour le role reseau : un
    // magasin qui commence par verifier que son volume existe echouerait sinon
    // avant d'atteindre son propre sous-arbre. Sa racine reste en revanche
    // inscriptible par personne -- c'est la couche plateforme, non sandboxee,
    // qui la peuple au demarrage.
    possede_le_cache(profile) && ancetre_du_cache(path)
}

/// La sortie audio, en ECRITURE seulement, pour le role de rendu.
///
/// BOUCHAUD_AUDIO_DSP_V1 (P8). LibMedia joue le son DANS WebContent
/// (`PlaybackStream`, `tools/ladybird/audio/PlaybackStreamBouchaud.cpp`) :
/// sans ce droit, chaque `<audio>` retombait sur la sortie nulle. Ce qui est
/// accorde est le strict necessaire : ouvrir `/dev/dsp` pour y ECRIRE du PCM
/// et le regler (ioctls OSS). Ni lecture (une capture future ne serait pas
/// accordee par accident), ni les autres noms du peripherique, ni le role
/// reseau. Le profil de rendu est partage avec ImageDecoder, WebWorker et le
/// Compositor : un rendu compromis peut donc faire du bruit -- c'est le prix,
/// et il est le meme qu'avec une socket PulseAudio sous Linux.
const fn joue_du_son(profile: SecurityProfile) -> bool {
    matches!(profile, SecurityProfile::BrowserContent)
}

pub fn ecriture_permise(profile: SecurityProfile, path: &str) -> bool {
    if dans_le_profil_prive(path) {
        return possede_le_cache(profile) && dans_le_cache_reseau(path);
    }
    if ecriture_commune(path) {
        return true;
    }
    joue_du_son(profile) && path == "/dev/dsp"
}
