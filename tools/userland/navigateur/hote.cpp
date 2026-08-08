// Hote Python+Qt de Bouchaud OS : un interprete qui sait peindre.
//
// Un seul processus, un seul binaire statique. Qt tient la fenetre, le
// framebuffer et les entrees ; Python tient tout le reste. Entre les deux, le
// module `bo` defini ici — et rien d'autre.
//
// Ce binaire n'est pas « le navigateur » : c'est le socle. Il execute le script
// Python qu'on lui donne, et ce script decide de ce qui s'affiche. Le
// navigateur natif (`navigateur.py`) en est un ; le backend Bouchaud OS de
// pywebview (`webview/platforms/bouchaud.py`) en est un autre, ce qui permet de
// faire tourner du code pywebview sans le modifier.
//
// ## Pourquoi pas PyQt
//
// PyQt expose les 200 000 lignes d'API de Qt a Python. Un navigateur n'en
// utilise qu'une poignee : ouvrir une fenetre, peindre des rectangles et du
// texte, mesurer ce texte, recevoir des touches et des clics. Le pont ci-dessous
// fait exactement cela en quelques centaines de lignes, se construit en dix
// secondes, et n'a pas besoin d'un PyQt statique — chose qui n'existe pas
// vraiment.
//
// ## Sens du flux
//
// Qt appelle Python, jamais l'inverse pendant la peinture :
//
//   paintEvent    -> rappel « peindre » -> liste d'affichage -> QPainter
//   keyPressEvent -> rappel « touche »
//   mousePress    -> rappel « clic »
//   wheelEvent    -> rappel « molette »
//
// Une liste d'affichage est une liste de tuples. Le code Python la produit,
// l'hote la peint. Cette frontiere est ce qui permet d'ecrire un moteur de rendu
// entier en Python sans jamais toucher a Qt.
//
// ## Verrou global
//
// `bo.boucle()` relache le GIL pendant `exec()`, et chaque rappel le reprend.
// Sans cela, un programme qui lance un fil Python — ce que fait pywebview pour
// executer la fonction passee a `webview.start()` — se bloquerait net : le fil
// principal garderait le verrou pour toute la duree de la boucle d'evenements.

// Python.h vient en premier, et ce n'est pas une preference de style : Qt
// definit `slots` comme mot-cle, et `PyType_Spec` a un champ qui porte ce nom.
// L'inclure apres Qt transforme la declaration en erreur de syntaxe.
#define PY_SSIZE_T_CLEAN
#include <Python.h>

#include "bojs.h"
#include "bomedia.h"

#include <QtWidgets/QApplication>
#include <QtWidgets/QWidget>
#include <QtGui/QPainter>
#include <QtGui/QFontMetricsF>
#include <QtGui/QImage>
#include <QtGui/QImageReader>
#include <QtGui/QGuiApplication>
#include <QtGui/QScreen>
#include <QtGui/QKeyEvent>
#include <QtGui/QMouseEvent>
#include <QtGui/QWheelEvent>
#include <QtGui/QLinearGradient>
#include <QtGui/QRadialGradient>
#include <QtGui/QPainterPath>
#include <QtGui/QPolygonF>
#include <QtGui/QFontDatabase>
#include <QtGui/QTransform>
#include <QtCore/QTimer>
#include <QtCore/QtPlugin>

#include <algorithm>
#include <cmath>
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>

Q_IMPORT_PLUGIN(QLinuxFbIntegrationPlugin)

// Les formats d'image sont des greffons Qt, et un binaire statique ne charge
// aucun greffon : sans ces trois lignes, `QImage::loadFromData` ne saurait lire
// que le PNG et le BMP, qui sont dans QtGui meme. Les declarer ici les lie en
// dur, comme la plateforme d'affichage.
Q_IMPORT_PLUGIN(QJpegPlugin)
Q_IMPORT_PLUGIN(QGifPlugin)
Q_IMPORT_PLUGIN(QICOPlugin)
// Le SVG vient du module QtSvg, pas de qtbase : un site recent y met son logo,
// ses icones et ses pictogrammes, et ils manquaient tous en silence.
Q_IMPORT_PLUGIN(QSvgPlugin)

namespace {

class Toile;
Toile *g_toile = nullptr;
/// Rappels enregistres par le script Python, par nom.
PyObject *g_rappels = nullptr;

// --- Outils de conversion ---------------------------------------------------

QColor couleurDepuisEntier(long valeur)
{
    // 0xAARRGGBB ; un alpha nul est interprete comme opaque, pour que
    // 0x336699 s'ecrive sans avoir a penser a l'alpha.
    const int a = (valeur >> 24) & 0xFF;
    return QColor((valeur >> 16) & 0xFF, (valeur >> 8) & 0xFF, valeur & 0xFF,
                  a == 0 ? 255 : a);
}

// --- Cache d'images ---------------------------------------------------------

/// Images decodees, designees par leur indice dans ce tableau.
///
/// Le decodage a lieu une fois, au chargement ; la liste d'affichage ne porte
/// qu'un entier. Sans ce detour, chaque rafraichissement redecoderait tous les
/// PNG de la page — a 60 trames par seconde et sous emulation, c'est la
/// difference entre une page qui defile et une page qui rame.
QVector<QImage> g_images;

const QImage *imageParIdentifiant(int identifiant)
{
    if (identifiant < 0 || identifiant >= g_images.size())
        return nullptr;
    const QImage &image = g_images.at(identifiant);
    return image.isNull() ? nullptr : &image;
}

QFont fabriqueFonte(double taille, bool gras, bool italique, bool fixe,
                    const QString &famille = QString())
{
    QFont f;
    // La famille demandee par la page passe avant tout : c'est elle que
    // `@font-face` a chargee, et sans elle un site s'affichait toujours dans la
    // police du systeme. Qt substitue de lui-meme si elle est absente.
    if (!famille.isEmpty())
        f.setFamily(famille);
    else if (fixe)
        f.setFamily(QStringLiteral("DejaVu Sans Mono"));
    f.setPixelSize(qMax(1, int(taille + 0.5)));
    f.setBold(gras);
    f.setItalic(italique);
    return f;
}

/// Appelle un rappel enregistre, arguments compris.
///
/// Le verrou est pris **avant** la construction des arguments, et pas seulement
/// autour de l'appel. C'est essentiel : `bo.boucle()` relache le verrou pour la
/// duree de la boucle d'evenements, et le moindre `Py_BuildValue` fait sans lui
/// alloue un tuple en lisant un etat d'interprete que le fil courant n'a plus —
/// c'est-a-dire un dereferencement de pointeur nul, au premier redessin venu.
void appelle(const char *nom, const char *format = nullptr, ...)
{
    PyGILState_STATE etat = PyGILState_Ensure();

    PyObject *arguments = nullptr;
    if (format) {
        va_list liste;
        va_start(liste, format);
        arguments = Py_VaBuildValue(format, liste);
        va_end(liste);
    } else {
        arguments = PyTuple_New(0);
    }

    if (arguments && g_rappels) {
        PyObject *fonction = PyDict_GetItemString(g_rappels, nom); // reference empruntee
        if (fonction && PyCallable_Check(fonction)) {
            PyObject *resultat = PyObject_CallObject(fonction, arguments);
            if (resultat)
                Py_DECREF(resultat);
            else
                PyErr_Print();
        }
    }
    Py_XDECREF(arguments);
    PyGILState_Release(etat);
}

// --- Widget de rendu --------------------------------------------------------

/// La surface. Elle ne sait rien du web : elle demande une liste d'affichage a
/// Python et la peint.
class Toile : public QWidget
{
public:
    Toile()
    {
        setFocusPolicy(Qt::StrongFocus);
        setMouseTracking(true);
    }

    /// Mesure un texte avec la fonte demandee. Le rendu en a besoin pour la
    /// mise en page — c'est la seule chose que Python ne peut pas calculer seul.
    double largeurTexte(const QString &texte, double taille, bool gras, bool fixe,
                        const QString &famille = QString())
    {
        // La famille compte ici autant qu'a la peinture : mesurer dans une
        // police et peindre dans une autre decale tout le texte de la page.
        QFontMetricsF metriques(fabriqueFonte(taille, gras, false, fixe, famille));
        return metriques.horizontalAdvance(texte);
    }

    double hauteurLigne(double taille, bool fixe)
    {
        QFontMetricsF metriques(fabriqueFonte(taille, false, false, fixe));
        return metriques.height();
    }

protected:
    void paintEvent(QPaintEvent *) override
    {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setRenderHint(QPainter::TextAntialiasing, true);
        p.fillRect(rect(), QColor(255, 255, 255));

        // Tout se fait sous le verrou, d'un bloc : construire les arguments,
        // obtenir la liste d'affichage, la parcourir, la relacher.
        PyGILState_STATE etat = PyGILState_Ensure();
        if (g_rappels) {
            PyObject *fonction = PyDict_GetItemString(g_rappels, "peindre");
            if (fonction && PyCallable_Check(fonction)) {
                PyObject *arguments = Py_BuildValue("(ii)", width(), height());
                PyObject *liste = arguments ? PyObject_CallObject(fonction, arguments)
                                            : nullptr;
                Py_XDECREF(arguments);
                if (liste) {
                    peintListe(p, liste);
                    Py_DECREF(liste);
                } else {
                    PyErr_Print();
                }
            }
        }
        PyGILState_Release(etat);
    }

    void keyPressEvent(QKeyEvent *e) override
    {
        appelle("touche", "(isi)", e->key(), e->text().toUtf8().constData(),
                int(e->modifiers()));
        update();
    }

    void mousePressEvent(QMouseEvent *e) override
    {
        appelle("clic", "(ii)", e->x(), e->y());
        update();
    }

    void mouseMoveEvent(QMouseEvent *e) override
    {
        appelle("survol", "(ii)", e->x(), e->y());
    }

    void wheelEvent(QWheelEvent *e) override
    {
        appelle("molette", "(i)", e->angleDelta().y());
        update();
    }

    void closeEvent(QCloseEvent *e) override
    {
        appelle("fermeture");
        QWidget::closeEvent(e);
    }

private:
    /// Peint une liste d'affichage.
    ///
    /// Chaque element est un tuple dont le premier champ nomme l'operation. Le
    /// format est volontairement plat et sans objets : c'est ce qui permet au
    /// code Python de la construire sans jamais dialoguer avec Qt.
    void peintListe(QPainter &p, PyObject *liste)
    {
        const Py_ssize_t n = PySequence_Size(liste);
        if (n < 0) {
            PyErr_Clear();
            return;
        }
        // Une liste d'affichage commence toujours sans rognage en cours : la
        // pile est videe ici pour qu'une liste mal fermee ne contamine pas la
        // trame suivante.
        pileRognage.clear();
        enveloppes = 0;
        for (Py_ssize_t i = 0; i < n; ++i) {
            PyObject *element = PySequence_GetItem(liste, i);
            if (!element)
                break;
            peintElement(p, element);
            Py_DECREF(element);
        }
        // Une liste tronquee — page changee en cours de peinture — laisserait
        // des etats empiles, et la trame suivante peindrait transformee.
        while (enveloppes > 0) {
            p.restore();
            --enveloppes;
        }
    }

    void peintElement(QPainter &p, PyObject *element)
    {
        PyObject *tete = PySequence_GetItem(element, 0);
        if (!tete)
            return;
        const char *operation = PyUnicode_AsUTF8(tete);
        if (!operation) {
            Py_DECREF(tete);
            PyErr_Clear();
            return;
        }

        if (!std::strcmp(operation, "rect")) {
            double x, y, l, h;
            long couleur;
            if (PyArg_ParseTuple(element, "sddddl", &operation, &x, &y, &l, &h, &couleur)) {
                p.setPen(Qt::NoPen);
                p.setBrush(couleurDepuisEntier(couleur));
                p.drawRect(QRectF(x, y, l, h));
            }
        } else if (!std::strcmp(operation, "rond")) {
            double x, y, l, h, rayon;
            long couleur;
            if (PyArg_ParseTuple(element, "sdddddl", &operation, &x, &y, &l, &h, &rayon, &couleur)) {
                p.setPen(Qt::NoPen);
                p.setBrush(couleurDepuisEntier(couleur));
                p.drawRoundedRect(QRectF(x, y, l, h), rayon, rayon);
            }
        } else if (!std::strcmp(operation, "ombre")) {
            // Qt ne floute pas un rectangle. L'ombre est donc empilee en
            // couches concentriques, chacune un peu plus grande et n'apportant
            // qu'une fraction de l'opacite : leur somme redonne un bord doux,
            // pour le prix de quelques `drawRoundedRect`.
            double x, y, l, h, rayon, flou;
            long couleur;
            if (PyArg_ParseTuple(element, "sddddddl", &operation, &x, &y, &l, &h,
                                 &rayon, &flou, &couleur)) {
                const int couches = std::max(1, std::min(16, int(std::ceil(flou))));
                QColor teinte = couleurDepuisEntier(couleur);
                const double part = teinte.alphaF() / double(couches + 1);
                p.setPen(Qt::NoPen);
                for (int i = couches; i >= 0; --i) {
                    const double extension = flou * double(i) / double(couches);
                    QColor couche = teinte;
                    couche.setAlphaF(part);
                    p.setBrush(couche);
                    p.drawRoundedRect(
                        QRectF(x - extension, y - extension,
                               l + 2 * extension, h + 2 * extension),
                        rayon + extension, rayon + extension);
                }
            }
        } else if (!std::strcmp(operation, "degrade")) {
            double x, y, l, h, rayon, angle;
            PyObject *etapes = nullptr;
            if (PyArg_ParseTuple(element, "sddddddO", &operation, &x, &y, &l, &h,
                                 &rayon, &angle, &etapes)) {
                // L'angle de CSS part du haut et tourne dans le sens des
                // aiguilles ; celui de Qt est un simple couple de points. La
                // ligne du degrade passe par le centre, et sa longueur est
                // celle de la projection de la boite sur sa direction.
                const double radians = angle * 3.14159265358979323846 / 180.0;
                const double dx = std::sin(radians);
                const double dy = -std::cos(radians);
                const double portee = std::fabs(l * dx) + std::fabs(h * dy);
                const QPointF centre(x + l / 2.0, y + h / 2.0);
                QLinearGradient pente(centre - QPointF(dx, dy) * portee / 2.0,
                                      centre + QPointF(dx, dy) * portee / 2.0);
                remplitEtapes(pente, etapes);
                p.setPen(Qt::NoPen);
                p.setBrush(pente);
                if (rayon > 0.0)
                    p.drawRoundedRect(QRectF(x, y, l, h), rayon, rayon);
                else
                    p.drawRect(QRectF(x, y, l, h));
            }
        } else if (!std::strcmp(operation, "degrade_radial")) {
            double x, y, l, h, rayon, angle;
            PyObject *etapes = nullptr;
            if (PyArg_ParseTuple(element, "sddddddO", &operation, &x, &y, &l, &h,
                                 &rayon, &angle, &etapes)) {
                // Un cercle centre, de rayon la demi-diagonale : c'est le
                // `farthest-corner` de CSS, son defaut, et ce que demandent
                // presque toutes les pages.
                const QPointF centre(x + l / 2.0, y + h / 2.0);
                const double portee = std::sqrt(l * l + h * h) / 2.0;
                QRadialGradient pente(centre, portee);
                remplitEtapes(pente, etapes);
                p.setPen(Qt::NoPen);
                p.setBrush(pente);
                if (rayon > 0.0)
                    p.drawRoundedRect(QRectF(x, y, l, h), rayon, rayon);
                else
                    p.drawRect(QRectF(x, y, l, h));
            }
        } else if (!std::strcmp(operation, "ombre_interne")) {
            // Une ombre interieure est un halo le long du bord interne. On la
            // dessine comme l'ombre portee — des couches concentriques — mais
            // rognee a la boite et **decalee vers l'exterieur** : ce qui reste
            // visible est la frange qui rentre.
            double x, y, l, h, rayon, dx, dy, flou, etendue;
            long couleur;
            if (PyArg_ParseTuple(element, "sdddddddddl", &operation, &x, &y, &l, &h,
                                 &rayon, &dx, &dy, &flou, &etendue, &couleur)) {
                p.save();
                QPainterPath dedans;
                if (rayon > 0.0)
                    dedans.addRoundedRect(QRectF(x, y, l, h), rayon, rayon);
                else
                    dedans.addRect(QRectF(x, y, l, h));
                p.setClipPath(dedans, Qt::IntersectClip);

                const int couches = std::max(1, std::min(16, int(std::ceil(flou + etendue))));
                QColor teinte = couleurDepuisEntier(couleur);
                const double part = teinte.alphaF() / double(couches + 1);
                QPen stylo;
                stylo.setColor(teinte);
                p.setBrush(Qt::NoBrush);
                for (int i = couches; i >= 0; --i) {
                    const double epaisseur = (flou + etendue) * double(i + 1)
                                             / double(couches + 1);
                    QColor couche = teinte;
                    couche.setAlphaF(part);
                    stylo.setColor(couche);
                    stylo.setWidthF(std::max(1.0, epaisseur));
                    p.setPen(stylo);
                    const double moitie = stylo.widthF() / 2.0;
                    p.drawRoundedRect(
                        QRectF(x + dx + moitie, y + dy + moitie,
                               l - stylo.widthF(), h - stylo.widthF()),
                        std::max(0.0, rayon - moitie), std::max(0.0, rayon - moitie));
                }
                p.restore();
            }
        } else if (!std::strcmp(operation, "polygone")) {
            // Un chemin rempli, exactement. La toile rendait jusqu'ici sa boite
            // englobante : juste pour un `rect()`, faux pour un camembert.
            PyObject *points = nullptr;
            long couleur;
            if (PyArg_ParseTuple(element, "sOl", &operation, &points, &couleur)) {
                QPolygonF forme;
                const Py_ssize_t nb = PySequence_Size(points);
                for (Py_ssize_t k = 0; k < nb; ++k) {
                    PyObject *point = PySequence_GetItem(points, k);
                    if (!point)
                        break;
                    double px = 0.0, py = 0.0;
                    if (PyArg_ParseTuple(point, "dd", &px, &py))
                        forme << QPointF(px, py);
                    else
                        PyErr_Clear();
                    Py_DECREF(point);
                }
                if (forme.size() >= 3) {
                    p.setPen(Qt::NoPen);
                    p.setBrush(couleurDepuisEntier(couleur));
                    p.drawPolygon(forme);
                }
            }
        } else if (!std::strcmp(operation, "degrade_points")) {
            // Le degrade d'une toile porte ses deux extremites, pas un angle :
            // les ramener a un angle perdrait la longueur de la ligne, donc
            // l'endroit exact ou chaque couleur tombe.
            double x, y, l, h, x0, y0, x1, y1;
            PyObject *etapes = nullptr;
            if (PyArg_ParseTuple(element, "sddddddddO", &operation, &x, &y, &l, &h,
                                 &x0, &y0, &x1, &y1, &etapes)) {
                QLinearGradient pente(QPointF(x0, y0), QPointF(x1, y1));
                remplitEtapes(pente, etapes);
                p.setPen(Qt::NoPen);
                p.setBrush(pente);
                p.drawRect(QRectF(x, y, l, h));
            }
        } else if (!std::strcmp(operation, "degrade_cercle")) {
            double x, y, l, h, cx, cy, rayon;
            PyObject *etapes = nullptr;
            if (PyArg_ParseTuple(element, "sdddddddO", &operation, &x, &y, &l, &h,
                                 &cx, &cy, &rayon, &etapes)) {
                QRadialGradient pente(QPointF(cx, cy), rayon > 0.0 ? rayon : 1.0);
                remplitEtapes(pente, etapes);
                p.setPen(Qt::NoPen);
                p.setBrush(pente);
                p.drawRect(QRectF(x, y, l, h));
            }
        } else if (!std::strcmp(operation, "contour")) {
            double x, y, l, h, rayon, epaisseur;
            long couleur;
            if (PyArg_ParseTuple(element, "sddddddl", &operation, &x, &y, &l, &h,
                                 &rayon, &epaisseur, &couleur)) {
                QPen stylo(couleurDepuisEntier(couleur));
                stylo.setWidthF(epaisseur);
                p.setPen(stylo);
                p.setBrush(Qt::NoBrush);
                // Le trait est centre sur le chemin : le rentrer d'une
                // demi-epaisseur le garde a l'interieur de la boite.
                const double moitie = epaisseur / 2.0;
                p.drawRoundedRect(
                    QRectF(x + moitie, y + moitie, l - epaisseur, h - epaisseur),
                    std::max(0.0, rayon - moitie), std::max(0.0, rayon - moitie));
            }
        } else if (!std::strcmp(operation, "transforme")) {
            // `transform` et `opacity` valent pour la boite et sa descendance :
            // l'etat du peintre est empile ici et rendu par « desenveloppe ».
            double a, b, c, d, e, f, ox, oy;
            if (PyArg_ParseTuple(element, "sdddddddd", &operation, &a, &b, &c, &d,
                                 &e, &f, &ox, &oy)) {
                p.save();
                // L'origine par defaut de CSS est le centre de la boite : on s'y
                // place, on transforme, on revient.
                p.translate(ox, oy);
                p.setWorldTransform(QTransform(a, b, c, d, e, f), true);
                p.translate(-ox, -oy);
                ++enveloppes;
            }
        } else if (!std::strcmp(operation, "opacite")) {
            double alpha;
            if (PyArg_ParseTuple(element, "sd", &operation, &alpha)) {
                p.save();
                p.setOpacity(p.opacity() * alpha);
                ++enveloppes;
            }
        } else if (!std::strcmp(operation, "desenveloppe")) {
            if (enveloppes > 0) {
                p.restore();
                --enveloppes;
            }
        } else if (!std::strcmp(operation, "texte")) {
            double x, y, taille;
            long couleur;
            int gras, italique, fixe, souligne;
            const char *texte;
            const char *famille = "";
            if (PyArg_ParseTuple(element, "sddsldpppps", &operation, &x, &y, &texte,
                                 &couleur, &taille, &gras, &italique, &fixe,
                                 &souligne, &famille)) {
                QFont f = fabriqueFonte(taille, gras, italique, fixe,
                                        QString::fromUtf8(famille));
                f.setUnderline(souligne);
                p.setFont(f);
                p.setPen(couleurDepuisEntier(couleur));
                QFontMetricsF metriques(f);
                // `y` est le haut de la ligne ; Qt dessine sur la ligne de base.
                p.drawText(QPointF(x, y + metriques.ascent()),
                           QString::fromUtf8(texte));
            }
        } else if (!std::strcmp(operation, "ligne")) {
            double x1, y1, x2, y2, epaisseur;
            long couleur;
            if (PyArg_ParseTuple(element, "sdddddl", &operation, &x1, &y1, &x2, &y2,
                                 &epaisseur, &couleur)) {
                QPen stylo(couleurDepuisEntier(couleur));
                stylo.setWidthF(epaisseur);
                p.setPen(stylo);
                p.drawLine(QPointF(x1, y1), QPointF(x2, y2));
            }
        } else if (!std::strcmp(operation, "clip")) {
            // Les rognages s'emboitent : la page rogne d'abord la zone de
            // contenu sous la barre d'outils, puis chaque `overflow: hidden`
            // rogne a l'interieur. Un simple `setClipRect` remplacerait le
            // precedent, et le contenu d'un bloc deborderait sur le chrome —
            // d'ou l'intersection, et la pile qui permet de revenir en arriere.
            double x, y, l, h;
            if (PyArg_ParseTuple(element, "sdddd", &operation, &x, &y, &l, &h)) {
                QRectF zone(x, y, l, h);
                if (!pileRognage.isEmpty())
                    zone = zone.intersected(pileRognage.last());
                pileRognage.append(zone);
                p.setClipRect(zone);
            }
        } else if (!std::strcmp(operation, "declip")) {
            if (!pileRognage.isEmpty())
                pileRognage.removeLast();
            if (pileRognage.isEmpty())
                p.setClipping(false);
            else
                p.setClipRect(pileRognage.last());
        } else if (!std::strcmp(operation, "imagepart")) {
            // `object-fit: cover` ne montre qu'une portion de l'image. Sans le
            // rectangle source, Qt l'etirerait a la boite — ce qui est
            // exactement ce que la propriete sert a eviter.
            double x, y, l, h, sx, sy, sl, sh;
            int identifiant;
            if (PyArg_ParseTuple(element, "sddddidddd", &operation, &x, &y, &l, &h,
                                 &identifiant, &sx, &sy, &sl, &sh)) {
                if (const QImage *image = imageParIdentifiant(identifiant))
                    p.drawImage(QRectF(x, y, l, h), *image, QRectF(sx, sy, sl, sh));
            }
        } else if (!std::strcmp(operation, "image")) {
            // L'image a ete decodee une fois par `bo.image` ; ici on ne fait que
            // la poser. Redecoder a chaque trame couterait un decodage PNG
            // complet par image et par rafraichissement.
            double x, y, l, h;
            int identifiant;
            if (PyArg_ParseTuple(element, "sddddi", &operation, &x, &y, &l, &h,
                                 &identifiant)) {
                if (const QImage *image = imageParIdentifiant(identifiant))
                    p.drawImage(QRectF(x, y, l, h), *image);
            }
        }
        PyErr_Clear();
        Py_DECREF(tete);
    }

public:
    /// Peint une liste d'affichage dans une image et rend ses octets ARGB32.
    ///
    /// C'est ce qui donne enfin des pixels a `getImageData` : la toile
    /// n'enregistrait que des operations, et lire ses pixels demandait de les
    /// jouer pour de vrai. On reutilise le meme peintre que l'ecran, donc le
    /// resultat est celui qu'on voit.
    QImage rasterise(PyObject *liste, int largeur, int hauteur)
    {
        QImage image(largeur, hauteur, QImage::Format_ARGB32);
        image.fill(Qt::transparent);
        QPainter peintre(&image);
        peintre.setRenderHint(QPainter::Antialiasing, true);
        peintre.setRenderHint(QPainter::TextAntialiasing, true);
        peintListe(peintre, liste);
        return image;
    }

private:
    /// Pose les arrets de couleur d'un degrade, lineaire ou radial.
    void remplitEtapes(QGradient &pente, PyObject *etapes)
    {
        const Py_ssize_t nb = PySequence_Size(etapes);
        for (Py_ssize_t k = 0; k < nb; ++k) {
            PyObject *etape = PySequence_GetItem(etapes, k);
            if (!etape)
                break;
            double position = 0.0;
            long teinte = 0;
            if (PyArg_ParseTuple(etape, "dl", &position, &teinte))
                pente.setColorAt(position, couleurDepuisEntier(teinte));
            else
                PyErr_Clear();
            Py_DECREF(etape);
        }
    }

    /// Zones de rognage en cours, de la plus exterieure a la plus interieure.
    QVector<QRectF> pileRognage;
    /// Etats de peintre empiles par « transforme » et « opacite ».
    int enveloppes = 0;
};

// --- Module Python `bo` -----------------------------------------------------

PyObject *bo_enregistrer(PyObject *, PyObject *args)
{
    PyObject *rappels;
    if (!PyArg_ParseTuple(args, "O!", &PyDict_Type, &rappels))
        return nullptr;
    Py_XDECREF(g_rappels);
    Py_INCREF(rappels);
    g_rappels = rappels;
    Py_RETURN_NONE;
}

PyObject *bo_ouvrir(PyObject *, PyObject *args)
{
    const char *titre = "Bouchaud OS";
    if (!PyArg_ParseTuple(args, "|s", &titre))
        return nullptr;
    if (g_toile) {
        g_toile->setWindowTitle(QString::fromUtf8(titre));
        g_toile->showFullScreen();
        g_toile->setFocus();
    }
    // Battement de service : c'est par lui qu'un fil Python fait executer du
    // travail sur le fil principal. Qt n'accepte d'etre touche que depuis
    // celui-ci ; un `QFontMetrics` construit ailleurs suffit a faire tomber le
    // programme.
    static QTimer *battement = nullptr;
    if (!battement) {
        battement = new QTimer();
        QObject::connect(battement, &QTimer::timeout, [] {
            appelle("tic");
        });
        battement->start(16);
    }
    Py_RETURN_NONE;
}

PyObject *bo_boucle(PyObject *, PyObject *)
{
    int code = 0;
    // Le verrou est relache pendant toute la boucle : les rappels le reprennent
    // a l'entree, et un fil Python lance par le programme peut avancer.
    Py_BEGIN_ALLOW_THREADS
    code = QCoreApplication::exec();
    Py_END_ALLOW_THREADS
    return PyLong_FromLong(code);
}

PyObject *bo_largeur_texte(PyObject *, PyObject *args)
{
    const char *texte;
    double taille;
    int gras = 0, fixe = 0;
    const char *famille = "";
    if (!PyArg_ParseTuple(args, "sd|pps", &texte, &taille, &gras, &fixe, &famille))
        return nullptr;
    if (!g_toile)
        return PyFloat_FromDouble(0.0);
    return PyFloat_FromDouble(
        g_toile->largeurTexte(QString::fromUtf8(texte), taille, gras, fixe,
                              QString::fromUtf8(famille)));
}

/// `bo.police(octets, famille, gras=False, italique=False) -> bool`
///
/// Range une police livree par la page (`@font-face`) dans la base de Qt, d'ou
/// `QFont` la retrouvera par son nom. Le moteur a deja ouvert le conteneur
/// WOFF : ce qui arrive ici est du TrueType ou de l'OpenType, les deux seuls
/// formats que Qt lit.
PyObject *bo_police(PyObject *, PyObject *args)
{
    Py_buffer donnees;
    const char *famille = "";
    int gras = 0, italique = 0;
    if (!PyArg_ParseTuple(args, "y*s|pp", &donnees, &famille, &gras, &italique))
        return nullptr;
    const QByteArray octets(static_cast<const char *>(donnees.buf),
                            int(donnees.len));
    PyBuffer_Release(&donnees);
    const int identifiant = QFontDatabase::addApplicationFontFromData(octets);
    if (identifiant < 0)
        Py_RETURN_FALSE;
    Py_RETURN_TRUE;
}

PyObject *bo_hauteur_ligne(PyObject *, PyObject *args)
{
    double taille;
    int fixe = 0;
    if (!PyArg_ParseTuple(args, "d|p", &taille, &fixe))
        return nullptr;
    if (!g_toile)
        return PyFloat_FromDouble(taille * 1.2);
    return PyFloat_FromDouble(g_toile->hauteurLigne(taille, fixe));
}

PyObject *bo_redessiner(PyObject *, PyObject *)
{
    if (g_toile)
        g_toile->update();
    Py_RETURN_NONE;
}

PyObject *bo_titre(PyObject *, PyObject *args)
{
    const char *titre;
    if (!PyArg_ParseTuple(args, "s", &titre))
        return nullptr;
    if (g_toile)
        g_toile->setWindowTitle(QString::fromUtf8(titre));
    Py_RETURN_NONE;
}

PyObject *bo_taille(PyObject *, PyObject *)
{
    // Avant l'ouverture, la toile n'a pas encore sa taille : on rend celle de
    // l'ecran, qui est de toute facon ce qu'elle prendra (plein ecran).
    if (g_toile && g_toile->isVisible())
        return Py_BuildValue("(ii)", g_toile->width(), g_toile->height());
    if (QScreen *ecran = QGuiApplication::primaryScreen()) {
        const QRect g = ecran->geometry();
        return Py_BuildValue("(ii)", g.width(), g.height());
    }
    return Py_BuildValue("(ii)", 1280, 720);
}

PyObject *bo_quitter(PyObject *, PyObject *)
{
    QCoreApplication::quit();
    Py_RETURN_NONE;
}

PyObject *bo_traiter_evenements(PyObject *, PyObject *)
{
    // Laisse Qt respirer pendant un chargement : sans cela, l'ecran resterait
    // fige tant que Python telecharge et analyse une page.
    Py_BEGIN_ALLOW_THREADS
    QCoreApplication::processEvents(QEventLoop::ExcludeUserInputEvents);
    Py_END_ALLOW_THREADS
    Py_RETURN_NONE;
}

PyObject *bo_image(PyObject *, PyObject *args)
{
    Py_buffer donnees;
    if (!PyArg_ParseTuple(args, "y*", &donnees))
        return nullptr;

    QImage image;
    const bool lue = image.loadFromData(
        reinterpret_cast<const uchar *>(donnees.buf), int(donnees.len));
    PyBuffer_Release(&donnees);

    if (!lue || image.isNull())
        // Format inconnu ou donnees tronquees : l'appelant retombe sur le
        // texte de remplacement, comme le ferait n'importe quel navigateur.
        Py_RETURN_NONE;

    g_images.append(image);
    return Py_BuildValue("(iii)", g_images.size() - 1, image.width(), image.height());
}

PyObject *bo_image_brute(PyObject *, PyObject *args)
{
    Py_buffer donnees;
    int largeur, hauteur, identifiant = -1;
    if (!PyArg_ParseTuple(args, "y*ii|i", &donnees, &largeur, &hauteur, &identifiant))
        return nullptr;

    const qsizetype attendu = qsizetype(largeur) * qsizetype(hauteur) * 4;
    if (largeur <= 0 || hauteur <= 0 || donnees.len < attendu) {
        PyBuffer_Release(&donnees);
        PyErr_SetString(PyExc_ValueError, "bo.image_brute : tampon trop court");
        return nullptr;
    }

    // Copie : le tampon Python peut disparaitre avant le prochain rendu.
    QImage image(reinterpret_cast<const uchar *>(donnees.buf), largeur, hauteur,
                 largeur * 4, QImage::Format_ARGB32);
    QImage copie = image.copy();
    PyBuffer_Release(&donnees);

    // Reutiliser l'emplacement est ce qui rend la video tenable : une trame par
    // seizieme de seconde ferait grossir le cache de 1500 images en une minute.
    if (identifiant >= 0 && identifiant < g_images.size()) {
        g_images[identifiant] = copie;
        return Py_BuildValue("(iii)", identifiant, largeur, hauteur);
    }
    g_images.append(copie);
    return Py_BuildValue("(iii)", g_images.size() - 1, largeur, hauteur);
}

PyObject *bo_formats_images(PyObject *, PyObject *)
{
    PyObject *liste = PyList_New(0);
    if (!liste)
        return nullptr;
    for (const QByteArray &format : QImageReader::supportedImageFormats()) {
        PyObject *nom = PyUnicode_FromString(format.constData());
        if (nom) {
            PyList_Append(liste, nom);
            Py_DECREF(nom);
        }
    }
    return liste;
}

/// `bo.rasterise(operations, largeur, hauteur) -> octets ARGB32`
///
/// Joue une liste d'affichage dans une image hors ecran et rend ses pixels.
/// `getImageData` en depend : la toile n'enregistre que des operations, et il
/// faut les peindre pour de vrai avant d'en lire un seul pixel.
PyObject *bo_rasterise(PyObject *, PyObject *args)
{
    PyObject *liste = nullptr;
    int largeur = 0, hauteur = 0;
    if (!PyArg_ParseTuple(args, "Oii", &liste, &largeur, &hauteur))
        return nullptr;
    if (!g_toile || largeur <= 0 || hauteur <= 0 || largeur > 8192 || hauteur > 8192) {
        PyErr_SetString(PyExc_ValueError, "bo.rasterise : taille hors bornes");
        return nullptr;
    }
    const QImage image = g_toile->rasterise(liste, largeur, hauteur);
    if (image.isNull())
        Py_RETURN_NONE;
    return PyBytes_FromStringAndSize(
        reinterpret_cast<const char *>(image.constBits()),
        Py_ssize_t(image.sizeInBytes()));
}

PyMethodDef bo_methodes[] = {
    {"enregistrer", bo_enregistrer, METH_VARARGS,
     "enregistrer({'peindre': f, 'touche': f, 'clic': f, ...})"},
    {"ouvrir", bo_ouvrir, METH_VARARGS, "ouvrir(titre) : affiche la fenetre"},
    {"boucle", bo_boucle, METH_NOARGS, "entre dans la boucle d'evenements Qt"},
    {"largeur_texte", bo_largeur_texte, METH_VARARGS,
     "largeur_texte(texte, taille, gras=False, fixe=False) -> pixels"},
    {"hauteur_ligne", bo_hauteur_ligne, METH_VARARGS,
     "hauteur_ligne(taille, fixe=False) -> pixels"},
    {"redessiner", bo_redessiner, METH_NOARGS, "demande un rafraichissement"},
    {"titre", bo_titre, METH_VARARGS, "titre(texte) : titre de la fenetre"},
    {"taille", bo_taille, METH_NOARGS, "taille() -> (largeur, hauteur)"},
    {"quitter", bo_quitter, METH_NOARGS, "termine la boucle d'evenements"},
    {"traiter_evenements", bo_traiter_evenements, METH_NOARGS,
     "laisse Qt traiter sa file pendant un travail long"},
    {"image", bo_image, METH_VARARGS,
     "image(octets) -> (identifiant, largeur, hauteur), ou None si illisible"},
    {"image_brute", bo_image_brute, METH_VARARGS,
     "image_brute(octets_bgra, largeur, hauteur, identifiant=-1) -> (id, l, h)"},
    {"police", bo_police, METH_VARARGS,
     "police(octets, famille, gras=False, italique=False) -> bool"},
    {"rasterise", bo_rasterise, METH_VARARGS,
     "rasterise(operations, largeur, hauteur) -> octets ARGB32"},
    {"formats_images", bo_formats_images, METH_NOARGS,
     "formats_images() -> formats que l'hote sait decoder"},
    {nullptr, nullptr, 0, nullptr},
};

PyModuleDef bo_module = {
    PyModuleDef_HEAD_INIT, "bo",
    "Pont minimal entre Python et Qt sur Bouchaud OS.",
    -1, bo_methodes, nullptr, nullptr, nullptr, nullptr,
};

PyObject *initialise_bo()
{
    return PyModule_Create(&bo_module);
}

/// Ajoute un chemin a `sys.path` de la configuration.
void ajoute_chemin(PyConfig *config, const char *format, const char *prefixe)
{
    char chemin[512];
    std::snprintf(chemin, sizeof chemin, format, prefixe);
    PyWideStringList_Append(&config->module_search_paths,
                            Py_DecodeLocale(chemin, nullptr));
}

} // namespace

int main(int argc, char **argv)
{
    QApplication app(argc, argv);

    // Les modules doivent exister avant l'initialisation de l'interprete : un
    // executable statique ne peut pas charger d'extension apres coup.
    PyImport_AppendInittab("bo", initialise_bo);
    PyImport_AppendInittab("bojs", initialise_bojs);
    PyImport_AppendInittab("bomedia", initialise_bomedia);

    const char *prefixe = std::getenv("BO_PREFIXE");
    if (!prefixe)
        prefixe = "/usr";

    // Sans argument, ou avec une simple adresse, on lance le navigateur natif.
    const char *script = nullptr;
    static char defaut[512];
    if (argc > 1 && std::strlen(argv[1]) > 3
        && !std::strcmp(argv[1] + std::strlen(argv[1]) - 3, ".py")) {
        script = argv[1];
    } else {
        std::snprintf(defaut, sizeof defaut, "%s/share/bo-navigateur/navigateur.py",
                      prefixe);
        script = defaut;
    }

    // UTF-8 partout, avant toute autre initialisation. Sans cela l'interprete
    // deduit son encodage d'une locale absente, retombe sur ASCII, et le
    // premier caractere accentue imprime par un programme leve une
    // `UnicodeEncodeError` — sur un OS francais, autant dire tout de suite.
    {
        PyPreConfig preconfig;
        PyPreConfig_InitIsolatedConfig(&preconfig);
        preconfig.utf8_mode = 1;
        PyStatus prestatut = Py_PreInitialize(&preconfig);
        if (PyStatus_Exception(prestatut)) {
            std::fprintf(stderr, "[bo] pre-initialisation de Python impossible\n");
            return 2;
        }
    }

    PyConfig config;
    PyConfig_InitIsolatedConfig(&config);
    config.parse_argv = 0;
    config.install_signal_handlers = 1;
    // `isolated` coupe les variables d'environnement, mais les programmes ont
    // besoin de trouver leur bibliotheque standard et leurs propres modules.
    config.use_environment = 1;
    config.module_search_paths_set = 1;
    ajoute_chemin(&config, "%s/lib/python312.zip", prefixe);
    ajoute_chemin(&config, "%s/share/bo-navigateur", prefixe);
    ajoute_chemin(&config, "%s/lib/python3/site-packages", prefixe);

    // `sys.argv` tel que le verrait un `python script.py args...` : le script en
    // premier, puis ce qui suit. Un programme pywebview y lit ses options comme
    // sur n'importe quel systeme.
    {
        wchar_t *args[64];
        int n = 0;
        args[n++] = Py_DecodeLocale(script, nullptr);
        for (int i = (script == defaut ? 1 : 2); i < argc && n < 63; ++i)
            args[n++] = Py_DecodeLocale(argv[i], nullptr);
        PyConfig_SetArgv(&config, n, args);
    }

    PyStatus statut = Py_InitializeFromConfig(&config);
    PyConfig_Clear(&config);
    if (PyStatus_Exception(statut)) {
        std::fprintf(stderr, "[bo] initialisation de Python impossible\n");
        return 2;
    }

    Toile toile;
    g_toile = &toile;

    if (QScreen *ecran = QGuiApplication::primaryScreen()) {
        std::printf("[bo] Qt %s, plateforme %s, ecran %dx%d\n", QT_VERSION_STR,
                    QGuiApplication::platformName().toUtf8().constData(),
                    ecran->geometry().width(), ecran->geometry().height());
    }
    std::printf("[bo] Python %s embarque\n", Py_GetVersion());
    std::printf("[bo] script : %s\n", script);
    std::fflush(stdout);

    FILE *fichier = std::fopen(script, "r");
    if (!fichier) {
        std::fprintf(stderr, "[bo] script introuvable : %s\n", script);
        return 3;
    }
    // Le script mene la danse : il enregistre ses rappels, ouvre la fenetre et
    // entre dans la boucle. L'hote ne decide de rien.
    const int code = PyRun_SimpleFile(fichier, script);
    std::fclose(fichier);

    Py_FinalizeEx();
    return code;
}
