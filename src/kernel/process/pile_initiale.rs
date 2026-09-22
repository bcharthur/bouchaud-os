//! LA PLACE QUE LA PILE INITIALE D'UN PROCESSUS DEMANDE VRAIMENT.
//!
//! # Pourquoi ce calcul existe
//!
//! `elf::build_stack` allouait, projetait et mettait a zero les huit
//! mebioctets de la pile utilisateur a chaque `exec`. C'etait cinquante-trois
//! millisecondes sur les cinquante-quatre que coutait l'exec d'un binaire de
//! treize kibioctets. La pile est maintenant une promesse `Zero` peuplee a la
//! demande, et seule la FENETRE que le constructeur ecrit est projetee
//! d'avance.
//!
//! Elle doit l'etre parce que `AddressSpace::write()` passe par la vue noyau
//! des tables : une page absente n'y provoque aucune faute, elle perd
//! l'ecriture. Une fenetre trop petite ne planterait donc pas -- elle
//! tronquerait silencieusement l'environnement d'un processus.
//!
//! # Pourquoi ce fichier est separe, et pur
//!
//! Parce que le defaut qu'il evite est MUET. Une chaine d'environnement
//! perdue ne leve aucune erreur : elle donne un navigateur qui ne trouve plus
//! ses polices, ou une libc dont le canari de pile vaut zero. Aucun test
//! d'integration ne voit cela, et il n'apparait qu'avec un environnement plus
//! gros que celui du jour ou le code a ete ecrit.
//!
//! Le calcul se verifie donc sur l'hote (`tools/process/test_pile_initiale.rs`),
//! avec les cas qui comptent : un `argv` enorme, un `envp` enorme, une liste
//! auxv qui grandit, et le depassement de la pile entiere.
//!
//! # La regle de conception
//!
//! Chaque terme MAJORE. Un octet de trop coute une page projetee de plus ; un
//! octet de moins coute une ecriture perdue que rien ne signale. L'asymetrie
//! est totale, et le calcul penche donc systematiquement du cote sur.

/// La taille d'une page, en octets.
pub const PAGE: u64 = 4096;

/// Combien d'octets un `push_bytes` peut consommer au-dela de la longueur.
///
/// `push_bytes` recule du nombre d'octets, PUIS aligne vers le bas sur huit.
/// L'alignement peut donc couter jusqu'a sept octets de plus, et le terminateur
/// nul un de plus.
const SURCOUT_PAR_CHAINE: u64 = 1 + 7;

/// Le nombre d'entrees auxv que le constructeur est autorise a ecrire.
///
/// Le constructeur en ecrit vingt aujourd'hui. La borne est volontairement
/// plus haute, et elle est VERIFIEE a l'execution : si la liste depasse cette
/// valeur, `build_stack` refuse au lieu de deborder de la fenetre. Une borne
/// non verifiee serait exactement le « 64 paires » devine que cette revision
/// remplace.
pub const AUXV_MAX: u64 = 48;

/// La place demandee par les chaines et les tableaux, avant arrondi.
///
/// `argv` et `envp` sont donnes par leurs LONGUEURS : ce module ne connait pas
/// les chaines elles-memes, et n'a pas besoin de les connaitre.
pub fn octets_bruts(argv: &[usize], envp: &[usize], auxv_entrees: u64) -> u64 {
    let mut octets: u64 = 0;

    // `saturating_add` A L'INTERIEUR AUSSI, et pas seulement a l'exterieur.
    //
    // Les longueurs viennent de l'espace utilisateur par `execve`. Un
    // `*longueur as u64 + SURCOUT` ordinaire deborde sur `usize::MAX` et rend
    // une valeur MINUSCULE : la fenetre calculee serait alors petite, le refus
    // ne se declencherait pas, et l'environnement serait tronque. C'est le
    // defaut exact que `tools/process/test_pile_initiale.rs` a trouve dans la
    // premiere version de ce fichier.
    for longueur in argv.iter() {
        octets = octets.saturating_add((*longueur as u64).saturating_add(SURCOUT_PAR_CHAINE));
    }
    for longueur in envp.iter() {
        octets = octets.saturating_add((*longueur as u64).saturating_add(SURCOUT_PAR_CHAINE));
    }

    // AT_RANDOM (seize octets), la chaine "x86_64\0" (sept), et la copie du
    // nom de l'executable -- qui est `argv[0]`, donc deja compte une fois
    // ci-dessus, mais ecrit DEUX fois.
    octets = octets.saturating_add(16 + 7);
    octets = octets.saturating_add(7 + 7);
    if let Some(premier) = argv.first() {
        octets = octets.saturating_add((*premier as u64).saturating_add(SURCOUT_PAR_CHAINE));
    }

    // Les tableaux de pointeurs : argc, argv + NULL, envp + NULL, puis les
    // paires auxv.
    octets = octets.saturating_add(8);
    octets = octets.saturating_add((argv.len() as u64 + 1).saturating_mul(8));
    octets = octets.saturating_add((envp.len() as u64 + 1).saturating_mul(8));
    octets = octets.saturating_add(auxv_entrees.saturating_mul(16));

    // L'alignement final du RSP sur seize octets, et le decalage de seize
    // octets que le constructeur laisse sous le sommet avant de commencer.
    octets = octets.saturating_add(16 + 16);

    octets
}

/// La fenetre a projeter, en octets, arrondie a la page.
///
/// Une page de plus que l'arrondi : l'adresse la plus basse ecrite peut tomber
/// juste sous une frontiere de page, et la fenetre doit la contenir.
pub fn fenetre(argv: &[usize], envp: &[usize], auxv_entrees: u64) -> u64 {
    let bruts = octets_bruts(argv, envp, auxv_entrees);
    let pages = (bruts / PAGE).saturating_add(2);
    pages.saturating_mul(PAGE)
}

/// La fenetre tient-elle dans la pile ?
///
/// Rend `false` quand l'environnement est si gros qu'il ne rentre pas. Le
/// refuser vaut mieux que de projeter une fenetre qui deborderait sous la
/// pile, dans la memoire d'autre chose.
pub fn tient_dans(fenetre_octets: u64, pile_octets: u64) -> bool {
    fenetre_octets < pile_octets
}

/// La plus basse adresse ecrite est-elle restee dans la fenetre ?
///
/// C'est la POSTCONDITION de `build_stack`. Elle existe parce que le calcul
/// ci-dessus est une majoration faite a la main : le jour ou le constructeur
/// ecrira quelque chose que ce module ne connait pas, cette verification-ci
/// le dira -- au lieu de laisser passer une ecriture perdue.
pub fn ecriture_dans_la_fenetre(plus_basse_ecrite: u64, sommet: u64, fenetre_octets: u64) -> bool {
    plus_basse_ecrite >= sommet.saturating_sub(fenetre_octets)
}
