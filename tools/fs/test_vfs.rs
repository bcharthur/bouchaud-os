//! BOUCHAUD_VFS_MONTAGES_V1 -- la resolution par point de montage, sur l'hote.

extern crate alloc;

#[path = "../../src/fs/vfs/chemins.rs"]
mod chemins;
use chemins::{ajoute_absents, inode_de, sous_point};

#[test]
fn un_point_ne_prefixe_qu_a_une_frontiere_de_composant() {
    assert_eq!(sous_point("/proc", "/proc/meminfo"), Some("meminfo"));
    assert_eq!(sous_point("/proc", "/proc"), Some(""));
    assert_eq!(sous_point("/proc", "/processus/x"), None);
    assert_eq!(sous_point("/dev", "/devices"), None);
    assert_eq!(sous_point("/proc", "/proc/self/status"), Some("self/status"));
}

#[test]
fn la_racine_couvre_tout() {
    assert_eq!(sous_point("/", "/etc/hosts"), Some("etc/hosts"));
    assert_eq!(sous_point("/", "/"), Some(""));
    assert_eq!(sous_point("/", "relatif"), None);
}

#[test]
fn l_inode_est_stable_et_distingue() {
    assert_eq!(inode_de("meminfo"), inode_de("meminfo"));
    assert_ne!(inode_de("meminfo"), inode_de("uptime"));
    assert_ne!(inode_de(""), 0);
}

#[test]
fn l_union_garde_le_ramfs_et_ajoute_le_vivant_sans_doublon() {
    // `/proc` : l'instantane RAMFS de demarrage (cpuinfo, meminfo) et procfs
    // (meminfo vivant, pids). `meminfo` n'apparait qu'une fois.
    let mut base = vec![("cpuinfo", false), ("meminfo", false)];
    ajoute_absents(&mut base, vec![("meminfo", false), ("1", true), ("42", true), ("42", true)], |e| e.0);
    assert_eq!(base, vec![("cpuinfo", false), ("meminfo", false), ("1", true), ("42", true)]);
}

#[test]
fn l_union_d_un_repertoire_vide_est_le_systeme_monte() {
    let mut base: Vec<(&str, bool)> = Vec::new();
    ajoute_absents(&mut base, vec![("null", false), ("input", true)], |e| e.0);
    assert_eq!(base, vec![("null", false), ("input", true)]);
}
