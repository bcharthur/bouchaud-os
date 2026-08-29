//! Decoupe d'un flux d'octets en lots pour le FIFO d'emission du 16550.
//!
//! # Pourquoi ce module existe separement
//!
//! Le pilote serie ecrivait un octet a la fois, en attendant THRE avant chacun.
//! Chaque attente est un `inb`, c'est-a-dire une sortie du mode traduit sous
//! TCG ; et `write(2)` s'execute sous le gros verrou du noyau, donc un
//! programme bavard y serialisait les quatre coeurs derriere COM1.
//!
//! Le 16550 a un FIFO d'emission de seize octets. Attendre une fois puis en
//! pousser seize divise par seize le nombre d'attentes, a debit de ligne
//! rigoureusement identique.
//!
//! Ce decoupage doit produire EXACTEMENT la meme suite d'octets qu'avant :
//! memes octets, meme ordre, meme conversion des sauts de ligne en CRLF. C'est
//! une propriete verifiable sans materiel, donc elle vit ici, dans un module
//! pur qu'un test d'hote inclut tel quel -- et non dans la fonction qui parle
//! aux ports.

/// Convertit `octets` (LF) en flux CRLF et l'emet par lots d'au plus
/// `tampon.len()` octets.
///
/// `emet` recoit des tranches non vides, dans l'ordre. Le contenu de `tampon`
/// apres l'appel n'a aucun sens : c'est un espace de travail, pas un resultat.
///
/// # Pourquoi un tampon fourni par l'appelant
///
/// Ce code tourne aussi sur le chemin de panique, ou l'on ne veut ni allocation
/// ni `static mut` partage entre CPU. Un tableau sur la pile de l'appelant est
/// la seule ressource dont on soit sur.
pub fn en_lots<F: FnMut(&[u8])>(octets: &[u8], tampon: &mut [u8], mut emet: F) {
    if tampon.len() < 2 {
        // Il faut pouvoir loger CR et LF ensemble : sinon un saut de ligne
        // devrait etre coupe en deux lots, ce que rien n'interdit mais qui
        // compliquerait la preuve pour aucun gain.
        return;
    }
    let mut rempli = 0usize;
    for &octet in octets {
        // Un CRLF ne doit jamais etre separe : on vide avant, pas au milieu.
        let besoin = if octet == b'\n' { 2 } else { 1 };
        if rempli + besoin > tampon.len() {
            emet(&tampon[..rempli]);
            rempli = 0;
        }
        if octet == b'\n' {
            tampon[rempli] = b'\r';
            rempli += 1;
        }
        tampon[rempli] = octet;
        rempli += 1;
    }
    if rempli != 0 {
        emet(&tampon[..rempli]);
    }
}

/// Le flux CRLF attendu pour `octets`, tel quel.
///
/// C'est la DEFINITION dont `en_lots` est la version par lots. Le test d'hote
/// compare la concatenation des lots a cette suite.
pub fn attendu(octets: &[u8], sortie: &mut alloc::vec::Vec<u8>) {
    for &octet in octets {
        if octet == b'\n' {
            sortie.push(b'\r');
        }
        sortie.push(octet);
    }
}
