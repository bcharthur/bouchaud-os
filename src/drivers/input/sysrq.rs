//! Alt+SysRq sur le flux brut des scancodes (jeu 1, traduction 8042 active).
//!
//! # Pourquoi ce detecteur existe (#283)
//!
//! Un demarrage a froid de Ladybird se fige parfois apres le fork des rendus :
//! le noyau vit (ses releves periodiques continuent), mais plus aucun
//! processus n'avance. Le resume de trente secondes compte les taches pretes
//! et bloquees, sans dire QUI attend QUOI. Cette information existe deja,
//! fil par fil (`forensic_thread`), mais rien ne permettait de la demander de
//! l'exterieur a une machine figee : le diagnostic distant exige une image
//! tokenisee, et l'invite ne repond plus a rien.
//!
//! Le moniteur QEMU, lui, repond toujours : `sendkey alt-sysrq` fait entrer
//! `0x38` (Alt) puis `0x54` (SysRq tenu avec Alt) par l'IRQ1. Le detecteur
//! vit dans le GESTIONNAIRE D'INTERRUPTION, avant toute file : un consommateur
//! fige ne peut donc pas l'empecher de voir la combinaison.
//!
//! Module PUR : aucun etat global, aucune dependance au noyau, pour etre
//! eprouve sur l'hote (`tools/platform/test_sysrq.rs`).

/// Etat du detecteur : bit 0 Alt gauche, bit 1 Alt droit (AltGr), bit 2
/// prefixe `0xE0` en attente.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetecteurSysRq(pub u8);

const ALT_GAUCHE: u8 = 1 << 0;
const ALT_DROIT: u8 = 1 << 1;
const PREFIXE_E0: u8 = 1 << 2;

/// Code d'appui d'Alt (jeu 1). Precede de `0xE0`, c'est Alt droit.
pub const ALT_APPUI: u8 = 0x38;
/// Code de relachement d'Alt (jeu 1).
pub const ALT_RELACHE: u8 = 0xB8;
/// SysRq : ce que le clavier emet pour Impr. ecran tenu AVEC Alt (jeu 1).
pub const SYSRQ_APPUI: u8 = 0x54;

impl DetecteurSysRq {
    pub const fn nouveau() -> Self {
        Self(0)
    }

    /// Avance d'un scancode. Rend `true` exactement a l'appui de SysRq
    /// pendant qu'un Alt est tenu ; le relachement et la repetition
    /// automatique d'Alt ne declenchent rien.
    pub fn pousse(&mut self, sc: u8) -> bool {
        if sc == 0xE0 {
            self.0 |= PREFIXE_E0;
            return false;
        }
        let etendu = self.0 & PREFIXE_E0 != 0;
        self.0 &= !PREFIXE_E0;
        let alt = if etendu { ALT_DROIT } else { ALT_GAUCHE };
        match sc {
            ALT_APPUI => {
                self.0 |= alt;
                false
            }
            ALT_RELACHE => {
                self.0 &= !alt;
                false
            }
            // `E0 54` n'existe pas en jeu 1 ; seul le code nu est SysRq.
            SYSRQ_APPUI => !etendu && self.0 & (ALT_GAUCHE | ALT_DROIT) != 0,
            _ => false,
        }
    }
}
