//! La reconnaissance d'une navigation REFUSEE, pure.
//!
//! # Pourquoi ce module est a part, et pourquoi il est pur
//!
//! Le `RequestServer` renonce a une requete AVANT tout `connect` quand le
//! resolveur n'est pas configure. Aucun appel systeme reseau n'a lieu : le
//! noyau ne voit rien. La seule trace qui sorte de l'anneau 3 est la ligne que
//! notre propre correctif fait ecrire sur la sortie d'erreur du navigateur.
//!
//! Ce chemin voit donc TOUT ce que l'anneau 3 ecrit sur sa console. C'est la
//! raison d'etre de ce module : la reconnaissance doit etre assez etroite pour
//! qu'aucune autre ligne ne la declenche, et assez simple pour se lire. Un
//! marqueur, deux champs, aucune allocation -- et un banc a l'hote qui verifie
//! qu'une trace ordinaire du navigateur ne fabrique pas de fausse navigation.
//!
//! Aucun `use crate::`, aucun `unsafe`, aucune horloge.

/// Longueur retenue d'une URL. Au-dela, elle est tronquee : l'hote et le
/// debut du chemin suffisent a savoir de quelle page on parle, et refuser une
/// URL longue ferait disparaitre la navigation.
pub const URL_MAX: usize = 96;

/// Le marqueur, et lui seul.
const MARQUEUR: &[u8] = b"BOUCHAUD_NAV_BLOQUEE url=";
const CHAMP_RAISON: &[u8] = b"raison=";

/// Ce qu'une ligne de refus porte.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Refus<'a> {
    pub url: &'a str,
    pub raison: &'a str,
}

/// Cherche un refus dans un bloc ecrit sur la console.
///
/// Rend `None` des que quelque chose manque : mieux vaut manquer un refus que
/// d'en inventer un avec une URL a moitie lue. La console recoit des blocs,
/// pas des lignes, et une ecriture peut etre coupee en deux appels.
pub fn refus_dans(bloc: &[u8]) -> Option<Refus<'_>> {
    let debut = fenetre(bloc, MARQUEUR)?;
    let reste = &bloc[debut + MARQUEUR.len()..];
    let fin = borne(reste).min(URL_MAX);
    if fin == 0 {
        return None;
    }
    let url = core::str::from_utf8(&reste[..fin]).ok()?;
    let raison = match fenetre(reste, CHAMP_RAISON) {
        Some(place) => {
            let queue = &reste[place + CHAMP_RAISON.len()..];
            let bout = borne(queue);
            core::str::from_utf8(&queue[..bout]).unwrap_or("refus")
        }
        None => "refus",
    };
    let raison = if raison.is_empty() { "refus" } else { raison };
    Some(Refus { url, raison })
}

/// La fin d'un champ : le premier blanc, ou la fin du bloc.
fn borne(reste: &[u8]) -> usize {
    reste
        .iter()
        .position(|o| *o == b' ' || *o == b'\n' || *o == b'\r' || *o == b'\t')
        .unwrap_or(reste.len())
}

/// La position d'un motif dans un tampon, sans allocation.
fn fenetre(foin: &[u8], aiguille: &[u8]) -> Option<usize> {
    if aiguille.is_empty() || foin.len() < aiguille.len() {
        return None;
    }
    (0..=foin.len() - aiguille.len())
        .find(|debut| &foin[*debut..debut + aiguille.len()] == aiguille)
}
