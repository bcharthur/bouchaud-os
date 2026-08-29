/// Tous les fichiers sous `/persist`, chemins relatifs a cette racine.
fn rassemble() -> Vec<Entree> {
    let systeme = fs();
    let racine = match systeme.resolve(RACINE, 0) {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let mut entrees = Vec::new();
    collecte(racine, &String::new(), &mut entrees);
    entrees
}

fn collecte(dossier: usize, prefixe: &str, entrees: &mut Vec<Entree>) {
    let systeme = fs();
    // Les indices sont releves d'abord : la collecte n'ecrit pas, mais elle
    // emprunte le systeme de fichiers a chaque tour, et garder un iterateur
    // ouvert par-dessus serait fragile.
    let mut enfants = Vec::new();
    for index in 0..systeme.nodes.len() {
        if systeme.nodes[index].used && systeme.nodes[index].parent == dossier
            && index != dossier
        {
            enfants.push(index);
        }
    }

    for index in enfants {
        let nom = systeme.nodes[index].name_str();
        let chemin = if prefixe.is_empty() {
            String::from(nom)
        } else {
            format!("{}/{}", prefixe, nom)
        };
        match systeme.nodes[index].kind {
            NodeKind::Dir => collecte(index, &chemin, entrees),
            NodeKind::File => {
                let longueur = systeme.nodes[index].content_len();
                if longueur == 0 || chemin.len() >= CHEMIN_MAX {
                    continue;
                }
                // Le contenu reste ou il est : `synchronise` le lit dans le
                // RAMFS au moment d'ecrire, et pour les entrees inchangees il
                // ne le lit que pour en calculer le sceau.
                entrees.push(Entree { chemin, noeud: index, longueur });
            }
        }
    }
}

/// Cree (dossiers compris) puis remplit un fichier sous `/persist`.
fn depose(racine: usize, chemin: &str, contenu: &[u8]) -> bool {
    let systeme = fs();
    let mut parent = racine;
    let mut morceaux = chemin.split('/').filter(|m| !m.is_empty()).peekable();

    while let Some(morceau) = morceaux.next() {
        if morceaux.peek().is_none() {
            let noeud = match systeme.find_child(parent, morceau) {
                Some(idx) => idx,
                None => match systeme.touch_at(parent, morceau) {
                    Ok(idx) => idx,
                    Err(_) => return false,
                },
            };
            return systeme.write_node_bytes(noeud, contenu);
        }
        parent = match systeme.find_child(parent, morceau) {
            Some(idx) => idx,
            None => match systeme.mkdir_at(parent, morceau) {
                Ok(idx) => idx,
                Err(_) => return false,
            },
        };
    }
    false
}
