//! Fonctions PURES du VFS (BOUCHAUD_VFS_MONTAGES_V1) : `tools/fs/test_vfs.rs`.

/// Le reste de `chemin` sous `point`, si `point` le prefixe a une frontiere
/// de composant. Pur : teste sur l'hote.
pub fn sous_point<'a>(point: &str, chemin: &'a str) -> Option<&'a str> {
    if point == "/" {
        return chemin.strip_prefix('/');
    }
    let reste = chemin.strip_prefix(point)?;
    if reste.is_empty() {
        Some("")
    } else {
        reste.strip_prefix('/')
    }
}

/// Inode stable d'un chemin relatif (FNV-1a) : le meme fichier a la meme
/// identite d'une ouverture a l'autre, sans table a tenir.
pub fn inode_de(relatif: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in relatif.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h | 1
}


/// BOUCHAUD_VFS_DOSSIERS_MONTES_V1 -- l'union d'un repertoire RAMFS et du
/// systeme monte dessus : ajoute a `base` les entrees de `montees` dont le nom
/// n'y figure pas encore (le RAMFS garde son entree : meme nom, une seule
/// ligne dans `ls`, et c'est l'ouverture qui choisit le systeme monte).
/// Ordre stable : celui de `base`, puis celui de `montees`. Pur.
pub fn ajoute_absents<T>(base: &mut alloc::vec::Vec<T>, montees: impl IntoIterator<Item = T>, nom: impl Fn(&T) -> &str) {
    for entree in montees {
        if !base.iter().any(|b| nom(b) == nom(&entree)) {
            base.push(entree);
        }
    }
}
