//! Extinction de la machine.
//!
//! Un noyau qui ne sait pas s'arreter ne peut pas etre teste automatiquement :
//! sans extinction, l'emulateur tourne jusqu'a ce qu'on le tue, et il n'y a
//! aucun code de retour a interpreter. C'est la brique qui manque a
//! [`crate::kernel::autorun`].
//!
//! Les ports `0xF4`, `0x604`, `0xB004` et `0x4004` ne sont pas standard PC :
//! ce sont des conventions d'emulateurs, essayees dans l'ordre du plus
//! specifique au plus general. Elles ne coupent RIEN sur une machine reelle.
//!
//! La vraie extinction vit dans [`crate::kernel::acpi_s5`] : bloc PM1 de la
//! FADT et valeurs `SLP_TYP` de l'objet `\_S5_` du DSDT. Elle est tentee en
//! dernier, une fois que les conventions d'emulateur ont montre qu'elles
//! n'avaient rien coupe.

use crate::arch::x86_64::ports::{outb, outw};

/// Code passe au peripherique `isa-debug-exit` quand tout s'est bien passe.
///
/// QEMU sort avec `(code << 1) | 1`, soit 33 ici : il lui est impossible de
/// rendre 0, puisque 0 signifierait « QEMU s'est arrete tout seul ».
pub const EXIT_OK: u8 = 0x10;
/// Code passe a `isa-debug-exit` en cas d'echec (QEMU sortira avec 35).
pub const EXIT_FAIL: u8 = 0x11;

/// Port du peripherique de test `isa-debug-exit` de QEMU.
const QEMU_DEBUG_EXIT: u16 = 0xF4;
/// Port d'extinction ACPI de QEMU (>= 2.0).
const QEMU_ACPI_SHUTDOWN: u16 = 0x604;
/// Meme role sur les anciennes versions de QEMU et sur Bochs.
const BOCHS_SHUTDOWN: u16 = 0xB004;
/// Meme role sous VirtualBox.
const VBOX_SHUTDOWN: u16 = 0x4004;

/// Arrete la machine, en signalant `code` a l'hote quand c'est possible.
///
/// `code` n'est lu que par le peripherique de test de QEMU : c'est par lui que
/// le lanceur de sondes apprend si le scenario a reussi. Les autres voies
/// eteignent sans rien rapporter.
pub fn shutdown(code: u8) -> ! {
    crate::serial_println!("[kernel] extinction demandee (code {})", code);

    // L'ENREGISTREUR DE VOL EN PREMIER, ET AVANT LA PERSISTANCE.
    //
    // Les trois premieres sessions physiques ont fini au bouton
    // d'alimentation, et chaque archive s'arretait aux premieres dizaines de
    // secondes -- celles ou tout allait bien. Ce sont les dernieres qu'on
    // cherche.
    //
    // Avant la persistance, parce que celle-ci peut echouer et qu'un echec ne
    // doit pas emporter le releve qui l'explique.
    crate::kernel::blackbox::vide_avant_extinction("extinction");

    // La zone persistante n'atteint le disque que sur `fsync` explicite. Un
    // programme qui ecrit sous /persist et se contente de fermer son fichier --
    // ce que fait la plupart du code, et notamment le telechargement d'un
    // fichier par le navigateur -- laissait donc ses octets dans le RAMFS, et
    // l'extinction les emportait. Une persistance qui ne survit qu'aux
    // programmes pensant a `fsync` n'est pas une persistance.
    //
    // L'ecriture a lieu ICI, avant de couper : c'est le dernier instant ou elle
    // est possible, et `synchronise` ecrit son en-tete en dernier, donc une
    // coupure pendant l'operation laisse l'etat precedent plutot qu'un melange.
    //
    // `synchronise` rend `-1` sur ECHEC, pas sur « zone vide » : une zone vide
    // rend zero. Annoncer « rien a ecrire » sur un `-1` presentait donc une
    // panne d'ecriture comme une situation normale -- exactement le genre de
    // message qui a fait chercher la cause de « disk I/O error » ailleurs
    // pendant plusieurs runs. La ligne dit maintenant ce qui s'est passe ; la
    // raison precise, elle, est deja journalisee par `synchronise`.
    match crate::fs::persistance::synchronise() {
        -1 => crate::serial_println!(
            "[kernel] persistance: ECHEC de l'ecriture a l'extinction, /persist n'est pas a jour"
        ),
        ecrits => crate::serial_println!(
            "[kernel] persistance: {} fichier(s) ecrit(s) a l'extinction",
            ecrits
        ),
    }
    unsafe {
        // Ne repond que si QEMU a ete lance avec `-device isa-debug-exit` ;
        // sinon l'ecriture part dans le vide, ce qui est sans consequence.
        outb(QEMU_DEBUG_EXIT, code);
        outw(QEMU_ACPI_SHUTDOWN, 0x2000);
        outw(BOCHS_SHUTDOWN, 0x2000);
        outw(VBOX_SHUTDOWN, 0x3400);
    }

    // LA VRAIE EXTINCTION, EN DERNIER ET SUR LE MATERIEL REEL.
    //
    // Les quatre ecritures ci-dessus sont des conventions d'EMULATEUR. Aucune
    // n'existe sur la machine de reference : choisir « Eteindre » y figeait
    // l'ecran sans couper le courant, et il fallait tenir le bouton
    // d'alimentation -- ce qui emporte le releve de vol qu'on venait d'ecrire.
    //
    // En dernier parce que sous QEMU les conventions ci-dessus ont deja coupe,
    // et que sur une vraie machine elles n'ont rien fait du tout.
    if crate::kernel::acpi_s5::eteint() {
        // `eteint` ne rend la main que sur echec : ce chemin n'est pas atteint.
        halt()
    }
    crate::serial_println!(
        "[kernel] extinction: aucune voie n'a coupe le courant (acpi={})",
        crate::kernel::acpi_s5::disponible() as u8,
    );
    halt()
}

/// Registre de reinitialisation PCI, present sur tout chipset moderne.
const RESET_PCI: u16 = 0xCF9;
/// Port de commande du controleur clavier 8042, qui sait aussi reinitialiser.
const CMD_8042: u16 = 0x64;

/// Redemarre la machine.
///
/// # Trois voies, de la plus propre a la plus brutale
///
/// 1. Le registre `0xCF9` : c'est la voie du chipset, celle qu'utilisent les
///    systemes modernes. `0x02` arme, `0x06` declenche une reinitialisation
///    complete.
/// 2. Le controleur clavier 8042 : la voie historique du PC, encore emulee
///    partout. Elle demande d'attendre que son tampon d'entree soit vide,
///    faute de quoi la commande est perdue en silence.
/// 3. Une IDT vide : le processeur ne trouve plus de gestionnaire, ne trouve
///    pas non plus de double faute, et se reinitialise. C'est laid, c'est
///    documente, et cela ne rate pas.
///
/// L'enregistreur de vol est vide AVANT, comme a l'extinction : un
/// redemarrage est une fin de session comme une autre.
pub fn reboot() -> ! {
    crate::serial_println!("[kernel] redemarrage demande");
    crate::kernel::blackbox::vide_avant_extinction("redemarrage");
    match crate::fs::persistance::synchronise() {
        -1 => crate::serial_println!(
            "[kernel] persistance: ECHEC de l'ecriture au redemarrage, /persist n'est pas a jour"
        ),
        ecrits => crate::serial_println!(
            "[kernel] persistance: {} fichier(s) ecrit(s) au redemarrage",
            ecrits
        ),
    }

    x86_64::instructions::interrupts::disable();
    unsafe {
        // 1. Le chipset.
        outb(RESET_PCI, 0x02);
        outb(RESET_PCI, 0x06);

        // 2. Le 8042. Son tampon d'entree doit etre vide, sinon la commande
        //    est jetee sans rien dire.
        for _ in 0..100_000 {
            if crate::arch::x86_64::ports::inb(CMD_8042) & 0x02 == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        outb(CMD_8042, 0xFE);
    }

    // 3. Une IDT vide : sans gestionnaire ni double faute, le processeur se
    //    reinitialise. Dernier recours, et il aboutit toujours.
    crate::serial_println!("[kernel] redemarrage: repli sur la reinitialisation par IDT vide");
    unsafe {
        let idt_vide: [u8; 10] = [0; 10];
        core::arch::asm!("lidt [{}]", in(reg) idt_vide.as_ptr(), options(nostack));
        core::arch::asm!("int3", options(nostack));
    }
    halt()
}

/// Arrete le processeur pour de bon.
///
/// Interruptions masquees avant le `hlt` : sans cela, la moindre IRQ reveillerait
/// le processeur et la boucle repartirait, consommant un cœur pour rien.
pub fn halt() -> ! {
    loop {
        x86_64::instructions::interrupts::disable();
        x86_64::instructions::hlt();
    }
}
