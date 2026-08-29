// Immutable RAMFS snapshot used by one persistence transaction.

struct SnapshotEntree {
    chemin: String,
    contenu: Vec<u8>,
}

fn rassemble_snapshot() -> Vec<SnapshotEntree> {
    let meta = rassemble();
    let systeme = fs();
    let mut out = Vec::with_capacity(meta.len());
    for entree in meta {
        let mut contenu = vec![0u8; entree.longueur];
        contenu.copy_from_slice(&systeme.nodes[entree.noeud].content[..entree.longueur]);
        out.push(SnapshotEntree { chemin: entree.chemin, contenu });
    }
    out
}
