/// Une sortie spontanée à zéro déclenche encore la récupération. Une
/// demande de fermeture n'efface jamais un statut de faute ou de signal.
pub const fn normale(code: i32, fermeture_volontaire: bool) -> bool {
    code == 0 && fermeture_volontaire
}
