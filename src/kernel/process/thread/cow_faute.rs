// BOUCHAUD_COW_V1 -- la casse d'une copie-a-l'ecriture pour le processus
// courant, AUCUN verrou tenu a l'entree.
//
// Trois appelants, une seule regle :
//   * la faute de page d'ECRITURE sur une PTE presente (ring 3) ;
//   * `fault_in_user_range` (compat Linux) et `usercopy::resident_range` (ABI
//     native), avant toute ecriture du noyau vers l'utilisateur ;
//   * la sortie d'un fil (`clear_child_tid`).
//
// L'ordre est celui que `CasseCow::Copiee` impose : PTE neuve, verrou `Mm`
// rendu, shootdown de la page sur les coeurs de cet espace, PUIS la part de
// l'ancienne frame rendue. L'autre proprietaire ne peut donc promouvoir
// l'ancienne frame (et y ecrire) qu'une fois qu'aucun coeur de cet espace ne
// peut plus la lire.

/// Issue de `casse_cow_courant`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IssueCow {
    /// La page est desormais inscriptible pour ce processus.
    Resolue,
    /// Ce n'est pas une page COW inscriptible : rien a faire (la faute reste
    /// une faute).
    PasCow,
    /// Plus de memoire, et l'OOM n'a rien pu rendre.
    SansMemoire,
}

pub fn casse_cow_courant(adresse: u64) -> IssueCow {
    let process = current_process();
    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    loop {
        let issue = process.mm.lock().space.prepare_casse_cow(page, true);
        match issue {
            crate::kernel::vmm::CasseCow::Promue => return IssueCow::Resolue,
            crate::kernel::vmm::CasseCow::Copiee { invalidation, ancienne } => {
                invalidation.execute();
                crate::kernel::vmm::rend_frame_possedee(ancienne);
                return IssueCow::Resolue;
            }
            crate::kernel::vmm::CasseCow::PasCow => {
                // Un frere a pu casser la meme page entre la faute et le
                // verrou : la faute est alors deja resolue.
                return if process.mm.lock().space.writable(page) {
                    IssueCow::Resolue
                } else {
                    IssueCow::PasCow
                };
            }
            crate::kernel::vmm::CasseCow::SansMemoire => {
                // BOUCHAUD_OOM_CONTROLE_V1 : la copie manque de frame comme
                // une faute anonyme -- meme politique, verrou deja rendu.
                if !sur_faute_sans_memoire(process.pid) {
                    return IssueCow::SansMemoire;
                }
            }
        }
    }
}
