//! CE QUE LE MONDE UTILISATEUR APPREND DU NOMBRE DE PROCESSEURS.
//!
//! # Le defaut, et ce qu'il coutait
//!
//! `sysroot.rs` publiait ceci, en dur :
//!
//! ```text
//!   /sys/devices/system/cpu/online     "0"
//!   /sys/devices/system/cpu/possible   "0"
//!   /sys/devices/system/cpu/present    "0"
//!   /proc/cpuinfo                      un seul bloc, siblings 1, cpu cores 1
//! ```
//!
//! « 0 » n'est pas un compte : c'est une PLAGE, et celle-ci ne contient que le
//! processeur zero. Tout ce qui demande « combien de processeurs » -- la glibc
//! par `sysconf(_SC_NPROCESSORS_ONLN)`, les pools de threads d'AK, ceux de
//! Skia, ceux de LibJS -- recevait donc la reponse UN.
//!
//! La machine de reference est un Ryzen 7 5700U : huit coeurs, seize fils. Le
//! noyau en ordonnance seize. Le releve physique disait « la charge WebContent
//! peut etre tres elevee sur un seul coeur alors que le CPU global parait
//! faible », et c'etait la description exacte de ce que ces quatre fichiers
//! provoquaient : chaque pool de threads du navigateur etait cree avec un seul
//! fil, parce qu'on lui avait dit qu'il n'y avait qu'un processeur.
//!
//! # Pourquoi ce calcul merite son propre fichier
//!
//! Parce que les plages Linux ont des cas limites, qu'ils sont tous muets, et
//! que chacun se traduit par une mauvaise taille de pool plutot que par une
//! erreur :
//!
//! ```text
//!   un processeur      -> "0"       et surtout pas "0-0", que le noyau
//!                                    Linux n'ecrit jamais
//!   seize processeurs  -> "0-15"    et non "0-16"
//!   zero processeur    -> n'existe pas ; ne jamais ecrire "0--1"
//! ```
//!
//! Aucun de ces trois ne fait echouer quoi que ce soit : ils rendent seulement
//! un navigateur monofil sur une machine a seize fils. C'est exactement le
//! genre de defaut qu'un banc d'essai hote attrape et qu'un test d'integration
//! laisse passer -- `tools/process/test_cpu_topologie.rs` les exerce.

/// La plage Linux qui decrit `count` processeurs, a partir de zero.
///
/// Ecrit dans `sortie` et rend le nombre d'octets ecrits, pour ne rien
/// allouer : ce texte est produit pendant l'installation du sysroot.
pub fn plage(count: usize, sortie: &mut [u8]) -> usize {
    if count == 0 || sortie.is_empty() {
        return 0;
    }
    let mut ecrits = ecris_nombre(0, sortie);
    if count == 1 {
        // Linux ecrit « 0 » et jamais « 0-0 ». Une plage degeneree est lue
        // correctement par la plupart des analyseurs, et par la plupart
        // seulement : mieux vaut ecrire ce que le noyau ecrit.
        return ecrits;
    }
    if ecrits < sortie.len() {
        sortie[ecrits] = b'-';
        ecrits += 1;
    }
    ecrits + ecris_nombre(count - 1, &mut sortie[ecrits..])
}

fn ecris_nombre(mut valeur: usize, sortie: &mut [u8]) -> usize {
    if sortie.is_empty() {
        return 0;
    }
    if valeur == 0 {
        sortie[0] = b'0';
        return 1;
    }
    let mut chiffres = [0u8; 20];
    let mut n = 0usize;
    while valeur > 0 && n < chiffres.len() {
        chiffres[n] = b'0' + (valeur % 10) as u8;
        valeur /= 10;
        n += 1;
    }
    let mut ecrits = 0usize;
    while n > 0 && ecrits < sortie.len() {
        n -= 1;
        sortie[ecrits] = chiffres[n];
        ecrits += 1;
    }
    ecrits
}

/// Ce que `/proc/cpuinfo` doit annoncer pour un processeur donne.
///
/// Les deux champs qui comptent sont `siblings` -- les fils logiques du meme
/// paquet -- et `cpu cores` -- les coeurs physiques. Une bibliotheque qui
/// distingue les deux, et Skia en est une, choisit son parallelisme d'apres
/// `cpu cores` et non d'apres le nombre de lignes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bloc {
    pub processeur: usize,
    pub siblings: usize,
    pub coeurs: usize,
    pub core_id: usize,
}

/// La description des `logiques` processeurs, en supposant `fils_par_coeur`.
///
/// Rend `None` au-dela du dernier processeur : la boucle d'ecriture s'arrete
/// dessus, et il n'y a donc qu'un seul endroit qui connaisse la borne.
pub fn bloc(logiques: usize, fils_par_coeur: usize, index: usize) -> Option<Bloc> {
    if logiques == 0 || index >= logiques {
        return None;
    }
    // `fils_par_coeur` a zero serait une division par zero, et un materiel qui
    // ne le renseigne pas est plus courant qu'on ne croit. Un fil par coeur
    // est le repli sur : il sous-estime le parallelisme au pire, il ne rend
    // jamais un compte absurde.
    let fils = if fils_par_coeur == 0 { 1 } else { fils_par_coeur };
    Some(Bloc {
        processeur: index,
        siblings: logiques,
        coeurs: core::cmp::max(1, logiques / fils),
        core_id: index / fils,
    })
}

/// Le nombre de processeurs a annoncer, borne et jamais nul.
///
/// Un noyau qui n'a pas encore fini d'enumerer ses processeurs rend zero. Le
/// publier ferait ecrire une plage vide, et une plage vide se lit « aucun
/// processeur » : les bibliotheques retombent alors sur leurs heuristiques,
/// qui valent souvent un. Annoncer UN est faux aussi, mais c'est le meme faux
/// qu'avant, et il est borne.
pub fn annonces(schedulables: usize, maximum: usize) -> usize {
    let borne = if maximum == 0 { 1 } else { maximum };
    core::cmp::max(1, core::cmp::min(schedulables, borne))
}

/// Combien de fils logiques par coeur retenir, mesure ou repli.
///
/// BOUCHAUD_C26_SMT_MESURE_ET_NON_SUPPOSE
///
/// La valeur etait une CONSTANTE : deux, parce que la TRIGKEY porte un Ryzen
/// 7 5700U. Juste sur cette machine, fausse partout ailleurs -- QEMU lance
/// `-smp 8` en huit paquets d'un seul fil, et `/proc/cpuinfo` annoncait alors
/// « cpu cores: 4 » sur une machine qui en a huit.
///
/// Trois regles, et chacune evite un nombre absurde :
///
///   * sans mesure, UN fil par coeur. C'est le repli prudent : il
///     sous-estime le partage au pire, il ne rend jamais plus de coeurs que
///     la machine n'a de processeurs ;
///   * jamais zero, qui serait une division par zero chez l'appelant ;
///   * jamais plus que le nombre de processeurs logiques -- « quatre fils par
///     coeur » sur une machine a deux processeurs decrirait un materiel qui
///     n'existe pas.
pub fn fils_retenus(mesure: Option<usize>, logiques: usize) -> usize {
    let logiques = if logiques == 0 { 1 } else { logiques };
    match mesure {
        Some(fils) if fils >= 1 => core::cmp::min(fils, logiques),
        _ => 1,
    }
}
