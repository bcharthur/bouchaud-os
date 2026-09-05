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

/// Ou le navigateur depose ce que l'utilisateur telecharge.
///
/// BOUCHAUD_C20_TELECHARGEMENTS
///
/// # Ce que ce droit ouvre, et pourquoi il est accorde quand meme
///
/// Il est accorde a `BrowserContent` -- WebContent, WebWorker, ImageDecoder --
/// c'est-a-dire aux roles qui executent le script d'un site. C'est un
/// elargissement reel, et le nier serait malhonnete : un rendu compromis peut
/// desormais deposer un fichier qui survit au redemarrage.
///
/// Trois choses le bornent, et la troisieme est la vraie raison.
///
/// D'abord le sous-arbre : le controle porte sur le chemin CANONIQUE
/// (`path::normalize_absolute` a deja resolu les `..`), donc un
/// `Content-Disposition: filename="../ladybird/profile/cookies.sqlite"` ne
/// sort pas d'ici. Le chrome assainit en plus le nom propose -- la ceinture
/// et les bretelles, parce que ce nom vient du SERVEUR.
///
/// Ensuite le statut de ce qui y atterrit : `security::profile` classe tout
/// binaire lance depuis un chemin contenant `/Downloads/` comme `Untrusted`.
/// Un fichier depose la ne peut donc pas servir a gagner des droits ; il peut
/// au pire etre execute avec moins de droits que tout le reste.
///
/// Enfin, l'alternative. Sans ce droit, le navigateur ne peut ecrire que dans
/// `/tmp`, qui est en RAMFS : un telechargement disparaitrait au redemarrage.
/// « Le navigateur ne sait pas enregistrer un fichier » n'est pas une
/// propriete de securite, c'est une fonction absente -- et l'utilisateur qui
/// la contourne le fera par un chemin que personne n'a examine.
///
/// La frontiere qui compte ne bouge pas : le PROFIL du navigateur
/// (`/persist/ladybird` : cookies, HSTS, cache) reste ferme aux roles de
/// rendu. Ce qu'ils gagnent est un depot, pas une memoire.
pub const DOSSIER_TELECHARGEMENTS: &str = "/persist/Downloads";

/// Ou le chrome du navigateur garde l'historique et les favoris.
///
/// BOUCHAUD_C21_HISTORIQUE_ET_FAVORIS
///
/// Voisin de nom du profil -- `/persist/ladybird-chrome` a cote de
/// `/persist/ladybird` -- et deliberement : la comparaison de `sous_arbre` va
/// jusqu'au separateur, donc l'un n'ouvre pas l'autre, et le voir dans un `ls`
/// dit tout de suite que ce sont deux choses.
///
/// # Ce que ce droit coute
///
/// Il est accorde au meme role que le depot de telechargement, et pour la meme
/// raison de fond : dans ce portage, le chrome vit DANS WebContent. Mais ce
/// qu'il expose n'est pas de la meme nature, et il faut le dire :
///
///   * l'historique est une donnee PRIVEE. Un rendu compromis y lit ce que
///     l'utilisateur a visite avant, et pas seulement pendant sa session ;
///   * un favori est une cible de NAVIGATION que l'utilisateur a choisie. Le
///     reecrire est un hameconnage durable -- un signet vers sa banque qui
///     mene ailleurs.
///
/// Ce que le chrome oppose : rien n'est relu tel quel. Toute adresse rechargee
/// passe par `BouchaudUrl::acceptable_pour_le_magasin` -- liste blanche de
/// schemas, donc pas de `javascript:` ni de `data:`, et aucun octet de
/// controle. Cela ferme l'execution ; cela ne ferme pas la substitution d'un
/// `https://` par un autre, et rien ici ne le peut.
///
/// Ce qui la fermera est nomme : sortir le chrome de WebContent
/// (`docs/ladybird/AUDIT_INTEGRATION.md` §5). Ce jour-la, ce droit et celui du
/// depot de telechargement partent ensemble, parce qu'ils existent tous deux
/// pour la meme raison.
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

/// Ce role porte-t-il le chrome du navigateur ?
///
/// C'est `BrowserContent` -- WebContent -- et non `BrowserNetwork`, parce que
/// dans ce portage la barre d'outils, l'historique, les favoris et la lecture
/// du corps d'un telechargement vivent tous dans le processus de rendu.
///
/// Les deux sous-arbres qui en dependent -- le depot de telechargement et le
/// magasin du chrome -- existent pour cette seule raison, et repartiront
/// ensemble le jour ou le chrome sortira de WebContent. Un seul predicat le
/// dit, plutot que deux identiques : ce n'est pas deux decisions, c'est une.
const fn depose_les_telechargements(profile: SecurityProfile) -> bool {
    matches!(profile, SecurityProfile::BrowserContent)
}

pub fn lecture_permise(profile: SecurityProfile, path: &str) -> bool {
    if lecture_commune(path) {
        return true;
    }
    // Le role qui depose relit son propre depot : c'est ainsi qu'il decouvre
    // qu'un fichier du meme nom existe deja, et qu'il numerote le suivant au
    // lieu de l'ecraser. Lui refuser la lecture ferait perdre le precedent.
    if depose_les_telechargements(profile)
        && (sous_arbre(path, DOSSIER_TELECHARGEMENTS) || sous_arbre(path, MAGASIN_DU_CHROME))
    {
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
    if depose_les_telechargements(profile)
        && (sous_arbre(path, DOSSIER_TELECHARGEMENTS) || sous_arbre(path, MAGASIN_DU_CHROME))
    {
        return true;
    }
    possede_le_profil(profile) && sous_arbre(path, PROFIL_NAVIGATEUR)
}
