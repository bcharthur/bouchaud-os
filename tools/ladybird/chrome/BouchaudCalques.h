#pragma once

#include "BouchaudDegat.h"

// BOUCHAUD_CHROME_V19_CALQUES
//
// # Ce que ce fichier decide
//
// Quels pixels une surface FLOTTANTE du chrome -- bulle de survol, barre de
// recherche, menu contextuel -- oblige a reecrire quand elle apparait, se
// deplace, change de contenu ou disparait.
//
// # Le probleme qu'il resout
//
// Depuis BouchaudDegat.h, une capture de page ne recopie que le rectangle que
// le moteur a signale. C'est ce qui a fait tomber le nombre de pixels ecrits
// d'un facteur cent. Mais cela pose une contrainte a tout ce qu'on dessine
// PAR-DESSUS la page : ces pixels-la n'appartiennent a personne. Le moteur ne
// les connait pas, donc il ne les signalera jamais comme changes ; et la
// surface les porte encore quand la bulle disparait.
//
// Deux fautes symetriques guettent, et toutes deux laissent des trainees :
//
//   * la bulle disparait, personne ne demande de reecrire la ou elle etait :
//     elle reste a l'ecran jusqu'a la prochaine trame complete ;
//   * la page se repeint sous la bulle, la bulle n'est pas redessinee : un
//     trou rectangulaire apparait dedans.
//
// La regle qui ferme les deux tient en deux phrases, et c'est tout ce que fait
// ce fichier :
//
//   1. le degat d'un calque qui a bouge est l'union de son ancienne et de sa
//      nouvelle boite -- c'est ce que `degat()` calcule, et il s'ajoute au
//      degat de page AVANT la copie, donc la page est restauree dessous ;
//   2. tout calque qui intersecte le rectangle publie est redessine APRES la
//      copie -- c'est la regle de dessin, exercee par `test_calques.cpp`.
//
// # Pourquoi il ne connait ni pixel ni police
//
// Comme BouchaudDegat.h : il ne depend de rien, se compile sur l'hote, et ses
// erreurs -- un pixel de trop, une boite oubliee -- ne font echouer aucun test
// d'integration. Elles se voient a l'ecran, vingt minutes de construction plus
// tard, sur une capture que personne ne regarde au pixel pres.

namespace BouchaudCalques {

using Rect = BouchaudDegat::Rect;

/// Combien de calques ce suivi peut porter.
///
/// Ce fichier ne sait pas ce qu'est une « bulle de survol » ni un « menu
/// contextuel » : ce sont des notions du chrome, et les nommer ici obligerait
/// a modifier l'arithmetique -- et son banc d'essai -- a chaque element
/// d'interface ajoute. Il ne connait que des rectangles numerotes.
///
/// C'est le chrome qui tient l'enumeration, et un `static_assert` chez lui
/// transforme un depassement de cette borne en erreur de compilation plutot
/// qu'en calque silencieusement ignore.
inline constexpr int maximum = 8;

/// Ce que la surface porte deja des calques, et ce qu'il faut donc reecrire.
///
/// Toutes les coordonnees sont des coordonnees de SURFACE : un calque flotte
/// au-dessus de la fenetre entiere, pas au-dessus de la page. C'est
/// l'appelant qui traduit vers le repere de page pour le degat -- la meme
/// frontiere que dans BouchaudDegat.h, et pour la meme raison : les deux
/// reperes se distinguent d'un decalage constant, et les confondre est la
/// faute la plus facile a commettre ici.
class Suivi {
public:
    /// Ou le calque doit se trouver a la trame qui vient. Boite vide = absent.
    constexpr void place(int calque, Rect boite)
    {
        if (valide(calque))
            m_courant[calque] = boite;
    }

    /// Le contenu du calque a change sans que sa boite bouge.
    ///
    /// « 1/5 » qui devient « 2/5 » occupe les memes pixels : sans ce signal,
    /// `degat()` conclurait qu'il n'y a rien a faire et le compteur resterait
    /// fige.
    constexpr void salit(int calque)
    {
        if (valide(calque))
            m_sale[calque] = true;
    }

    constexpr Rect boite(int calque) const
    {
        return valide(calque) ? m_courant[calque] : Rect {};
    }

    constexpr bool visible(int calque) const { return !boite(calque).vide(); }

    /// Le rectangle de SURFACE que les calques obligent a reecrire.
    ///
    /// Pour un calque qui a bouge, c'est l'union de l'ancienne et de la
    /// nouvelle boite : la premiere parce qu'il faut restaurer la page
    /// dessous, la seconde parce qu'il faut l'y dessiner. Prendre seulement la
    /// nouvelle est l'erreur qui laisse la trainee.
    constexpr Rect degat() const
    {
        Rect total {};
        for (int i = 0; i < maximum; ++i) {
            if (!(m_courant[i] == m_precedent[i]))
                total = total.englobe(m_precedent[i]).englobe(m_courant[i]);
            else if (m_sale[i])
                total = total.englobe(m_courant[i]);
        }
        return total;
    }

    /// La trame est peinte et publiee : ce qui etait voulu est maintenant porte.
    ///
    /// A n'appeler QUE si la trame a reellement ete dessinee. L'appeler sur une
    /// trame abandonnee ferait croire au suivi que la surface porte un calque
    /// qui n'y a jamais ete ecrit, et il ne le redemanderait plus jamais.
    constexpr void acte()
    {
        for (int i = 0; i < maximum; ++i) {
            m_precedent[i] = m_courant[i];
            m_sale[i] = false;
        }
    }

private:
    static constexpr bool valide(int calque) { return calque >= 0 && calque < maximum; }

    Rect m_precedent[maximum] {};
    Rect m_courant[maximum] {};
    bool m_sale[maximum] {};
};

/// Faut-il redessiner ce calque, sachant que `publie` va etre reecrit ?
///
/// La question se pose pour TOUS les calques visibles, pas seulement pour ceux
/// qui ont bouge : une capture de page qui repeint sous un calque immobile
/// effacerait ses pixels. C'est une fonction libre et non une methode parce
/// qu'elle ne consulte aucun etat -- et parce qu'ecrite ici, elle est exercee
/// par le meme test que le reste.
constexpr bool doit_redessiner(Rect boite, Rect publie)
{
    return !boite.intersecte(publie).vide();
}

}
