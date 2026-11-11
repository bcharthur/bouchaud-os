use super::types::{NATIVE_BASE, NATIVE_LAST};

pub const VERSION: u64 = NATIVE_BASE + 0x00;
pub const HANDLE_CLOSE: u64 = NATIVE_BASE + 0x01;
pub const HANDLE_DUP: u64 = NATIVE_BASE + 0x02;
pub const HANDLE_INFO: u64 = NATIVE_BASE + 0x03;
pub const HANDLE_COUNT: u64 = NATIVE_BASE + 0x04;

pub const CHANNEL_CREATE: u64 = NATIVE_BASE + 0x10;
pub const CHANNEL_SEND: u64 = NATIVE_BASE + 0x11;
pub const CHANNEL_RECV: u64 = NATIVE_BASE + 0x12;
/// Envoi avec ATTENUATION des droits transferes.
///
/// `CHANNEL_SEND` reste inchange -- il transfere les droits tels quels --
/// parce qu'un appelant deja compile ne peut pas deviner qu'un sixieme
/// argument est apparu. Le nouveau numero porte un tableau de masques, un par
/// handle : chaque capacite franchit la frontiere avec l'intersection de ses
/// droits et de son masque. C'est ce qui permet a un courtier de donner une
/// vue en LECTURE SEULE d'une region qu'il detient en lecture-ecriture.
pub const CHANNEL_SEND_ATTENUE: u64 = NATIVE_BASE + 0x13;

pub const EVENT_CREATE: u64 = NATIVE_BASE + 0x20;
pub const EVENT_SIGNAL: u64 = NATIVE_BASE + 0x21;
pub const EVENT_RESET: u64 = NATIVE_BASE + 0x22;
pub const EVENT_QUERY: u64 = NATIVE_BASE + 0x23;

pub const WAITSET_CREATE: u64 = NATIVE_BASE + 0x30;
pub const WAITSET_ADD: u64 = NATIVE_BASE + 0x31;
pub const WAITSET_REMOVE: u64 = NATIVE_BASE + 0x32;
pub const WAITSET_POLL: u64 = NATIVE_BASE + 0x33;
/// Attente BLOQUANTE, level-triggered, avec echeance (ABI 1.1).
pub const WAITSET_WAIT: u64 = NATIVE_BASE + 0x34;

pub const SHM_CREATE: u64 = NATIVE_BASE + 0x40;
pub const SHM_SIZE: u64 = NATIVE_BASE + 0x41;
pub const SHM_READ: u64 = NATIVE_BASE + 0x42;
pub const SHM_WRITE: u64 = NATIVE_BASE + 0x43;

pub const PROCESS_SPAWN: u64 = NATIVE_BASE + 0x50;
pub const PROCESS_INFO: u64 = NATIVE_BASE + 0x51;
pub const PROCESS_KILL: u64 = NATIVE_BASE + 0x52;

pub const TIME_NOW: u64 = NATIVE_BASE + 0x60;

/// BOUCHAUD_UPDATE_MECANISME_V1 (ABI 1.2) : sur une capacite Update.
/// Etat du boot (passage du chargeur + bloc META) ; exige INSPECT.
pub const UPDATE_STATE: u64 = NATIVE_BASE + 0x70;
/// Declarer le boot courant sain : commit du candidat booste ; exige
/// UPDATE_HEALTH. Le noyau n'accepte que ce que `passage::commit_sante` accepte.
pub const UPDATE_COMMIT_HEALTH: u64 = NATIVE_BASE + 0x71;
/// Installation dans le slot INACTIF ; exigent UPDATE_INSTALL. begin(cible),
/// write(decalage, tampon, longueur), finish(empreinte*, longueur, essais).
pub const UPDATE_INSTALL_BEGIN: u64 = NATIVE_BASE + 0x72;
pub const UPDATE_INSTALL_WRITE: u64 = NATIVE_BASE + 0x73;
pub const UPDATE_INSTALL_FINISH: u64 = NATIVE_BASE + 0x74;
/// Relecture incrementale (au plus 1 Mio par appel) ; rend les octets restants.
pub const UPDATE_INSTALL_VERIFY: u64 = NATIVE_BASE + 0x75;

/// BOUCHAUD_COMPOSITORD_PRESENTATION_V1 (ABI 1.3) : sur une capacite Display
/// (droit DISPLAY_PRESENT), poser un rectangle d'une region partagee (droit
/// READ) a l'ecran. (affichage, region, demande*) -> empreinte relue.
pub const DISPLAY_PRESENT: u64 = NATIVE_BASE + 0x80;
/// BOUCHAUD_BUREAU_RING3_2A (ABI 1.4) : taille de l'ecran (largeur << 32 |
/// hauteur), sur la capacite Display (droit DISPLAY_PRESENT).
pub const DISPLAY_INFO: u64 = NATIVE_BASE + 0x81;
/// Sur une capacite Fenetre (droit READ) : valider les messages du client et
/// ecrire son etat (fenetre, etat*) -> 1 vivant / 0 mort.
pub const FENETRE_POMPE: u64 = NATIVE_BASE + 0x90;
/// Sur une capacite Fenetre (READ) et une region (WRITE) : recopier un
/// rectangle de la face livree (fenetre, region, lecture*) -> pixels copies.
pub const FENETRE_LIT: u64 = NATIVE_BASE + 0x91;
/// BOUCHAUD_BUREAU_RING3_2B (ABI 1.4) : sur une capacite Fenetre (WRITE),
/// nouvelle zone utile et foyer (fenetre, largeur, hauteur, focus).
pub const FENETRE_CONFIGURE: u64 = NATIVE_BASE + 0x92;
/// Sur une capacite Fenetre (WRITE) : demande de fermeture polie.
pub const FENETRE_FERME: u64 = NATIVE_BASE + 0x93;
/// Sur la capacite Display : relancer le client configure de ce rang ; rend
/// une nouvelle capacite Fenetre (affichage, rang).
pub const DISPLAY_RELANCE_CLIENT: u64 = NATIVE_BASE + 0x82;

#[inline]
pub const fn is_native(number: u64) -> bool {
    number >= NATIVE_BASE && number <= NATIVE_LAST
}

pub const fn name(number: u64) -> &'static str {
    match number {
        VERSION => "bo_version",
        HANDLE_CLOSE => "bo_handle_close",
        HANDLE_DUP => "bo_handle_dup",
        HANDLE_INFO => "bo_handle_info",
        HANDLE_COUNT => "bo_handle_count",
        CHANNEL_CREATE => "bo_channel_create",
        CHANNEL_SEND => "bo_channel_send",
        CHANNEL_RECV => "bo_channel_recv",
        CHANNEL_SEND_ATTENUE => "bo_channel_send_attenue",
        EVENT_CREATE => "bo_event_create",
        EVENT_SIGNAL => "bo_event_signal",
        EVENT_RESET => "bo_event_reset",
        EVENT_QUERY => "bo_event_query",
        WAITSET_CREATE => "bo_waitset_create",
        WAITSET_ADD => "bo_waitset_add",
        WAITSET_REMOVE => "bo_waitset_remove",
        WAITSET_POLL => "bo_waitset_poll",
        WAITSET_WAIT => "bo_waitset_wait",
        SHM_CREATE => "bo_shm_create",
        SHM_SIZE => "bo_shm_size",
        SHM_READ => "bo_shm_read",
        SHM_WRITE => "bo_shm_write",
        PROCESS_SPAWN => "bo_process_spawn",
        PROCESS_INFO => "bo_process_info",
        PROCESS_KILL => "bo_process_kill",
        TIME_NOW => "bo_time_now",
        UPDATE_STATE => "bo_update_state",
        UPDATE_COMMIT_HEALTH => "bo_update_commit_health",
        UPDATE_INSTALL_BEGIN => "bo_update_install_begin",
        UPDATE_INSTALL_WRITE => "bo_update_install_write",
        UPDATE_INSTALL_FINISH => "bo_update_install_finish",
        UPDATE_INSTALL_VERIFY => "bo_update_install_verify",
        DISPLAY_PRESENT => "bo_display_present",
        DISPLAY_INFO => "bo_display_info",
        FENETRE_POMPE => "bo_fenetre_pompe",
        FENETRE_LIT => "bo_fenetre_lit",
        FENETRE_CONFIGURE => "bo_fenetre_configure",
        FENETRE_FERME => "bo_fenetre_ferme",
        DISPLAY_RELANCE_CLIENT => "bo_display_relance_client",
        _ => "bo_unknown",
    }
}
