//! Le contrat du compositeur ring 3 : `composited`.
//!
//! # Ce qui existe, et ce qui manque
//!
//! Le compositeur de Bouchaud est un FIL NOYAU (`gui::window_manager`). Il
//! fonctionne, il tient soixante trames par seconde, et il a un defaut qui ne
//! se corrige pas par reglage : une faute dans la composition est une faute
//! NOYAU. Un client qui remplit une file, un rectangle de degat mal calcule,
//! une police qui deborde -- chacun de ces cas met en jeu la machine entiere,
//! pas une fenetre.
//!
//! `composited` est le meme travail, en ring 3, sur l'ABI native. Ce module
//! porte son CONTRAT : le format du fil, le registre des surfaces, la propriete
//! des tampons, l'accumulation des degats, les mesures de trame. Il ne connait
//! ni le noyau, ni le materiel, ni les appels systeme -- c'est ce qui permet de
//! le mettre a l'epreuve sur l'hote, et c'est aussi ce qui permet au service C
//! freestanding de l'implementer sans partager de code.
//!
//! # Le tranchant vertical
//!
//! Le chemin que ce contrat doit rendre possible, de bout en bout :
//!
//!   1. un client ring 3 demande une surface ;
//!   2. `composited` la lui donne -- region partagee, deux tampons ;
//!   3. le client dessine dans le tampon ARRIERE ;
//!   4. il annonce `TrameLivree { tampon, degat }` ;
//!   5. `composited` echange les tampons et compose le degat ;
//!   6. le backend d'affichage presente ;
//!   7. `composited` rend au client le tampon qui n'est plus affiche.
//!
//! Le pas 7 est celui qu'on oublie, et c'est celui qui corrompt l'affichage :
//! un client qui reecrit dans un tampon encore lu par le compositeur produit
//! une dechirure. La PROPRIETE d'un tampon est donc explicite ici, et le
//! contrat refuse une trame livree sur un tampon que le client ne possede pas.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne remplace pas `window_manager`. Le compositeur noyau reste le chemin
//! par defaut tant que celui-ci n'est pas demontre : une migration graphique
//! qui echoue laisse une machine sans ecran, donc sans moyen de diagnostiquer.

use alloc::vec::Vec;

include!("composited_corps.rs");
