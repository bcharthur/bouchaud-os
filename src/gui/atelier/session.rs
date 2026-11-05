//! Politique pure de la touche Echap dans Atelier.
//! Echap ne quitte jamais la session : seules les actions explicites le font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchapSansMenu {
    PrisParApplication,
    FermerFenetre,
    Ignorer,
}

pub const fn action_echap(prise_par_application: bool, a_des_fenetres: bool) -> EchapSansMenu {
    if prise_par_application {
        EchapSansMenu::PrisParApplication
    } else if a_des_fenetres {
        EchapSansMenu::FermerFenetre
    } else {
        EchapSansMenu::Ignorer
    }
}
