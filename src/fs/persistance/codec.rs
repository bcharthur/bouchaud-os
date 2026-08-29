// --- Lecture et ecriture des champs -------------------------------------------

fn lit_u32(octets: &[u8]) -> u32 {
    u32::from_le_bytes([octets[0], octets[1], octets[2], octets[3]])
}

fn lit_u64(octets: &[u8]) -> u64 {
    let mut brut = [0u8; 8];
    brut.copy_from_slice(&octets[..8]);
    u64::from_le_bytes(brut)
}

fn ecrit_u32(cible: &mut [u8], valeur: u32) {
    cible[..4].copy_from_slice(&valeur.to_le_bytes());
}

fn ecrit_u64(cible: &mut [u8], valeur: u64) {
    cible[..8].copy_from_slice(&valeur.to_le_bytes());
}

fn chaine(octets: &[u8]) -> String {
    let fin = octets.iter().position(|&c| c == 0).unwrap_or(octets.len());
    String::from_utf8_lossy(&octets[..fin]).into_owned()
}
