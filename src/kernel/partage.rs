//! Cycle de vie des pages partagees.
//!
//! ## Le probleme qu'il resout
//!
//! `MAP_SHARED` sur un fichier ne peut pas se contenter de copier : deux
//! processus doivent voir les **memes frames physiques**. Le noyau tient donc un
//! cache de pages indexe par (nœud, numero de page), et `mmap` y puise plutot
//! que d'allouer.
//!
//! Ce cache n'evinçait rien. Les frames d'un `memfd` ferme restaient allouees
//! jusqu'a l'extinction de la machine — ce qui est sans consequence tant qu'un
//! `memfd` est cree une fois au demarrage, et ruineux des qu'un compositeur
//! recree sa surface a chaque redimensionnement. Mille surfaces de 4 MiB, et il
//! ne reste plus rien.
//!
//! ## Trois references, pas une
//!
//! Liberer une frame encore mappee corromprait silencieusement la memoire d'un
//! processus vivant — la pire espece de bogue, parce qu'il se manifeste
//! ailleurs et plus tard. Le compte doit donc distinguer :
//!
//! * **les descripteurs ouverts** (`ouverts`) : combien de `FileDesc` designent
//!   ce nœud. `dup`, `fork` et `SCM_RIGHTS` en ajoutent, `close` en retire ;
//! * **les mappages vivants** (`mappages`) : combien de plages `MAP_SHARED`
//!   projettent ce nœud, tous processus confondus. `mmap` en ajoute, `munmap`
//!   et la mort d'un processus en retirent ;
//! * **la possession des pages** : le cache lui-meme, seul proprietaire des
//!   frames. Un espace d'adressage les emprunte par `map_foreign` et ne les
//!   libere jamais avec lui.
//!
//! Les pages ne sont evincees que lorsque **les deux compteurs sont a zero**.
//! Un descripteur ferme dont le mappage reste vivant — le cas normal apres
//! `mmap` d'un `memfd` — ne libere donc rien, et c'est exactement ce que POSIX
//! promet.
//!
//! ## Ce qui n'est pas fait
//!
//! Un `munmap` partiel ne relache pas la reference : la plage est retrecie mais
//! le compte reste. C'est deliberement conservateur — une fuite se mesure, un
//! usage apres liberation se debogue.

use alloc::vec::Vec;

use crate::kernel::vmm::{self, PAGE_SIZE};

/// Ce que le cache sait d'un nœud partage.
struct Partagee {
    node: usize,
    /// Descripteurs ouverts sur ce nœud.
    ouverts: usize,
    /// Plages `MAP_SHARED` vivantes qui le projettent.
    mappages: usize,
    /// Pages possedees par le cache : (numero de page, frame physique).
    pages: Vec<(u64, u64)>,
}

static mut CACHE: Option<Vec<Partagee>> = None;

fn cache() -> &'static mut Vec<Partagee> {
    unsafe {
        let pointeur = &raw mut CACHE;
        if (*pointeur).is_none() {
            *pointeur = Some(Vec::new());
        }
        (*pointeur).as_mut().unwrap()
    }
}

fn position(node: usize) -> Option<usize> {
    cache().iter().position(|entree| entree.node == node)
}

/// L'entree du nœud, creee si besoin.
fn entree(node: usize) -> usize {
    match position(node) {
        Some(index) => index,
        None => {
            cache().push(Partagee { node, ouverts: 0, mappages: 0, pages: Vec::new() });
            cache().len() - 1
        }
    }
}

// --- Comptage ---------------------------------------------------------------

/// Un descripteur de plus designe ce nœud.
pub fn ouvre(node: usize) {
    let index = entree(node);
    cache()[index].ouverts += 1;
}

/// Un descripteur de moins. Peut declencher l'eviction.
pub fn ferme(node: usize) {
    if let Some(index) = position(node) {
        cache()[index].ouverts = cache()[index].ouverts.saturating_sub(1);
        evince_si_orphelin(node);
    }
}

/// Une plage `MAP_SHARED` de plus projette ce nœud.
pub fn mappe(node: usize) {
    let index = entree(node);
    cache()[index].mappages += 1;
}

/// Une plage de moins. Peut declencher l'eviction.
pub fn demappe(node: usize) {
    if let Some(index) = position(node) {
        cache()[index].mappages = cache()[index].mappages.saturating_sub(1);
        evince_si_orphelin(node);
    }
}

// --- Pages ------------------------------------------------------------------

/// Frame partagee d'une page de fichier, creee et remplie au premier acces.
///
/// Le remplissage vient du contenu RAMFS ; au-dela de la fin du fichier la
/// frame est simplement neuve, donc a zero — c'est ce que voit un `mmap` d'un
/// `memfd` plus grand que ce qu'on y a ecrit.
pub fn page(node: usize, numero: u64) -> Option<u64> {
    let index = entree(node);
    if let Some((_, frame)) = cache()[index].pages.iter().find(|(p, _)| *p == numero) {
        return Some(*frame);
    }
    let frame = vmm::alloc_frame()?;
    {
        let fs = crate::fs::ramfs::fs();
        let contenu = &fs.nodes[node].content;
        let debut = (numero * PAGE_SIZE) as usize;
        if debut < contenu.len() {
            let fin = core::cmp::min(contenu.len(), debut + PAGE_SIZE as usize);
            unsafe {
                core::ptr::copy_nonoverlapping(
                    contenu[debut..fin].as_ptr(),
                    crate::kernel::memory::phys_to_virt(frame),
                    fin - debut,
                );
            }
        }
    }
    cache()[index].pages.push((numero, frame));
    Some(frame)
}

/// Recopie les pages partagees d'un nœud vers son contenu RAMFS.
///
/// C'est ce qui rend les ecritures d'un `MAP_SHARED` visibles par un `read`
/// ordinaire. Les frames sont la source de verite ; le contenu n'en est que le
/// reflet.
pub fn writeback(node: usize) {
    let index = match position(node) {
        Some(index) => index,
        None => return,
    };
    let pages = cache()[index].pages.clone();
    if pages.is_empty() {
        return;
    }
    let fs = crate::fs::ramfs::fs();
    for (numero, frame) in pages {
        let debut = (numero * PAGE_SIZE) as usize;
        if debut >= fs.nodes[node].content.len() {
            continue;
        }
        let fin = core::cmp::min(fs.nodes[node].content.len(), debut + PAGE_SIZE as usize);
        unsafe {
            core::ptr::copy_nonoverlapping(
                crate::kernel::memory::phys_to_virt(frame),
                fs.nodes[node].content[debut..fin].as_mut_ptr(),
                fin - debut,
            );
        }
    }
}

/// Recopie tout ce que le cache detient (`msync` sans plage precise).
pub fn writeback_tout() {
    let nœuds: Vec<usize> = cache().iter().map(|entree| entree.node).collect();
    for node in nœuds {
        writeback(node);
    }
}

// --- Eviction ---------------------------------------------------------------

/// Plus aucun descripteur ni mappage : les frames repartent a l'allocateur.
///
/// L'ordre compte. On recopie **avant** de liberer, sinon les dernieres
/// ecritures d'un `MAP_SHARED` sur fichier nomme seraient perdues au moment
/// meme ou le programme croit les avoir enregistrees.
fn evince_si_orphelin(node: usize) {
    let index = match position(node) {
        Some(index) => index,
        None => return,
    };
    if cache()[index].ouverts > 0 || cache()[index].mappages > 0 {
        return;
    }

    let anonyme = crate::fs::ramfs::fs().est_anonyme(node);
    if !anonyme {
        // Un fichier nomme survit a son cache : ses octets doivent y rester.
        writeback(node);
    }

    let entree = cache().swap_remove(index);
    for (_, frame) in entree.pages {
        vmm::free_frame(frame);
    }
    // Un `memfd` n'a plus de nom, plus de descripteur et plus de mappage : rien
    // ne peut plus le designer. Garder son inode serait une seconde fuite, plus
    // discrete que celle des frames mais tout aussi bornante.
    if anonyme {
        crate::fs::ramfs::fs().libere_anonyme(node);
    }
}

// --- Diagnostic -------------------------------------------------------------

/// (nœuds suivis, pages detenues) — pour `shmstat` et les tests.
pub fn statistiques() -> (usize, usize) {
    let liste = cache();
    (liste.len(), liste.iter().map(|entree| entree.pages.len()).sum())
}

/// Etat detaille d'un nœud : (ouverts, mappages, pages).
pub fn etat(node: usize) -> Option<(usize, usize, usize)> {
    position(node).map(|index| {
        let entree = &cache()[index];
        (entree.ouverts, entree.mappages, entree.pages.len())
    })
}
