#pragma once
// P19-LOAD-UNIQUE : machine a etats independante de Ladybird, testable sur l'hote.
// Aucun chargement reel n'est interrompu ni simule. Les taux ne sont pas inventes.
#include <stdint.h>
namespace BouchaudChargement {
inline constexpr uint64_t TICKS_10S = 625;  // GUI tick 16 ms
inline constexpr uint64_t TICKS_30S = 1875;
inline constexpr uint64_t TICKS_FADE = 14;

enum class Phase : unsigned char { Repos, Chargement, Termine, Arrete, Echec };
enum class Transition : unsigned char { Nouvelle, DoublonDebut, Fin, DoublonFin, Arret, Erreur };
struct Etat {
    Phase phase { Phase::Repos };
    uint64_t generation { 0 };
    uint64_t debut_tick { 0 };
    uint64_t fin_tick { 0 };
    uint64_t fenetre_tick { 0 };
    unsigned demarrages_fenetre { 0 };
    unsigned debuts_doublons { 0 };
    bool rechargements_rapproches { false };

    bool actif() const { return phase == Phase::Chargement; }
    bool affiche_fin(uint64_t maintenant) const {
        return phase == Phase::Termine && maintenant >= fin_tick && maintenant - fin_tick < TICKS_FADE;
    }
    bool lent(uint64_t maintenant) const {
        return actif() && maintenant >= debut_tick && maintenant - debut_tick >= TICKS_30S;
    }
    bool signale_rafale(uint64_t maintenant) const {
        return rechargements_rapproches && actif() && maintenant >= fenetre_tick && maintenant - fenetre_tick <= TICKS_10S;
    }
    Transition commence(uint64_t maintenant, bool action_utilisateur = false) {
        if (actif() && !action_utilisateur) {
            ++debuts_doublons;
            return Transition::DoublonDebut;
        }
        if (action_utilisateur || generation == 0 || maintenant < fenetre_tick
            || maintenant - fenetre_tick > TICKS_10S) {
            fenetre_tick = maintenant;
            demarrages_fenetre = 1;
        } else if (demarrages_fenetre < 9999) {
            ++demarrages_fenetre;
        }
        rechargements_rapproches = demarrages_fenetre >= 3;
        phase = Phase::Chargement;
        debut_tick = maintenant;
        ++generation;
        return Transition::Nouvelle;
    }
    Transition termine(uint64_t maintenant) {
        if (!actif()) return Transition::DoublonFin;
        phase = Phase::Termine;
        fin_tick = maintenant;
        return Transition::Fin;
    }
    Transition arrete() {
        phase = Phase::Arrete;
        return Transition::Arret;
    }
    Transition echec() {
        phase = Phase::Echec;
        return Transition::Erreur;
    }
};
}
