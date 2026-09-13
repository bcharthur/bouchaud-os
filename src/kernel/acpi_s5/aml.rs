//! Lecture de l'objet AML `\_S5_` : la seule partie de l'extinction ACPI qui
//! se prouve sans machine.
//!
//! Elle est isolee ici parce qu'elle ne touche ni port, ni memoire physique,
//! ni table du noyau : elle prend des octets et rend deux entiers. C'est donc
//! la seule moitie d'`acpi_s5` qu'une suite hote peut mettre a l'epreuve, et
//! c'est aussi celle ou une faute serait invisible -- une valeur `SLP_TYP`
//! fausse ne fait pas planter la machine, elle l'endort au lieu de l'eteindre.

/// Extrait `SLP_TYPa` et `SLP_TYPb` du paquet `\_S5_` du DSDT.
///
/// # Forme reconnue
///
/// ```text
/// '_' 'S' '5' '_'   NameOp deja consomme
/// 0x12              PackageOp
/// <PkgLength>       1 a 4 octets, dont les deux bits de poids fort du premier
///                   donnent le nombre d'octets supplementaires
/// <NumElements>     1 octet
/// <element 0>       ZeroOp / OneOp / BytePrefix+valeur
/// <element 1>       idem
/// ```
///
/// Tout ce qui sort de cette forme fait rendre `None` : mieux vaut ne pas
/// eteindre que d'ecrire une valeur `SLP_TYP` inventee, qui mettrait la machine
/// dans un etat de veille dont elle ne sait pas revenir.
pub fn extrait_s5(dsdt: &[u8]) -> Option<(u8, u8)> {
    let mut index = 0usize;
    while index + 4 <= dsdt.len() {
        if &dsdt[index..index + 4] == b"_S5_" {
            if let Some(valeurs) = lit_paquet_s5(&dsdt[index + 4..]) {
                return Some(valeurs);
            }
        }
        index += 1;
    }
    None
}

pub fn lit_paquet_s5(reste: &[u8]) -> Option<(u8, u8)> {
    // Certaines tables intercalent un octet de remplissage avant `PackageOp`.
    let mut position = 0usize;
    while position < reste.len().min(2) && reste[position] != 0x12 {
        position += 1;
    }
    if position >= reste.len() || reste[position] != 0x12 {
        return None;
    }
    position += 1;
    if position >= reste.len() {
        return None;
    }
    let octets_supplementaires = (reste[position] >> 6) as usize;
    position += 1 + octets_supplementaires;
    if position >= reste.len() {
        return None;
    }
    let elements = reste[position];
    position += 1;
    if elements < 2 {
        return None;
    }
    let a = lit_entier_aml(reste, &mut position)?;
    let b = lit_entier_aml(reste, &mut position)?;
    Some((a & 0x07, b & 0x07))
}

fn lit_entier_aml(octets: &[u8], position: &mut usize) -> Option<u8> {
    let op = *octets.get(*position)?;
    *position += 1;
    match op {
        0x00 => Some(0),        // ZeroOp
        0x01 => Some(1),        // OneOp
        0x0a => {               // BytePrefix
            let valeur = *octets.get(*position)?;
            *position += 1;
            Some(valeur)
        }
        0x0b => {               // WordPrefix : seul l'octet bas nous interesse
            let valeur = *octets.get(*position)?;
            *position += 2;
            Some(valeur)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{extrait_s5, lit_paquet_s5};

    /// La forme la plus repandue : `Name(\_S5_, Package(2){ 0x05, 0x05 })`.
    #[test]
    fn paquet_a_prefixe_octet() {
        let table = [
            b'_', b'S', b'5', b'_', 0x12, 0x06, 0x02, 0x0a, 0x05, 0x0a, 0x05,
        ];
        assert_eq!(extrait_s5(&table), Some((5, 5)));
    }

    /// QEMU emet `Package(2){ Zero, Zero }` : les operandes valent alors 0.
    #[test]
    fn paquet_a_zero_op() {
        let table = [b'_', b'S', b'5', b'_', 0x12, 0x04, 0x02, 0x00, 0x00];
        assert_eq!(extrait_s5(&table), Some((0, 0)));
    }

    /// Une longueur de paquet sur deux octets decale les elements.
    #[test]
    fn longueur_de_paquet_sur_deux_octets() {
        let table = [
            b'_', b'S', b'5', b'_', 0x12, 0x41, 0x0a, 0x02, 0x0a, 0x07, 0x0a, 0x07,
        ];
        assert_eq!(extrait_s5(&table), Some((7, 7)));
    }

    /// Un paquet d'un seul element ne dit pas quoi ecrire dans PM1b.
    #[test]
    fn paquet_trop_court_refuse() {
        let table = [b'_', b'S', b'5', b'_', 0x12, 0x03, 0x01, 0x00];
        assert_eq!(extrait_s5(&table), None);
    }

    /// Une valeur qui n'est pas un entier AML connu n'est pas devinee.
    #[test]
    fn operande_inconnue_refusee() {
        let table = [b'_', b'S', b'5', b'_', 0x12, 0x05, 0x02, 0x5b, 0x00, 0x00];
        assert_eq!(extrait_s5(&table), None);
    }

    /// `_S5_` absent : aucune extinction ne doit etre tentee.
    #[test]
    fn sans_s5_rien() {
        let table = [b'_', b'S', b'3', b'_', 0x12, 0x06, 0x02, 0x0a, 0x05, 0x0a, 0x05];
        assert_eq!(extrait_s5(&table), None);
    }

    /// `SLP_TYP` tient sur trois bits : une valeur plus large est tronquee,
    /// pas propagee dans les bits voisins du registre PM1.
    #[test]
    fn slp_typ_borne_a_trois_bits() {
        let reste = [0x12, 0x06, 0x02, 0x0a, 0xff, 0x0a, 0xff];
        assert_eq!(lit_paquet_s5(&reste), Some((7, 7)));
    }
}
