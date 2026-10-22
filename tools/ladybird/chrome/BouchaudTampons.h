/*
 * Bouchaud OS -- chrome Ladybird : les deux tampons de la surface, cote client
 * (BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1, P10+.3d).
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * Le contrat V1 n'a qu'un tampon : le chrome reecrit la surface pendant que le
 * gestionnaire de fenetres peut la lire (dechirure). Le contrat V2 en a deux
 * dans la meme memoire partagee (le tampon `t` commence a l'octet
 * `t * pas * hauteur`) ; le gestionnaire ne lit que la FACE (le dernier tampon
 * annonce par `FrameReady`) et rend l'ancienne par `Liberation`. La regle du
 * cote noyau est `src/gui/surface_tampons.rs` ; celle-ci est son miroir.
 *
 * Ce que le client doit garantir, et que ce fichier decide :
 *
 *   * il n'ecrit jamais dans la face, ni dans un tampon que le gestionnaire
 *     n'a pas rendu -- `choisit()` rend -1 tant qu'aucun n'est libre ;
 *   * le tampon qu'il va ecrire porte DEUX trames de retard : avant d'y
 *     peindre la suivante, il y recopie depuis la face ce que la derniere
 *     trame a publie (`rejeu`). Ensuite seulement le plan incremental du
 *     chrome (BouchaudDegat) vaut pour lui comme pour une surface unique.
 *
 * Pur, sans AK : verifie sur l'hote par `test_tampons.cpp`.
 */

#pragma once

#include "BouchaudDegat.h"

namespace BouchaudTampons {

/// Bit du troisieme mot de `Hello` : le client sait travailler a deux tampons.
static constexpr unsigned CAPACITE_DEUX_TAMPONS = 1;

using Rect = BouchaudDegat::Rect;

struct Client {
    /// Le contrat retenu : 1 (V1) ou 2.
    int nombre { 1 };
    /// Le client peut ecrire ce tampon (jamais annonce, ou rendu).
    bool libre[2] { true, true };
    /// Ce tampon a deja porte une trame annoncee.
    bool deja_annonce[2] { false, false };
    /// La face : le dernier tampon annonce (-1 : aucun encore).
    int face { -1 };
    /// Ce que la trame de la face a publie (repere de la surface).
    Rect publie_face {};
    /// Le tampon en cours de preparation (-1 : aucun libre).
    int arriere { 0 };
    /// Le rejeu de ce tampon est deja fait depuis sa derniere annonce. Une
    /// annonce RATEE (canal plein) laisse le tampon arriere tel quel : un
    /// second rejeu depuis la face y ecraserait ce qui a ete peint et que le
    /// suivi du chrome croit present.
    bool rejoue[2] { false, false };

    /// La regle du noyau (`Tampons::negocie`) : deux tampons seulement si le
    /// gestionnaire les offre ET si le client les veut.
    static constexpr Client negocie(int offerts, bool voulu)
    {
        Client c;
        c.nombre = (offerts >= 2 && voulu) ? 2 : 1;
        return c;
    }

    /// Le tampon dans lequel peindre la prochaine trame ; -1 s'il faut
    /// attendre une `Liberation`. En V1, toujours 0.
    constexpr int choisit()
    {
        if (nombre == 1) {
            arriere = 0;
            return 0;
        }
        // Un tampon ecrit mais pas annonce (canal plein) reste le bon : il
        // porte deja le debut de la trame.
        if (arriere >= 0 && arriere != face && libre[arriere])
            return arriere;
        for (int t = 0; t < 2; ++t) {
            if (t != face && libre[t]) {
                arriere = t;
                return t;
            }
        }
        arriere = -1;
        return -1;
    }

    /// Ce qu'il faut recopier de la face vers `t` avant d'y peindre, pour que
    /// `t` porte exactement la derniere trame annoncee. Vide en V1, avant la
    /// premiere trame, ou si `t` est la face.
    constexpr Rect rejeu(int t, int largeur, int hauteur) const
    {
        if (nombre == 1 || face < 0 || t == face || t < 0 || t > 1 || rejoue[t])
            return {};
        // Un tampon qui n'a jamais porte de trame ne contient rien d'utile :
        // il recoit toute la surface.
        if (!deja_annonce[t])
            return Rect { 0, 0, largeur, hauteur };
        return publie_face;
    }

    /// Le rejeu calcule par `rejeu(t)` vient d'etre recopie dans `t`.
    constexpr void rejeu_fait(int t)
    {
        if (nombre == 2 && t >= 0 && t <= 1 && t != face)
            rejoue[t] = true;
    }

    /// La trame peinte dans `t` vient d'etre annoncee avec `publie`.
    constexpr void annonce(int t, Rect publie)
    {
        if (nombre == 1 || t < 0 || t > 1)
            return;
        libre[t] = false;
        deja_annonce[t] = true;
        rejoue[t] = false;
        face = t;
        publie_face = publie;
    }

    /// Le gestionnaire rend `t`. Faux (et rien ne change) si ce n'etait pas
    /// attendu : la face, un tampon deja libre, un indice hors bornes.
    constexpr bool libere(int t)
    {
        if (nombre == 1 || t < 0 || t > 1 || t == face || libre[t])
            return false;
        libre[t] = true;
        return true;
    }
};

} // namespace BouchaudTampons
