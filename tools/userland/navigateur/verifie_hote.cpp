// Verifie que l'hote Qt peint reellement ce que la liste d'affichage decrit.
//
//   ./verifie-hote.sh
//
// ## Pourquoi ce fichier existe
//
// Tout le moteur est eprouve par `test_moteur.py` — mais il s'arrete a la
// liste d'affichage. Ce que Qt en fait ensuite n'etait verifie par rien : les
// operations de peinture ont ete ecrites, relues, et jamais executees. Une
// erreur d'ordre d'arguments, une couleur inversee, un rognage qui ne rogne
// pas ne se seraient vus qu'a l'ecran, sous emulation, des mois plus tard.
//
// Ce banc prend le vrai `hote.cpp`, le fait peindre hors ecran, et **relit les
// pixels**. Il ne demande ni l'OS, ni framebuffer, ni QuickJS : seulement Qt et
// Python, que n'importe quelle machine de developpement a.
//
// ## Ce qu'il ne prouve pas
//
// Que le binaire du navigateur se construit — cela demande Qt statique, CPython
// embarque, QuickJS et ffmpeg. Ici Qt est celui du systeme, et les deux modules
// que `main` enregistre sont remplaces par des bouchons.

// Python avant Qt, comme dans `hote.cpp` : Qt definit `slots` en macro, ce qui
// entre en collision avec un champ du meme nom dans les en-tetes de CPython.
#include <Python.h>
#include <QtCore/qplugin.h>

// Les greffons statiques (framebuffer, JPEG, GIF, ICO, SVG) n'existent que
// dans un Qt construit en statique ; ce banc se lie au Qt du systeme, donc on
// neutralise leur declaration. C'est la seule chose que ce banc ne verifie pas
// du fichier reel.
#undef Q_IMPORT_PLUGIN
#define Q_IMPORT_PLUGIN(x)

#define main hote_main_inutilise
#include "hote.cpp"
#undef main

#include <cstdio>

// `main` du navigateur reference ces deux modules ; ce banc ne les utilise pas.
PyObject *initialise_bojs() { Py_RETURN_NONE; }
PyObject *initialise_bomedia() { Py_RETURN_NONE; }

namespace {

int echecs = 0;
int reussites = 0;

void verifie(const char *quoi, bool vrai, const char *detail = "")
{
    if (vrai) {
        ++reussites;
    } else {
        ++echecs;
        std::printf("  - %s %s\n", quoi, detail);
    }
}

/// Compare un pixel a une couleur, avec la tolerance de l'anticrenelage.
void pixel(const QImage &image, const char *quoi, int x, int y,
           int r, int v, int b, int marge = 12)
{
    if (x < 0 || y < 0 || x >= image.width() || y >= image.height()) {
        verifie(quoi, false, "(hors image)");
        return;
    }
    const QColor c = image.pixelColor(x, y);
    const bool proche = std::abs(c.red() - r) <= marge
                     && std::abs(c.green() - v) <= marge
                     && std::abs(c.blue() - b) <= marge;
    char detail[128];
    std::snprintf(detail, sizeof detail, "— obtenu (%d,%d,%d), attendu (%d,%d,%d)",
                  c.red(), c.green(), c.blue(), r, v, b);
    verifie(quoi, proche, detail);
}

/// Peint une liste d'operations hors ecran et rend l'image.
QImage peint(PyObject *liste, int largeur = 60, int hauteur = 40)
{
    // `rasterise` est l'entree publique de la peinture : c'est elle que
    // `getImageData` emprunte, donc c'est elle qu'il faut eprouver.
    const QImage rendu = g_toile->rasterise(liste, largeur, hauteur);
    QImage fond(largeur, hauteur, QImage::Format_ARGB32);
    fond.fill(Qt::white);
    QPainter peintre(&fond);
    peintre.drawImage(0, 0, rendu);
    peintre.end();
    return fond;
}

PyObject *liste_de(std::initializer_list<PyObject *> operations)
{
    PyObject *liste = PyList_New(0);
    for (PyObject *operation : operations)
        PyList_Append(liste, operation);
    return liste;
}

// --- Les verifications --------------------------------------------------------

void verifie_formes()
{
    PyObject *rect = Py_BuildValue("(sddddl)", "rect", 10.0, 10.0, 30.0, 20.0,
                                   long(0xFFFF0000));
    QImage image = peint(liste_de({rect}));
    pixel(image, "rect: l'interieur est peint", 20, 15, 255, 0, 0);
    pixel(image, "rect: l'exterieur ne l'est pas", 5, 5, 255, 255, 255);
    pixel(image, "rect: le bord droit s'arrete", 45, 15, 255, 255, 255);
    Py_DECREF(rect);

    // Un rectangle arronди : les coins restent blancs, le centre non.
    PyObject *rond = Py_BuildValue("(sdddddl)", "rond", 0.0, 0.0, 40.0, 40.0,
                                   16.0, long(0xFF0000FF));
    image = peint(liste_de({rond}));
    pixel(image, "rond: le centre est peint", 20, 20, 0, 0, 255);
    pixel(image, "rond: le coin est epargne", 1, 1, 255, 255, 255);
    Py_DECREF(rond);

    // Un polygone : c'est ce qui remplit un chemin de toile.
    PyObject *points = Py_BuildValue("[(dd)(dd)(dd)]", 0.0, 0.0, 40.0, 0.0, 0.0, 30.0);
    PyObject *polygone = Py_BuildValue("(sOl)", "polygone", points,
                                       long(0xFF008000));
    image = peint(liste_de({polygone}));
    pixel(image, "polygone: l'interieur est peint", 5, 5, 0, 128, 0);
    pixel(image, "polygone: l'exterieur est epargne", 35, 25, 255, 255, 255);
    Py_DECREF(points);
    Py_DECREF(polygone);
}

void verifie_degrades()
{
    PyObject *etapes = Py_BuildValue("[(dl)(dl)]", 0.0, long(0xFF000000),
                                     1.0, long(0xFFFFFFFF));
    PyObject *pente = Py_BuildValue("(sddddddO)", "degrade", 0.0, 0.0, 60.0, 40.0,
                                    0.0, 90.0, etapes);
    QImage image = peint(liste_de({pente}));
    // 90 degres en CSS pointe vers la droite : noir a gauche, blanc a droite.
    pixel(image, "degrade: sombre a gauche", 2, 20, 0, 0, 0, 30);
    pixel(image, "degrade: clair a droite", 57, 20, 255, 255, 255, 30);
    const int gauche = image.pixelColor(2, 20).red();
    const int droite = image.pixelColor(57, 20).red();
    verifie("degrade: il progresse dans le bon sens", droite > gauche + 100);
    Py_DECREF(pente);

    PyObject *radial = Py_BuildValue("(sddddddO)", "degrade_radial", 0.0, 0.0,
                                     60.0, 40.0, 0.0, 0.0, etapes);
    image = peint(liste_de({radial}));
    const int centre = image.pixelColor(30, 20).red();
    const int bord = image.pixelColor(1, 1).red();
    verifie("degrade radial: sombre au centre, clair au bord", bord > centre + 80);
    Py_DECREF(radial);
    Py_DECREF(etapes);
}

void verifie_ombres_et_bords()
{
    // Une ombre portee : un halo sous la boite, plus dense pres d'elle.
    PyObject *ombre = Py_BuildValue("(sddddddl)", "ombre", 20.0, 10.0, 20.0, 20.0,
                                    0.0, 6.0, long(0x80000000));
    QImage image = peint(liste_de({ombre}));
    const int pres = image.pixelColor(30, 20).red();
    const int loin = image.pixelColor(30, 38).red();
    verifie("ombre: elle assombrit sous la boite", pres < 200, "");
    verifie("ombre: elle s'estompe en s'eloignant", loin > pres);
    Py_DECREF(ombre);

    // Un contour : le trait est peint, l'interieur reste vide.
    PyObject *contour = Py_BuildValue("(sddddddl)", "contour", 5.0, 5.0, 40.0, 30.0,
                                      0.0, 4.0, long(0xFF008000));
    image = peint(liste_de({contour}));
    pixel(image, "contour: le trait est peint", 6, 20, 0, 128, 0, 60);
    pixel(image, "contour: l'interieur reste vide", 25, 20, 255, 255, 255);
    Py_DECREF(contour);

    // Une ombre interieure : le pourtour s'assombrit, le centre non.
    PyObject *interne = Py_BuildValue("(sdddddddddl)", "ombre_interne", 0.0, 0.0,
                                      60.0, 40.0, 0.0, 0.0, 0.0, 8.0, 0.0,
                                      long(0xC0000000));
    image = peint(liste_de({interne}));
    const int bord = image.pixelColor(1, 20).red();
    const int coeur = image.pixelColor(30, 20).red();
    verifie("ombre interne: le bord est assombri", bord < 240, "");
    verifie("ombre interne: le centre l'est moins", coeur > bord);
    Py_DECREF(interne);
}

void verifie_degrades_de_toile()
{
    PyObject *etapes = Py_BuildValue("[(dl)(dl)]", 0.0, long(0xFF000000),
                                     1.0, long(0xFFFFFFFF));
    // Le degrade d'une toile porte ses deux extremites, pas un angle.
    PyObject *points = Py_BuildValue("(sddddddddO)", "degrade_points", 0.0, 0.0,
                                     60.0, 40.0, 0.0, 0.0, 60.0, 0.0, etapes);
    QImage image = peint(liste_de({points}));
    verifie("degrade a points: il progresse de gauche a droite",
            image.pixelColor(57, 20).red() > image.pixelColor(2, 20).red() + 100);
    Py_DECREF(points);

    PyObject *cercle = Py_BuildValue("(sdddddddO)", "degrade_cercle", 0.0, 0.0,
                                     60.0, 40.0, 30.0, 20.0, 30.0, etapes);
    image = peint(liste_de({cercle}));
    verifie("degrade a cercle: il s'eclaircit vers le bord",
            image.pixelColor(1, 20).red() > image.pixelColor(30, 20).red() + 80);
    Py_DECREF(cercle);
    Py_DECREF(etapes);
}

void verifie_enveloppes()
{
    // `transforme` : un rectangle translate doit se peindre ailleurs.
    PyObject *transforme = Py_BuildValue("(sdddddddd)", "transforme",
                                         1.0, 0.0, 0.0, 1.0, 20.0, 0.0, 0.0, 0.0);
    PyObject *rect = Py_BuildValue("(sddddl)", "rect", 0.0, 0.0, 10.0, 10.0,
                                   long(0xFFFF0000));
    PyObject *fin = Py_BuildValue("(s)", "desenveloppe");
    QImage image = peint(liste_de({transforme, rect, fin}));
    pixel(image, "transforme: la boite a bouge", 25, 5, 255, 0, 0);
    pixel(image, "transforme: son ancienne place est libre", 5, 5, 255, 255, 255);
    Py_DECREF(transforme);

    // `opacite` : la meme couleur, plus pale.
    PyObject *opacite = Py_BuildValue("(sd)", "opacite", 0.5);
    image = peint(liste_de({opacite, rect, fin}));
    const QColor pale = image.pixelColor(5, 5);
    verifie("opacite: la couleur est melangee au fond",
            pale.red() > 200 && pale.green() > 100 && pale.green() < 200);
    Py_DECREF(opacite);

    // Une enveloppe non refermee ne doit pas contaminer la suite : le peintre
    // depile ce qui reste a la fin de la liste.
    PyObject *transforme2 = Py_BuildValue("(sdddddddd)", "transforme",
                                          1.0, 0.0, 0.0, 1.0, 20.0, 0.0, 0.0, 0.0);
    PyObject *liste = liste_de({transforme2, rect});
    image = peint(liste);
    image = peint(liste_de({rect}));
    pixel(image, "enveloppe: une liste tronquee ne deteint pas", 5, 5, 255, 0, 0);
    Py_DECREF(transforme2);
    Py_DECREF(liste);

    Py_DECREF(rect);
    Py_DECREF(fin);
}

void verifie_rognage()
{
    PyObject *clip = Py_BuildValue("(sdddd)", "clip", 0.0, 0.0, 20.0, 40.0);
    PyObject *rect = Py_BuildValue("(sddddl)", "rect", 0.0, 0.0, 60.0, 40.0,
                                   long(0xFFFF0000));
    PyObject *declip = Py_BuildValue("(s)", "declip");
    QImage image = peint(liste_de({clip, rect, declip}));
    pixel(image, "clip: ce qui est dedans est peint", 10, 20, 255, 0, 0);
    pixel(image, "clip: ce qui deborde est coupe", 40, 20, 255, 255, 255);

    // Apres `declip`, la peinture reprend toute la surface.
    image = peint(liste_de({clip, declip, rect}));
    pixel(image, "declip: le rognage est bien rendu", 40, 20, 255, 0, 0);
    Py_DECREF(clip);
    Py_DECREF(rect);
    Py_DECREF(declip);
}

void verifie_texte_et_polices()
{
    PyObject *texte = Py_BuildValue("(sddsldiiiis)", "texte", 2.0, 2.0, "Hg",
                                    long(0xFF000000), 24.0, 0, 0, 0, 0, "");
    QImage image = peint(liste_de({texte}));
    int encres = 0;
    for (int y = 0; y < image.height(); ++y)
        for (int x = 0; x < image.width(); ++x)
            if (image.pixelColor(x, y).red() < 128)
                ++encres;
    verifie("texte: des glyphes sont encres", encres > 20);
    Py_DECREF(texte);

    // La mesure doit suivre la taille demandee, sinon la mise en page et la
    // peinture ne s'accordent pas.
    const double petite = g_toile->largeurTexte(QStringLiteral("Bonjour"), 10, false, false);
    const double grande = g_toile->largeurTexte(QStringLiteral("Bonjour"), 30, false, false);
    verifie("mesure: elle grandit avec la taille", grande > petite * 2.0);
    const double gras = g_toile->largeurTexte(QStringLiteral("Bonjour"), 20, true, false);
    const double maigre = g_toile->largeurTexte(QStringLiteral("Bonjour"), 20, false, false);
    verifie("mesure: le gras est plus large", gras >= maigre);
}

void verifie_images()
{
    // Une image brute posee dans le cache, puis peinte : c'est le chemin de
    // `putImageData` et des images de page.
    const int cote = 4;
    QByteArray octets(cote * cote * 4, 0);
    for (int i = 0; i < cote * cote; ++i) {
        octets[i * 4 + 0] = char(0);          // bleu
        octets[i * 4 + 1] = char(0);          // vert
        octets[i * 4 + 2] = char(255);        // rouge
        octets[i * 4 + 3] = char(255);        // alpha
    }
    QImage source(reinterpret_cast<const uchar *>(octets.constData()), cote, cote,
                  QImage::Format_ARGB32);
    g_images.append(source.copy());
    const int identifiant = g_images.size() - 1;

    PyObject *image_op = Py_BuildValue("(sddddi)", "image", 0.0, 0.0, 20.0, 20.0,
                                       identifiant);
    QImage rendue = peint(liste_de({image_op}));
    pixel(rendue, "image: elle est posee", 10, 10, 255, 0, 0);
    Py_DECREF(image_op);

    // `imagepart` : `object-fit: cover` ne montre qu'une portion.
    PyObject *part = Py_BuildValue("(sddddidddd)", "imagepart", 0.0, 0.0, 20.0,
                                   20.0, identifiant, 0.0, 0.0, 2.0, 2.0);
    rendue = peint(liste_de({part}));
    pixel(rendue, "imagepart: la portion est posee", 10, 10, 255, 0, 0);
    Py_DECREF(part);
}

}  // namespace

int main(int argc, char **argv)
{
    qputenv("QT_QPA_PLATFORM", "offscreen");
    QApplication app(argc, argv);
    Py_Initialize();

    Toile toile;
    g_toile = &toile;

    verifie_formes();
    verifie_degrades();
    verifie_degrades_de_toile();
    verifie_ombres_et_bords();
    verifie_enveloppes();
    verifie_rognage();
    verifie_texte_et_polices();
    verifie_images();

    std::printf("\n");
    if (echecs) {
        std::printf("RESULTAT : %d verification(s) en echec sur %d\n",
                    echecs, echecs + reussites);
        return 1;
    }
    std::printf("RESULTAT : 0 verification(s) en echec (%d passees)\n", reussites);
    return 0;
}
