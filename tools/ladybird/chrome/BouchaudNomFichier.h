#pragma once

// BOUCHAUD_C20_TELECHARGEMENTS
//
// # Ce que ce fichier decide
//
// Le nom sous lequel un telechargement est ecrit sur le disque, a partir du
// nom que le SERVEUR propose.
//
// # Pourquoi c'est un fichier a part
//
// Parce que cette entree est hostile. Le nom vient d'un en-tete
// `Content-Disposition`, c'est-a-dire de la machine d'en face, et les trois
// facons de s'en servir sont connues depuis vingt ans :
//
//   * la traversee -- `../ladybird/profile/cookies.sqlite` --, qui ecrit
//     ailleurs que dans le depot ;
//   * le fichier cache -- `.bashrc` --, qui n'apparait pas dans la liste que
//     l'utilisateur regarde ;
//   * le nom demesure, qui deborde ce qui l'accueille.
//
// Le noyau en attrape deja une : le controle de bac a sable porte sur le
// chemin CANONIQUE, `..` deja resolu, et le depot est un sous-arbre. Ce
// fichier-ci est la seconde ligne, et il en attrape trois. Deux lignes valent
// mieux qu'une quand la premiere est a quatre couches de distance de l'endroit
// ou la donnee entre.
//
// Il ne depend de rien -- ni AK, ni LibGfx, ni allocation -- donc
// `tools/ladybird/chrome/test_nom_fichier.cpp` l'exerce sur l'hote a chaque
// CI, la ou le reste ne tourne que dans QEMU au bout de vingt minutes.

namespace BouchaudNomFichier {

/// Longueur maximale, extension comprise.
///
/// Volontairement bien en deca de ce qu'un systeme de fichiers accepte : un
/// nom de deux cent cinquante caracteres est illisible dans une liste, et
/// aucun nom legitime n'en approche.
inline constexpr int longueur_max = 96;

/// Ce qu'on ecrit quand il ne reste rien du nom propose.
inline constexpr char const* nom_par_defaut = "telechargement";

/// Un nom de fichier, en tampon fixe : ce fichier n'alloue pas.
struct Nom {
    char octets[longueur_max + 1] {};
    int taille { 0 };

    constexpr char const* c_str() const { return octets; }
    constexpr bool vide() const { return taille == 0; }
};

/// Ce caractere peut-il figurer tel quel dans un nom de fichier ?
///
/// La liste est BLANCHE et non noire. Une liste noire oblige a penser a tout
/// ce qui est dangereux -- `/`, `\`, `:`, les octets de controle, l'octet nul,
/// l'UTF-8 mal forme, les caracteres de direction qui font lire un nom a
/// l'envers -- et il en reste toujours un. Une liste blanche oblige a penser a
/// ce qui est utile, et ce qui manque se voit tout de suite.
constexpr bool caractere_permis(char c)
{
    return (c >= 'a' && c <= 'z')
        || (c >= 'A' && c <= 'Z')
        || (c >= '0' && c <= '9')
        || c == '.' || c == '-' || c == '_' || c == ' ' || c == '(' || c == ')';
}

/// Rend un nom sur, forme du nom propose.
///
/// Garanties, et ce sont elles que le banc d'essai verifie :
///
///   * jamais vide ;
///   * jamais `.` ni `..` ;
///   * aucun caractere hors de la liste blanche, donc aucun separateur ;
///   * ne commence ni par un point ni par une espace ;
///   * au plus `longueur_max` octets.
constexpr Nom assainit(char const* propose, int taille)
{
    Nom nom;
    if (propose != nullptr) {
        for (int index = 0; index < taille && nom.taille < longueur_max; ++index) {
            auto const c = propose[index];
            // Un caractere interdit devient un souligne plutot que de
            // disparaitre : `rapport/2026.pdf` doit rester lisible comme
            // `rapport_2026.pdf`, et non devenir `rapport2026.pdf`, qui a
            // l'air d'un nom que le serveur aurait choisi.
            nom.octets[nom.taille++] = caractere_permis(c) ? c : '_';
        }
    }

    // Les points et les espaces de TETE sautent. Le point cache le fichier sur
    // un systeme a la convention Unix, et c'est aussi ce qui reste de `..`
    // quand les separateurs ont ete remplaces.
    int debut = 0;
    while (debut < nom.taille && (nom.octets[debut] == '.' || nom.octets[debut] == ' '))
        ++debut;
    if (debut > 0) {
        int ecriture = 0;
        for (int index = debut; index < nom.taille; ++index)
            nom.octets[ecriture++] = nom.octets[index];
        nom.taille = ecriture;
    }

    // Ceux de QUEUE aussi : un nom qui finit par une espace se copie mal et se
    // lit encore plus mal.
    while (nom.taille > 0
        && (nom.octets[nom.taille - 1] == ' ' || nom.octets[nom.taille - 1] == '.')) {
        --nom.taille;
    }

    for (int index = nom.taille; index <= longueur_max; ++index)
        nom.octets[index] = '\0';

    if (nom.taille == 0) {
        int index = 0;
        while (nom_par_defaut[index] != '\0' && index < longueur_max) {
            nom.octets[index] = nom_par_defaut[index];
            ++index;
        }
        nom.taille = index;
        nom.octets[index] = '\0';
    }
    return nom;
}

}
