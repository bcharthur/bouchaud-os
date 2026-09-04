//! Choix du donneur pour le vol de travail -- la POLITIQUE, separee du noyau.
//!
//! # Pourquoi ce module existe separement
//!
//! La regle qui decide s'il faut voler tenait en une ligne au milieu de
//! `try_steal`, entouree d'acces per-CPU, d'identites generationnelles et de
//! revendications atomiques. Elle etait donc inverifiable autrement qu'en
//! demarrant le systeme -- et c'est exactement ce qui a permis au defaut
//! ci-dessous de vivre sans etre vu.
//!
//! Ici elle est une fonction pure : la pression est fournie par l'appelant.
//! Un test hote peut donc la contredire en une milliseconde.
//!
//! # Le defaut que ce module encode, et sa mesure
//!
//! `pression_volable()` rend `bandes[1].longueur()` : le nombre de taches
//! NORMALES EN ATTENTE. La tache que le coeur execute n'y figure pas -- elle a
//! quitte la file au moment d'etre elue.
//!
//! Le filtre s'ecrivait `pression > 1`, justifie par « garder une tache au
//! donneur ». Ce raisonnement serait juste si la longueur comptait la tache
//! courante. Elle ne la compte pas. `> 1` exigeait donc DEUX taches en attente
//! EN PLUS de celle qui tourne : un coeur avec une tache en cours et une en
//! attente n'etait jamais deleste, meme avec trois coeurs au repos.
//!
//! La campagne SMP4 le montre sans ambiguite -- `steal=0/0`, donc pas une
//! seule TENTATIVE, et `rej_bal` cumule a 2852 sur les quatre coeurs : chaque
//! occasion rejetee au filtre, avant meme d'essayer.
//!
//! Le seuil correct est UNE tache en attente. Le donneur garde alors ce qu'il
//! execute -- rien ne lui est retire --, et la tache qui patientait derriere
//! part sur un coeur libre. C'est precisement la ou nait le parallelisme.

/// Nombre minimal de taches EN ATTENTE pour qu'un vol cree du parallelisme.
///
/// Un donneur a `PRESSION_MINIMALE` en attente execute deja autre chose : lui
/// prendre celle qui patiente ne le ralentit pas, cela occupe un coeur de plus.
pub const PRESSION_MINIMALE: usize = 1;

/// Choisit le coeur a delester, ou aucun.
///
/// `pression(c)` doit rendre le travail VOLABLE de `c` -- le fond de file, pas
/// l'interactif : une tache interactive volee paie une migration au moment
/// precis ou elle doit repondre.
///
/// Le donneur retenu est le plus charge, pour que le vol suivant ait encore un
/// candidat plutot que d'egaliser deux coeurs et de relancer un scan.
pub fn choisit_donneur(
    voleur: usize,
    en_ligne: usize,
    pression: impl Fn(usize) -> usize,
) -> Option<usize> {
    (0..en_ligne)
        .filter(|&candidat| candidat != voleur)
        .map(|candidat| (candidat, pression(candidat)))
        .filter(|&(_, charge)| charge >= PRESSION_MINIMALE)
        .max_by_key(|&(_, charge)| charge)
        .map(|(candidat, _)| candidat)
}
