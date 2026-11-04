//! Dossiers personnels, regles PURES (BOUCHAUD_DOSSIERS_PERSONNELS_V1).
//!
//! Le dossier personnel d'un compte est la ou ses documents SURVIVENT : sous
//! `/persist/home/<compte>` quand la machine a une zone de persistance, sinon
//! l'accueil declare du compte (volatile, comme avant). Il porte trois
//! sous-dossiers standard, crees a l'ouverture de la session s'ils manquent --
//! jamais ecrases, jamais vides.

/// Sous-dossiers standard, dans l'ordre de la barre laterale de Fichiers.
pub const SOUS_DOSSIERS: [&str; 3] = ["Documents", "Images", "T\u{e9}l\u{e9}chargements"];

/// Droits du dossier personnel (prive) et de ses sous-dossiers.
pub const MODE_PERSONNEL: u16 = 0o700;
pub const MODE_SOUS_DOSSIER: u16 = 0o700;

/// Racine des dossiers personnels persistants.
pub const RACINE_PERSISTANTE: &str = "/persist/home";

/// Un nom de compte utilisable comme nom de dossier : non vide, sans `/`, ni
/// `.` ni `..`.
pub fn nom_acceptable(compte: &str) -> bool {
    !compte.is_empty() && compte != "." && compte != ".." && !compte.contains('/')
        && !compte.contains('\0')
}

/// Le dossier personnel de `compte`. `durable` : une zone de persistance
/// existe. `accueil` : l'accueil declare du compte (`/` pour root).
pub fn chemin(compte: &str, durable: bool, accueil: &str) -> alloc::string::String {
    use alloc::string::String;
    if durable && nom_acceptable(compte) {
        let mut s = String::from(RACINE_PERSISTANTE);
        s.push('/');
        s.push_str(compte);
        s
    } else {
        String::from(accueil)
    }
}
