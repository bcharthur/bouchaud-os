//! Ce que le noyau sait du navigateur, en tant que GROUPE de processus.
//!
//! Ladybird n'est pas un processus : c'est un courtier, un serveur de
//! requetes, un decodeur d'images, et un moteur de rendu par contexte de
//! rendu. Le noyau les suivait comme un seul client graphique -- il n'en
//! connaissait qu'un --, ce qui suffisait tant qu'il n'y en avait qu'un.

pub mod supervision;

// BOUCHAUD_C24_PROFIL_DE_DEMARRAGE
//
// Ou passe le temps entre le double-clic et la premiere trame. Module PUR --
// il ne mesure rien, il RANGE des instants que la supervision et le profil lui
// donnent --, donc `tools/navigateur/test_demarrage.rs` l'exerce sur l'hote,
// alors que le demarrage ne s'observe qu'au bout d'une construction complete
// de Ladybird.
pub mod demarrage;
