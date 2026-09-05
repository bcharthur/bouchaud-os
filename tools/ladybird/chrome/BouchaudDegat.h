#pragma once

// BOUCHAUD_CHROME_V18_DEGAT_PARTIEL
//
// # Ce que ce fichier decide
//
// Quels pixels de la surface partagee une nouvelle capture de page doit
// reecrire, et quel rectangle annoncer au compositeur.
//
// # Le defaut qu'il corrige
//
// Chaque capture recopiait la page entiere et annoncait la surface entiere,
// quelle que soit la taille du changement. Sur la page d'accueil de Google --
// dont le champ de recherche prend le focus tout seul, et dont le curseur
// clignote donc deux fois par seconde -- cela donnait, mesure sur la machine
// de l'utilisateur :
//
//     M11_RENDER_STATS full=312 toolbar=40 page=312 pixels=486703296
//     PERF-BROWSER pid=5 frames_delta=61 inputs_delta=0 bottleneck=memory-pagefault
//
// 312 recompositions completes en trois minutes sans une seule entree, a
// 1 554 048 pixels chacune. La page paraissait « se rafraichir en boucle »
// parce qu'elle se rafraichissait reellement en boucle : un curseur de deux
// pixels de large repeignait 1278x626.
//
// Sur une page vraiment statique (example.com) le compteur restait a 3 : le
// modele d'invalidation de LibWeb fonctionne. Ce qui manquait, c'est que sa
// conclusion -- « voici le rectangle qui a change » -- etait calculee, puis
// jetee.
//
// # Pourquoi cette arithmetique vit dans son propre fichier
//
// Elle ne depend de rien : ni de LibGfx, ni du protocole GUI, ni d'une surface
// projetee. Elle se compile donc sur l'hote et `test_degat.cpp` l'exerce a
// chaque CI, la ou le reste du chrome ne s'execute que dans QEMU au bout de
// vingt minutes de construction. Les erreurs que ce code peut commettre -- un
// pixel de trop, un pixel de moins, un rectangle non borne -- laissent des
// trainees a l'ecran et ne font echouer aucun test d'integration.

namespace BouchaudDegat {

struct Rect {
    int x { 0 };
    int y { 0 };
    int w { 0 };
    int h { 0 };

    constexpr bool vide() const { return w <= 0 || h <= 0; }

    constexpr bool operator==(Rect const& autre) const
    {
        // Deux rectangles vides sont le meme rectangle, quelles que soient
        // leurs coordonnees : sans cela, `efface_necessaire` serait vrai des
        // que la capture ne couvre rien, et on effacerait pour rien.
        if (vide() && autre.vide())
            return true;
        return x == autre.x && y == autre.y && w == autre.w && h == autre.h;
    }

    constexpr Rect intersecte(Rect const& autre) const
    {
        if (vide() || autre.vide())
            return {};
        auto const x0 = x > autre.x ? x : autre.x;
        auto const y0 = y > autre.y ? y : autre.y;
        auto const x1 = (x + w) < (autre.x + autre.w) ? (x + w) : (autre.x + autre.w);
        auto const y1 = (y + h) < (autre.y + autre.h) ? (y + h) : (autre.y + autre.h);
        if (x1 <= x0 || y1 <= y0)
            return {};
        return Rect { x0, y0, x1 - x0, y1 - y0 };
    }
};

/// Tout ce dont le plan depend, en une seule valeur comparable.
///
/// La comparaison est ce qui rend le suivi sur : des qu'une de ces cinq
/// grandeurs bouge, la trame suivante est complete. Un degat partiel n'est
/// juste que si la surface porte deja la trame precedente *a la meme
/// geometrie* ; se souvenir de la geometrie evite d'avoir a raisonner sur
/// chaque cas -- fenetre agrandie, barre d'outils plus haute, capture qui
/// retrecit -- separement.
struct Geometrie {
    int surface_largeur { 0 };
    int surface_hauteur { 0 };
    int page_haut { 0 };
    int capture_largeur { 0 };
    int capture_hauteur { 0 };

    constexpr bool operator==(Geometrie const& autre) const
    {
        return surface_largeur == autre.surface_largeur
            && surface_hauteur == autre.surface_hauteur
            && page_haut == autre.page_haut
            && capture_largeur == autre.capture_largeur
            && capture_hauteur == autre.capture_hauteur;
    }

    /// La zone de page, en coordonnees de page (origine sous la barre d'outils).
    constexpr Rect zone_page() const
    {
        auto const hauteur = surface_hauteur - page_haut;
        if (surface_largeur <= 0 || hauteur <= 0)
            return {};
        return Rect { 0, 0, surface_largeur, hauteur };
    }
};

/// Le travail d'une trame.
///
/// `efface` et `copie` sont en coordonnees de PAGE ; `publie` est en
/// coordonnees de SURFACE. Les deux reperes se distinguent d'un decalage de
/// `page_haut`, et les confondre etait la faute la plus facile a commettre
/// ici : elle decale toute la page de la hauteur de la barre d'outils, ce qui
/// se voit tout de suite, ou decale seulement le rectangle publie, ce qui ne
/// se voit que sur les pages qui changent peu.
struct Plan {
    Rect efface {};
    Rect copie {};
    Rect publie {};
    bool efface_necessaire { false };
    bool complet { false };

    constexpr bool rien_a_faire() const { return publie.vide(); }
};

class Suivi {
public:
    /// Exige une trame complete a la prochaine composition.
    ///
    /// A appeler quand la surface a cesse de porter la trame precedente pour
    /// une raison que la geometrie ne dit pas : premiere trame, remappage,
    /// recomposition demandee par le gestionnaire de fenetres.
    void invalide() { m_page_valide = false; }

    bool page_valide() const { return m_page_valide; }

    Plan planifie(Geometrie const& geometrie, Rect degat_page)
    {
        if (geometrie != m_geometrie) {
            m_geometrie = geometrie;
            m_page_valide = false;
        }

        auto const zone = geometrie.zone_page();
        auto const degat = m_page_valide ? degat_page.intersecte(zone) : zone;

        Plan plan;
        if (degat.vide())
            return plan;

        plan.efface = degat;
        plan.copie = degat.intersecte(
            Rect { 0, 0, geometrie.capture_largeur, geometrie.capture_hauteur });
        plan.publie = Rect { degat.x, degat.y + geometrie.page_haut, degat.w, degat.h };
        // Effacer puis recopier par-dessus les memes pixels serait une ecriture
        // double sans effet. L'effacement ne sert qu'a la ou la capture ne
        // couvre pas -- page plus courte que la fenetre, capture retrecie.
        plan.efface_necessaire = plan.copie != degat;
        plan.complet = degat == zone;

        m_page_valide = true;
        return plan;
    }

private:
    Geometrie m_geometrie {};
    bool m_page_valide { false };
};

}
