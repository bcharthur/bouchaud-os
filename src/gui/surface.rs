//! Surface partagee entre le gestionnaire de fenetres et un client ring 3.
//!
//! ## Ce que c'est
//!
//! Un `memfd` — un nœud RAMFS anonyme — dont les pages sont allouees d'avance et
//! dont on garde la liste des frames physiques. Le client la projette avec
//! `mmap(MAP_SHARED)` ; le compositeur, lui, la lit directement par l'offset de
//! memoire physique du noyau. Les deux voient les **memes frames** : il n'y a
//! aucune copie entre le client et le compositeur, seulement la copie finale de
//! la zone abimee vers le tampon du bureau.
//!
//! ## Pourquoi le noyau garde ses propres frames
//!
//! Le cache de pages (`kernel::partage`) evince les frames d'un nœud des que ni
//! descripteur ni mappage ne le designe. Le compositeur detient donc une
//! reference « descripteur ouvert » pour toute la vie de la surface : sans elle,
//! la mort du client libererait des frames que le compositeur serait encore en
//! train de lire, et l'affichage se remplirait de la memoire de quelqu'un
//! d'autre — au moment precis ou une fenetre se ferme.
//!
//! ## Pourquoi lire page par page
//!
//! Les frames viennent de l'allocateur : elles ne sont pas contigues. Une ligne
//! de 1100 pixels fait 4400 octets et traverse donc au moins une frontiere de
//! page. `copie_ligne` recolle les morceaux ; c'est le seul endroit du
//! compositeur qui a le droit de savoir cela.

use alloc::vec::Vec;

use crate::kernel::fd::{FdKind, FileDesc};
use crate::kernel::memory;
use crate::kernel::partage;
use crate::kernel::task::Process;
use crate::kernel::vmm::PAGE_SIZE;

/// Une surface XRGB8888 partagee.
pub struct Surface {
    /// Nœud RAMFS anonyme qui la porte (ce qu'un `memfd_create` aurait rendu).
    pub node: usize,
    pub largeur: usize,
    pub hauteur: usize,
    /// Octets par ligne. Egal a `largeur * 4` : aucun alignement supplementaire
    /// n'est impose, Qt s'accommode de tout pas et un pas simple evite une
    /// deuxieme verite sur la geometrie.
    pub pas: usize,
    /// Frames physiques, indexees par numero de page.
    frames: Vec<u64>,
}

impl Surface {
    /// Alloue une surface de `largeur` x `hauteur` pixels.
    ///
    /// Toutes les pages sont touchees ici, pas a la demande. Une surface qui
    /// n'entre pas dans la memoire disponible doit echouer **maintenant**, quand
    /// il reste la possibilite d'afficher un message, et pas au milieu de la
    /// premiere trame du navigateur.
    pub fn nouvelle(largeur: usize, hauteur: usize) -> Option<Surface> {
        if largeur == 0 || hauteur == 0 {
            return None;
        }
        let pas = largeur * 4;
        let octets = pas.checked_mul(hauteur)?;
        let pages = (octets + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;

        let node = crate::fs::ramfs::fs().cree_anonyme("bo-surface").ok()?;
        // La reference du compositeur, prise avant toute page : si l'allocation
        // echoue a mi-chemin, `Drop` rendra proprement ce qui a ete pris.
        partage::ouvre(node);

        let mut surface = Surface { node, largeur, hauteur, pas, frames: Vec::new() };
        surface.frames.reserve(pages);
        for numero in 0..pages {
            match partage::page(node, numero as u64) {
                Some(lease) => surface.frames.push(lease.frame()),
                None => {
                    crate::kernel::dmesg::log_fmt(format_args!(
                        "gui: surface {}x{} refusee, memoire physique insuffisante",
                        largeur, hauteur
                    ));
                    return None; // `Drop` relache la reference et evince.
                }
            }
        }
        // Les frames sortent de l'allocateur telles quelles : les mettre a zero
        // evite qu'un fond de fenetre affiche la memoire d'un programme mort en
        // attendant la premiere trame du client.
        surface.efface(0x0011_1B2E);
        Some(surface)
    }

    /// Taille en octets.
    pub fn octets(&self) -> usize {
        self.pas * self.hauteur
    }

    /// Un descripteur neuf sur cette surface, a installer dans un client.
    ///
    /// C'est l'equivalent d'un `SCM_RIGHTS` sans socket : le gestionnaire de
    /// fenetres vit dans le noyau, il peut donc poser le descripteur directement
    /// dans la table du processus qu'il vient de creer. Le jour ou un serveur
    /// graphique passera en ring 3, c'est cette fonction — et elle seule — qui
    /// deviendra un `sendmsg`.
    pub fn descripteur(&self, processus: &Process) -> i32 {
        processus.files.lock().insert(FileDesc::new(FdKind::File(self.node)))
    }

    /// Adresse noyau du premier octet d'une page, si elle existe.
    fn page(&self, numero: usize) -> Option<*mut u8> {
        self.frames.get(numero).map(|frame| memory::phys_to_virt(*frame))
    }

    /// Remplit toute la surface d'une couleur XRGB.
    ///
    /// Page par page, et non ligne par ligne : la couleur est la meme partout,
    /// le decoupage en lignes n'apporterait donc rien qu'un cout. Un pixel ne
    /// chevauche jamais deux pages — `PAGE_SIZE` est un multiple de 4 et le pas
    /// aussi —, ce qui autorise a traiter chaque frame comme 1024 pixels.
    pub fn efface(&self, couleur: u32) {
        let pixels_par_page = PAGE_SIZE as usize / 4;
        for numero in 0..self.frames.len() {
            if let Some(base) = self.page(numero) {
                let mots = base as *mut u32;
                for i in 0..pixels_par_page {
                    unsafe { core::ptr::write_volatile(mots.add(i), couleur) };
                }
            }
        }
    }

    /// Copie `compte` pixels de la ligne `y` a partir de `x` vers `dst`.
    ///
    /// C'est le chemin chaud du compositeur : appele une fois par ligne abimee.
    /// La copie se fait par tranches de page, avec `copy_nonoverlapping` — un
    /// `memcpy` par tranche plutot qu'une boucle par pixel.
    pub fn copie_ligne(&self, y: usize, x: usize, compte: usize, dst: &mut [u32]) {
        if y >= self.hauteur || x >= self.largeur {
            return;
        }
        let compte = compte.min(self.largeur - x).min(dst.len());
        let mut restant = compte * 4;
        let mut octet = y * self.pas + x * 4;
        let mut ecrit = 0usize; // octets deja copies
        let destination = dst.as_mut_ptr() as *mut u8;
        while restant > 0 {
            let numero = octet / PAGE_SIZE as usize;
            let dans_page = octet % PAGE_SIZE as usize;
            let tranche = core::cmp::min(restant, PAGE_SIZE as usize - dans_page);
            match self.page(numero) {
                Some(base) => unsafe {
                    core::ptr::copy_nonoverlapping(
                        base.add(dans_page),
                        destination.add(ecrit),
                        tranche,
                    );
                },
                // Hors des frames connues : on laisse la destination telle
                // quelle plutot que de lire une page qui ne nous appartient pas.
                None => return,
            }
            restant -= tranche;
            octet += tranche;
            ecrit += tranche;
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // Rend la reference « descripteur ouvert » prise a la creation. Les
        // frames ne partent que si plus aucun mappage client ne les projette :
        // c'est le cache de pages qui tranche, pas nous.
        partage::ferme(self.node);
    }
}
