/// Ce nœud est-il sous [`RACINE`] ?
///
/// C'est ce qui permet a `fsync` de n'ecrire sur le disque que lorsque le
/// descripteur en cause designe vraiment un fichier persistant : les programmes
/// appellent `fsync` sans compter, et chacun coute sinon une reecriture de toute
/// la zone.
pub fn sous_racine(mut noeud: usize) -> bool {
    let systeme = fs();
    let racine = match systeme.resolve(RACINE, 0) {
        Some(idx) => idx,
        None => return false,
    };
    // La remontee est bornee par le nombre de nœuds : un cycle dans les parents
    // ne doit pas faire boucler le noyau.
    for _ in 0..systeme.nodes.len() {
        if noeud == racine {
            return true;
        }
        let parent = systeme.nodes[noeud].parent;
        if parent == noeud {
            return false;
        }
        noeud = parent;
    }
    false
}


/// Un fichier retenu : son chemin sous [`RACINE`] et OU le trouver.
///
/// Le contenu n'est plus recopie a la collecte. Il l'etait pour tous les
/// fichiers, a chaque `fsync`, et la plupart ne sont pas ecrits : voir
/// [`synchronise`].
struct Entree {
    chemin: String,
    noeud: usize,
    longueur: usize,
}
