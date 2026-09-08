// W^X : une page n'est jamais inscriptible ET executable en meme temps.
//
// # Ce qui manquait, et ce que cela ouvrait
//
// `prot_to_flags` traduisait `PROT_*` en drapeaux de table de pages, sans
// jamais examiner la COMBINAISON. Une demande `PROT_WRITE | PROT_EXEC`
// produisait donc une page portant `PTE_WRITE` et ne portant pas
// `PTE_NO_EXEC`.
//
// C'est exactement ce dont une injection de code a besoin, et rien de plus :
// obtenir une page ou l'on peut ECRIRE des octets puis les EXECUTER, sans
// jamais avoir a contourner quoi que ce soit. Un debordement qui atteint un
// `mmap` suffit ; il n'y a pas de seconde etape a franchir.
//
// Le meme trou existait au chargement d'un ELF : un segment declare `PF_W |
// PF_X` etait mappe tel quel. Aucune chaine de compilation moderne n'en emet ;
// un binaire qui en porte un a ete fabrique pour cela.
//
// # Ce que cette regle interdit, et ce qu'elle n'interdit pas
//
// Elle interdit qu'une page soit inscriptible et executable AU MEME MOMENT.
// Elle n'interdit pas la sequence : mapper en ecriture, ecrire, puis
// `mprotect` en execution seule. C'est ainsi que travaille un chargeur
// dynamique, et c'est ainsi que travaillerait un compilateur a la volee -- la
// difference etant qu'a aucun instant les deux droits ne coexistent.
//
// # Pourquoi refuser plutot que corriger en silence
//
// Retirer discretement le droit d'execution rendrait une page qui n'est pas
// celle qu'on a demandee, et l'appelant s'en apercevrait par une faute a
// l'execution, loin de la cause. Retirer le droit d'ecriture ferait echouer la
// premiere ecriture. Les deux mentent.
//
// Refuser avec `EACCES` dit ce qui s'est passe, a l'endroit ou cela se passe.
// C'est aussi ce que fait un systeme durci, et un programme portable sait deja
// le rattraper -- il demande alors l'ecriture puis l'execution en deux temps.

#![allow(dead_code)]

/// Drapeau « page inscriptible » d'une entree de table de pages.
pub const ECRITURE: u64 = 1 << 1;
/// Drapeau « page non executable ».
pub const NON_EXECUTABLE: u64 = 1 << 63;

/// Ces drapeaux donnent-ils une page a la fois inscriptible et executable ?
///
/// L'execution est le DEFAUT sur x86-64 : une page est executable des que le
/// bit 63 est absent. C'est pour cela que la regle porte sur l'absence de
/// `NON_EXECUTABLE` et non sur la presence d'un drapeau d'execution -- oublier
/// de poser le bit suffit a rendre la page executable, et c'est l'oubli qu'on
/// cherche.
#[inline]
pub const fn inscriptible_et_executable(drapeaux: u64) -> bool {
    drapeaux & ECRITURE != 0 && drapeaux & NON_EXECUTABLE == 0
}

/// Une demande `PROT_*` viole-t-elle W^X ?
#[inline]
pub const fn prot_viole_wx(prot: u32, prot_write: u32, prot_exec: u32) -> bool {
    prot & prot_write != 0 && prot & prot_exec != 0
}

/// Un segment ELF viole-t-il W^X ?
#[inline]
pub const fn segment_viole_wx(drapeaux_segment: u32, pf_w: u32, pf_x: u32) -> bool {
    drapeaux_segment & pf_w != 0 && drapeaux_segment & pf_x != 0
}
