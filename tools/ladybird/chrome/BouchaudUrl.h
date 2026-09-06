#pragma once

// BOUCHAUD_C21_HISTORIQUE_ET_FAVORIS
//
// # Ce que ce fichier decide
//
// Quelles adresses le chrome accepte de NAVIGUER, et quelles adresses il
// accepte de relire depuis un fichier.
//
// # Les deux entrees, et pourquoi elles ne sont pas la meme
//
// Une adresse tapee par l'utilisateur et une adresse relue d'un magasin sur
// disque ne meritent pas la meme confiance. La premiere vient de quelqu'un qui
// sait ce qu'il fait ; la seconde vient d'un fichier que ce navigateur ecrit
// mais qu'il n'est pas seul a pouvoir ecrire -- et qui, dans ce portage, vit
// dans un sous-arbre auquel le moteur de rendu a acces.
//
// D'ou deux predicats. Le premier ferme les schemas qui EXECUTENT :
// `javascript:` evalue du script dans le document courant -- c'est l'auto-XSS
// classique, celui qu'on fait coller a quelqu'un dans sa barre d'adresse -- et
// un `data:` de premier niveau fabrique un document a l'origine opaque, ce que
// tous les navigateurs ont fini par bloquer pour la meme raison. Le second
// ajoute ce qu'un fichier peut porter et qu'un clavier ne produit pas : des
// octets de controle, dont ceux qui renversent le sens de lecture.
//
// # Pourquoi c'est un fichier a part
//
// Comme BouchaudNomFichier.h : cela ne depend de rien, donc
// `tools/ladybird/chrome/test_url.cpp` l'exerce sur l'hote a chaque CI. Une
// liste de schemas est exactement le genre de code qu'on relit sans y voir le
// trou.

namespace BouchaudUrl {

/// Longueur maximale d'une adresse retenue dans un magasin.
///
/// Deux kibioctets couvrent tout ce qu'un humain met en signet ; au-dela, ce
/// n'est plus une adresse qu'on relira, c'est une ligne qui fait grossir un
/// fichier qu'on relit a chaque demarrage.
inline constexpr int longueur_max = 2048;

constexpr bool commence_par(char const* texte, int taille, char const* prefixe)
{
    int index = 0;
    while (prefixe[index] != '\0') {
        if (index >= taille || texte[index] != prefixe[index])
            return false;
        ++index;
    }
    return true;
}

/// Ce schema peut-il etre navigue depuis la barre d'adresse ?
///
/// Liste BLANCHE. `javascript:` et `data:` ne sont pas « oublies » : ils sont
/// dehors parce qu'ils executent, et une liste noire aurait laisse passer le
/// prochain schema qui execute.
constexpr bool schema_navigable(char const* url, int taille)
{
    return commence_par(url, taille, "http://")
        || commence_par(url, taille, "https://")
        || commence_par(url, taille, "about:")
        || commence_par(url, taille, "file://");
}

/// Aucun octet de controle, aucun octet non ASCII.
///
/// Ce qu'un clavier ne produit pas et qu'un fichier peut porter : le saut de
/// ligne, qui couperait un enregistrement en deux ; la tabulation, qui est le
/// separateur de champs du magasin ; et les marqueurs bidirectionnels, qui font
/// lire une adresse a l'envers. Une adresse legitime est deja pourcent-encodee
/// bien avant d'arriver ici.
constexpr bool texte_propre(char const* texte, int taille)
{
    for (int index = 0; index < taille; ++index) {
        auto const octet = static_cast<unsigned char>(texte[index]);
        if (octet < 0x20 || octet >= 0x7f)
            return false;
    }
    return true;
}

/// Cette adresse peut-elle etre retenue, ou relue, dans un magasin ?
constexpr bool acceptable_pour_le_magasin(char const* url, int taille)
{
    return taille > 0
        && taille <= longueur_max
        && texte_propre(url, taille)
        && schema_navigable(url, taille);
}

}
