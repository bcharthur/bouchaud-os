/// L'acces demande est-il couvert par les droits detenus ?
///
/// C'est la regle de base, et le seul endroit ou `AccessDenied` doit naitre
/// d'un manque de droit -- a distinguer de `BadHandle`, qui dit que le handle
/// n'existe pas. Confondre les deux renseigne un appelant hostile sur ce qui
/// existe dans la table d'autrui.
#[inline]
pub const fn verifie_acces(detenus: Rights, requis: Rights) -> Result<()> {
    if detenus.contains(requis) {
        Ok(())
    } else {
        Err(Error::AccessDenied)
    }
}

/// L'objet est-il du genre attendu ?
///
/// Un handle valide vers un `Event` presente a un appel qui attend un `Channel`
/// est une erreur de TYPE, pas de droit : `WrongType` le dit, et ne laisse pas
/// croire qu'un droit manquant suffirait.
#[inline]
pub const fn verifie_genre(reel: ObjectKind, attendu: ObjectKind) -> Result<()> {
    if reel as u16 == attendu as u16 {
        Ok(())
    } else {
        Err(Error::WrongType)
    }
}

/// La generation portee par le handle designe-t-elle encore cette incarnation ?
///
/// Un emplacement se recycle. Sans la generation, un handle conserve apres
/// fermeture designerait l'objet SUIVANT installe la -- qui peut appartenir a
/// un autre sous-systeme, avec d'autres droits. C'est le probleme ABA, et il se
/// referme ici.
#[inline]
pub const fn verifie_generation(emplacement: u32, handle: u32) -> Result<()> {
    if emplacement != 0 && emplacement == handle {
        Ok(())
    } else {
        Err(Error::BadHandle)
    }
}

/// Les droits qu'une DUPLICATION peut porter.
///
/// Deux conditions, et les deux comptent :
///
///   * la source doit porter `DUP`. Un handle donne sans ce droit ne peut pas
///     etre multiplie, ce qui borne la diffusion d'une capacite ;
///   * les droits demandes doivent etre un SOUS-ENSEMBLE de ceux de la source.
///     Demander plus n'est pas silencieusement rogne : c'est refuse. Rogner
///     laisserait un appelant croire qu'il detient ce qu'il a demande.
#[inline]
pub const fn verifie_duplication(source: Rights, demandes: Rights) -> Result<Rights> {
    if !source.contains(Rights::DUP) {
        return Err(Error::AccessDenied);
    }
    if !demandes.subset_of(source) {
        return Err(Error::AccessDenied);
    }
    Ok(demandes)
}

/// Les droits qu'un TRANSFERT par IPC peut porter.
///
/// La source doit porter `TRANSFER` -- sans quoi la capacite ne franchit pas la
/// frontiere du processus --, et le resultat est l'INTERSECTION des droits de
/// la source et du masque demande par l'emetteur.
///
/// Intersection et non verification : contrairement a la duplication, un
/// emetteur qui demande large veut dire « tout ce que je peux donner ». Le
/// masque est un plafond d'attenuation, pas une commande. `Rights::TOUS` --
/// donc « ne rien attenuer » -- reproduit exactement l'ancien comportement, ce
/// qui rend la migration des appelants sans risque.
///
/// Le droit `TRANSFER` lui-meme suit le masque : un courtier peut donc donner
/// une capacite que le receveur ne pourra PAS repasser plus loin.
#[inline]
pub const fn verifie_transfert(source: Rights, masque: Rights) -> Result<Rights> {
    if !source.contains(Rights::TRANSFER) {
        return Err(Error::AccessDenied);
    }
    Ok(source.intersection(masque))
}

/// Un handle derive est-il bien borne par sa source ?
///
/// Utilise par les tests et les assertions : c'est l'invariant que toutes les
/// regles ci-dessus doivent preserver, quelle que soit la profondeur de la
/// chaine de delegation.
#[inline]
pub const fn borne_par(derive: Rights, source: Rights) -> bool {
    derive.subset_of(source)
}
