// BOUCHAUD_PAT_ECRITURE_COMBINEE_V1
//
// LE FRAMEBUFFER N'EST PAS DE LA MEMOIRE, ET IL ETAIT TRAITE COMME UN REGISTRE
// =============================================================================
//
// # Ce que le TRIGKEY a mesure
//
// L'enregistreur de vol chronometre chaque presentation. Sur la machine de
// reference :
//
//     zone 50x25 (curseur)      0,03 ms      ~167 Mo/s
//     zone 1920x1080 (plein)   75,00 ms      ~110 Mo/s
//
// Le cout est STRICTEMENT proportionnel aux octets ecrits, quelle que soit la
// taille du rectangle. Ce n'est donc pas un surcout par trame : c'est le debit
// de la destination. Cent dix megaoctets par seconde, quand un `memcpy` vers
// de la memoire normale en fait dix a vingt MILLE sur ce meme processeur.
//
// Cent fois trop lent, et une seule cause possible : chaque ecriture part en
// transaction isolee sur le bus, sans jamais etre regroupee.
//
// # Pourquoi, et pourquoi cela ne se voyait pas
//
// Le framebuffer est une fenetre PCIe. Le micrologiciel decrit cette plage
// comme NON CACHABLE dans ses MTRR, et il a raison : pour des registres de
// peripherique, chaque ecriture doit partir seule et dans l'ordre.
//
// Un framebuffer n'est pas un banc de registres. L'ordre des pixels n'a aucune
// importance et personne ne lit un pixel apres l'avoir ecrit : les ecritures
// PEUVENT etre regroupees. C'est exactement ce que le type ECRITURE COMBINEE
// existe pour dire.
//
// Tant que le bureau ne redessine qu'un curseur, la difference ne se voit pas :
// cinq kilooctets a 167 Mo/s, c'est trente microsecondes. Elle devient brutale
// des qu'une fenetre est agrandie -- chaque trame devient plein ecran, et
// passe de trente microsecondes a soixante-quinze millisecondes.
//
// C'est le defaut que l'utilisateur a decrit ainsi : « fluide, puis apres avoir
// agrandi le terminal, la souris se teleporte ».
//
// # Ce que ce module fait
//
// Le type effectif d'une page est la COMBINAISON du type MTRR et du type PAT.
// La table est dans le manuel Intel (Vol. 3, « Effective Page-Level Memory
// Types ») et elle dit ceci : MTRR non cachable + PAT ecriture combinee donne
// ECRITURE COMBINEE. C'est la seule facon documentee de rendre une fenetre
// MMIO combinable sans toucher aux MTRR du micrologiciel.
//
// On reprogramme donc UNE entree de la table PAT -- la cinquieme, d'indice
// quatre -- et on laisse les quatre premieres strictement intactes. C'est
// important : toutes les pages existantes du systeme designent les entrees
// zero a trois, et elles doivent continuer a vouloir dire ce qu'elles
// voulaient dire. Aucune page ne vise l'entree quatre tant qu'on ne l'a pas
// demande explicitement.
//
// # La sequence, et pourquoi elle est aussi lourde
//
// Changer la PAT pendant que des lignes de cache portent des donnees ecrites
// sous l'ancien type laisserait ces lignes dans un etat que le processeur ne
// sait plus decrire. Le manuel donne donc une sequence, et elle n'est pas
// negociable : couper les interruptions, desactiver le cache, le vider, vider
// le TLB, ecrire la PAT, revider, reactiver. Elle s'execute une fois par
// coeur, au demarrage, et jamais ensuite.

use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Numero du MSR `IA32_PAT`.
const IA32_PAT: u32 = 0x277;

/// Valeur par defaut de la PAT, telle que le processeur demarre :
/// WB, WT, UC-, UC, WB, WT, UC-, UC.
const PAT_DEFAUT: u64 = 0x0007_0406_0007_0406;

/// Type « ecriture combinee » dans une entree de PAT.
const TYPE_ECRITURE_COMBINEE: u64 = 0x01;

/// L'INDICE que les pages devront designer pour obtenir l'ecriture combinee.
///
/// Quatre, et pas moins : les indices zero a trois sont ceux que designent
/// toutes les pages existantes -- une page qui ne porte ni `PAT`, ni `PCD`, ni
/// `PWT` designe l'indice zero. Les toucher changerait le sens de tout le
/// systeme d'un coup.
pub const INDICE_ECRITURE_COMBINEE: u64 = 4;

/// Valeur de la PAT une fois l'entree quatre passee en ecriture combinee.
const PAT_AVEC_ECRITURE_COMBINEE: u64 =
    (PAT_DEFAUT & !(0xFFu64 << 32)) | (TYPE_ECRITURE_COMBINEE << 32);

/// Coeurs ayant effectivement reprogramme leur PAT.
static COEURS_CONFIGURES: AtomicUsize = AtomicUsize::new(0);

/// Combien de coeurs ont reprogramme leur PAT.
///
/// La PAT est PAR COEUR. Un coeur qui ne l'a pas reprogrammee lit les memes
/// pages avec l'ancien type : il ne fautera pas, il sera simplement lent, et
/// silencieusement. Le compte se publie donc, et le garde-fou le compare au
/// nombre de coeurs en ligne.
pub fn coeurs_configures() -> usize {
    COEURS_CONFIGURES.load(Ordering::Relaxed)
}

#[inline]
fn lit_msr(msr: u32) -> u64 {
    let (haut, bas): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") bas, out("edx") haut,
             options(nomem, nostack, preserves_flags));
    }
    ((haut as u64) << 32) | bas as u64
}

#[inline]
fn ecrit_msr(msr: u32, valeur: u64) {
    unsafe {
        asm!("wrmsr",
             in("ecx") msr,
             in("eax") valeur as u32,
             in("edx") (valeur >> 32) as u32,
             options(nomem, nostack, preserves_flags));
    }
}

/// Le processeur expose-t-il la table PAT ?
///
/// `CPUID.1:EDX[16]`. Presente sur tout x86-64, mais on ne l'ecrit pas sans
/// l'avoir demande : ecrire un MSR absent leve une faute de protection
/// generale, et un noyau qui faute au demarrage ne dit rien a personne.
fn pat_disponible() -> bool {
    // `__cpuid` ET PAS DE L'ASSEMBLEUR A LA MAIN.
    //
    // La premiere version de cette fonction ecrivait `cpuid` elle-meme, avec
    // un `push rbx` / `pop rbx` autour -- et `options(nostack)`, qui affirme
    // le contraire. Pire : le registre de sortie pouvait etre alloue a `rbx`,
    // celui-la meme qu'on restaurait juste apres.
    //
    // Elle rendait donc faux, et le seul effet visible etait
    // `coeurs_pat=0` dans le journal. Le correctif tout entier devenait
    // inerte : sans PAT reprogrammee, le bit pose sur les pages designe
    // l'entree quatre restee en « write-back », et un intervalle MTRR non
    // cachable reste non cachable. Tout le travail, et aucun changement.
    //
    // L'intrinseque sait faire cela correctement, et le depot s'en sert deja
    // ailleurs.
    let feuille = unsafe { core::arch::x86_64::__cpuid(1) };
    feuille.edx & (1 << 16) != 0
}

/// Reprogramme la PAT du coeur courant.
///
/// A appeler UNE fois par coeur, au demarrage. Rend `false` si le processeur
/// n'expose pas de PAT -- auquel cas le framebuffer restera non cachable, et
/// le dira.
pub fn configure_ce_coeur() -> bool {
    if !pat_disponible() {
        return false;
    }
    // LE COMPTEUR DIT CE QUI EST, PAS CE QUE J'AI FAIT.
    //
    // La premiere version sortait ici sans compter le coeur, au motif qu'elle
    // n'avait rien eu a ecrire. Le releve affichait alors `coeurs_pat=0` sur
    // une machine ou la PAT etait pourtant correcte -- et un `0` qui veut dire
    // « tout va bien » est pire qu'une absence de chiffre.
    //
    // Le cas est reel : plusieurs micrologiciels, dont EDK2, programment deja
    // une entree en ecriture combinee pour leur propre usage graphique.
    //
    // Ce qui compte n'est pas qui a ecrit le MSR, mais ce qu'il contient.
    if entree_quatre_est_combinee() {
        COEURS_CONFIGURES.fetch_add(1, Ordering::Relaxed);
        return true;
    }

    // LA SEQUENCE DU MANUEL, ET RIEN D'AUTRE.
    //
    // Intel Vol. 3, « Programming the PAT » : le cache doit etre desactive et
    // vide pendant l'ecriture, sans quoi des lignes ecrites sous l'ancien type
    // survivraient sous le nouveau -- un etat que le processeur ne sait plus
    // decrire, et dont les consequences ne ressemblent a rien de reconnaissable.
    let irq = crate::arch::x86_64::cpu::interrupts_enabled();
    if irq {
        unsafe { asm!("cli", options(nomem, nostack)) };
    }
    unsafe {
        let cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        // CD=1 (bit 30), NW=0 (bit 29) : cache desactive, ecriture immediate.
        asm!("mov cr0, {}", in(reg) (cr0 | (1 << 30)) & !(1u64 << 29),
             options(nostack, preserves_flags));
        asm!("wbinvd", options(nostack, preserves_flags));

        let cr3: u64;
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        asm!("mov cr3, {}", in(reg) cr3, options(nostack, preserves_flags));

        ecrit_msr(IA32_PAT, PAT_AVEC_ECRITURE_COMBINEE);

        asm!("mov cr3, {}", in(reg) cr3, options(nostack, preserves_flags));
        asm!("wbinvd", options(nostack, preserves_flags));
        asm!("mov cr0, {}", in(reg) cr0, options(nostack, preserves_flags));
    }
    if irq {
        unsafe { asm!("sti", options(nomem, nostack)) };
    }

    // On verifie ce qu'on vient d'ecrire : un `wrmsr` accepte sans effet -- sous
    // un hyperviseur qui filtre, par exemple -- laisserait croire au correctif
    // sans le rendre.
    if !entree_quatre_est_combinee() {
        return false;
    }
    COEURS_CONFIGURES.fetch_add(1, Ordering::Relaxed);
    true
}

/// L'entree quatre de CE coeur est-elle en ecriture combinee ?
fn entree_quatre_est_combinee() -> bool {
    (lit_msr(IA32_PAT) >> 32) & 0xFF == TYPE_ECRITURE_COMBINEE
}

/// Valeur brute de la PAT du coeur courant, pour le releve.
pub fn valeur_brute() -> u64 {
    if pat_disponible() { lit_msr(IA32_PAT) } else { 0 }
}
