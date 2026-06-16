//! Pilote disque (block device) — socle.
//!
//! Aucun peripherique bloc n'est encore pilote : le systeme de fichiers est en
//! RAM (volatil). Prochaine etape : driver ATA/virtio-blk, cache, puis un FS
//! persistant (BFS) survivant au reboot.

/// Un disque persistant est-il monte ?
pub fn present() -> bool {
    false
}

/// Affiche l'occupation des systemes de fichiers (commande `df`).
pub fn print_df() {
    use crate::fs::ramfs;
    let fs = ramfs::fs();
    crate::println!("Sys. fichiers   Type     Inodes  Etat");
    crate::println!("ramfs /         RAMFS    {:>3}/{:<3} monte (volatil)",
        fs.used_nodes(), ramfs::MAX_NODES);
    crate::println!("(aucun disque persistant : driver virtio-blk/ATA + BFS a venir)");
}
