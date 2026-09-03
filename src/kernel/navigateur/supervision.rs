// La supervision des processus du navigateur.
//
// CE QUI MANQUAIT
// ===============
//
// Le noyau savait qu'un client graphique existait -- `perf::browser_*` le
// suivait --, et il n'en connaissait qu'UN. Or Ladybird n'est pas un
// processus : c'est un courtier, un serveur de requetes, un decodeur d'images,
// et un moteur de rendu PAR CONTEXTE. Le passage a plusieurs onglets ne
// demande pas seulement de les lancer : il demande de SAVOIR lequel est mort,
// de ne pas emporter les autres avec lui, et de ne pas relancer indefiniment
// celui qui plante a chaque essai.
//
// Ce module est cette brique. Il ne lance rien -- c'est le role du courtier --
// et il ne decide rien a sa place. Il tient le REGISTRE : qui existe, dans
// quel role, pour quel contexte, depuis quand, combien de fois deja.
//
// L'ISOLATION DES PANNES EST UNE PROPRIETE DU REGISTRE
// ===================================================
//
// « Un moteur de rendu qui plante n'emporte pas le navigateur » ne se decrete
// pas : cela se verifie. La mort d'un `Rendu` ne touche que sa propre entree ;
// celle d'un `Courtier` marque en revanche tous ses enfants comme condamnes,
// puisqu'ils n'ont plus personne a qui parler. Les deux regles sont ici, et
// le test les exerce.
//
// LA BOUCLE DE PLANTAGE EST BORNEE
// ================================
//
// Relancer sans compter transforme un moteur de rendu qui plante sur une page
// en une machine qui ne fait plus que redemarrer. Le budget est par ENTREE et
// par FENETRE DE TEMPS : un plantage isole se relance, une serie rapprochee
// s'arrete et le dit.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::SpinLock;

include!("supervision_corps.rs");
