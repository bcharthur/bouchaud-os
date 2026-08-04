// Hote du navigateur de Bouchaud OS : Qt d'un cote, CPython de l'autre.
//
// Un seul processus, un seul binaire statique. Qt tient la fenetre, le
// framebuffer et les entrees ; Python tient le moteur web et l'interface. Entre
// les deux, le module `bo` defini ici — et rien d'autre.
//
// ## Pourquoi pas PyQt
//
// PyQt expose les 200 000 lignes d'API de Qt a Python. Un navigateur n'en
// utilise qu'une poignee : ouvrir une fenetre, peindre des rectangles et du
// texte, mesurer ce texte, recevoir des touches et des clics. Le pont ci-dessous
// fait exactement cela en quelques centaines de lignes, se construit en dix
// secondes, et n'a pas besoin d'un PyQt statique — chose qui n'existe pas
// vraiment. Le prix a payer est de ne pas pouvoir ecrire du Qt arbitraire en
// Python ; pour un navigateur, ce n'est pas un prix.
//
// ## Sens du flux
//
// Qt appelle Python, jamais l'inverse pendant la peinture :
//
//   paintEvent  -> bo._peindre(largeur, hauteur) -> liste d'affichage -> QPainter
//   keyPressEvent -> bo._touche(code, texte)
//   mousePress  -> bo._clic(x, y)
//   wheelEvent  -> bo._molette(delta)
//   timer       -> bo._tic()
//
// Une liste d'affichage est une liste de tuples. Le moteur la produit, l'hote
// la peint. Cette frontiere est ce qui permet d'ecrire tout le moteur en Python
// sans jamais toucher a Qt.

// Python.h vient en premier, et ce n'est pas une preference de style : Qt
// definit `slots` comme mot-cle, et `PyType_Spec` a un champ qui porte ce nom.
// L'inclure apres Qt transforme la declaration en erreur de syntaxe.
#define PY_SSIZE_T_CLEAN
#include <Python.h>

#include <QtWidgets/QApplication>
#include <QtWidgets/QWidget>
#include <QtGui/QPainter>
#include <QtGui/QFontMetricsF>
#include <QtGui/QGuiApplication>
#include <QtGui/QScreen>
#include <QtGui/QKeyEvent>
#include <QtGui/QMouseEvent>
#include <QtGui/QWheelEvent>
#include <QtCore/QTimer>
#include <QtCore/QtPlugin>

#include <cstdio>
#include <cstdlib>
#include <cstring>

Q_IMPORT_PLUGIN(QLinuxFbIntegrationPlugin)

namespace {

class Toile;
Toile *g_toile = nullptr;

// --- Outils de conversion ---------------------------------------------------

QColor couleurDepuisEntier(long valeur)
{
    // 0xAARRGGBB ; un alpha nul est interprete comme opaque, pour que
    // 0x336699 s'ecrive sans avoir a penser a l'alpha.
    const int a = (valeur >> 24) & 0xFF;
    return QColor((valeur >> 16) & 0xFF, (valeur >> 8) & 0xFF, valeur & 0xFF,
                  a == 0 ? 255 : a);
}

QFont fabriqueFonte(double taille, bool gras, bool italique, bool fixe)
{
    QFont f;
    if (fixe)
        f.setFamily(QStringLiteral("DejaVu Sans Mono"));
    f.setPixelSize(qMax(1, int(taille + 0.5)));
    f.setBold(gras);
    f.setItalic(italique);
    return f;
}

/// Appelle une fonction du module Python du navigateur. Renvoie la reference,
/// ou nullptr en signalant l'exception sur la sortie d'erreur.
PyObject *appellePython(const char *nom, PyObject *arguments)
{
    PyObject *module = PyImport_ImportModule("navigateur");
    if (!module) {
        PyErr_Print();
        Py_XDECREF(arguments);
        return nullptr;
    }
    PyObject *fonction = PyObject_GetAttrString(module, nom);
    Py_DECREF(module);
    if (!fonction) {
        PyErr_Clear();
        Py_XDECREF(arguments);
        return nullptr;
    }
    PyObject *resultat = PyObject_CallObject(fonction, arguments);
    Py_DECREF(fonction);
    Py_XDECREF(arguments);
    if (!resultat)
        PyErr_Print();
    return resultat;
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

    /// Mesure un texte avec la fonte demandee. Le moteur en a besoin pour la
    /// mise en page — c'est la seule chose que Python ne peut pas calculer seul.
    double largeurTexte(const QString &texte, double taille, bool gras, bool fixe)
    {
        QFontMetricsF metriques(fabriqueFonte(taille, gras, false, fixe));
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

        PyObject *liste = appellePython("_peindre",
                                        Py_BuildValue("(ii)", width(), height()));
        if (!liste)
            return;
        peintListe(p, liste);
        Py_DECREF(liste);
    }

    void keyPressEvent(QKeyEvent *e) override
    {
        PyObject *r = appellePython(
            "_touche", Py_BuildValue("(is)", e->key(), e->text().toUtf8().constData()));
        Py_XDECREF(r);
        update();
    }

    void mousePressEvent(QMouseEvent *e) override
    {
        PyObject *r = appellePython("_clic", Py_BuildValue("(ii)", e->x(), e->y()));
        Py_XDECREF(r);
        update();
    }

    void mouseMoveEvent(QMouseEvent *e) override
    {
        PyObject *r = appellePython("_survol", Py_BuildValue("(ii)", e->x(), e->y()));
        Py_XDECREF(r);
    }

    void wheelEvent(QWheelEvent *e) override
    {
        const int cran = e->angleDelta().y();
        PyObject *r = appellePython("_molette", Py_BuildValue("(i)", cran));
        Py_XDECREF(r);
        update();
    }

private:
    /// Peint une liste d'affichage.
    ///
    /// Chaque element est un tuple dont le premier champ nomme l'operation. Le
    /// format est volontairement plat et sans objets : c'est ce qui permet au
    /// moteur de la construire sans jamais dialoguer avec Qt.
    void peintListe(QPainter &p, PyObject *liste)
    {
        const Py_ssize_t n = PySequence_Size(liste);
        if (n < 0) {
            PyErr_Clear();
            return;
        }
        for (Py_ssize_t i = 0; i < n; ++i) {
            PyObject *element = PySequence_GetItem(liste, i);
            if (!element)
                break;
            peintElement(p, element);
            Py_DECREF(element);
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
        } else if (!std::strcmp(operation, "texte")) {
            double x, y, taille;
            long couleur;
            int gras, italique, fixe, souligne;
            const char *texte;
            if (PyArg_ParseTuple(element, "sddsldpppp", &operation, &x, &y, &texte,
                                 &couleur, &taille, &gras, &italique, &fixe, &souligne)) {
                QFont f = fabriqueFonte(taille, gras, italique, fixe);
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
            double x, y, l, h;
            if (PyArg_ParseTuple(element, "sdddd", &operation, &x, &y, &l, &h))
                p.setClipRect(QRectF(x, y, l, h));
        } else if (!std::strcmp(operation, "declip")) {
            p.setClipping(false);
        } else if (!std::strcmp(operation, "image")) {
            // Donnees brutes d'un format que Qt sait lire (PNG, BMP, XPM...).
            double x, y, l, h;
            Py_buffer donnees;
            if (PyArg_ParseTuple(element, "sdddds*", &operation, &x, &y, &l, &h, &donnees)) {
                QImage image;
                if (image.loadFromData(reinterpret_cast<const uchar *>(donnees.buf),
                                       int(donnees.len))) {
                    p.drawImage(QRectF(x, y, l, h), image);
                }
                PyBuffer_Release(&donnees);
            }
        }
        PyErr_Clear();
        Py_DECREF(tete);
    }
};

// --- Module Python `bo` -----------------------------------------------------

PyObject *bo_largeur_texte(PyObject *, PyObject *args)
{
    const char *texte;
    double taille;
    int gras = 0, fixe = 0;
    if (!PyArg_ParseTuple(args, "sd|pp", &texte, &taille, &gras, &fixe))
        return nullptr;
    if (!g_toile)
        return PyFloat_FromDouble(0.0);
    return PyFloat_FromDouble(
        g_toile->largeurTexte(QString::fromUtf8(texte), taille, gras, fixe));
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

PyObject *bo_quitter(PyObject *, PyObject *)
{
    QCoreApplication::quit();
    Py_RETURN_NONE;
}

PyObject *bo_traiter_evenements(PyObject *, PyObject *)
{
    // Laisse Qt respirer pendant un chargement : sans cela, l'ecran resterait
    // fige tant que Python telecharge et analyse une page.
    QCoreApplication::processEvents(QEventLoop::ExcludeUserInputEvents);
    Py_RETURN_NONE;
}

PyMethodDef bo_methodes[] = {
    {"largeur_texte", bo_largeur_texte, METH_VARARGS,
     "largeur_texte(texte, taille, gras=False, fixe=False) -> pixels"},
    {"hauteur_ligne", bo_hauteur_ligne, METH_VARARGS,
     "hauteur_ligne(taille, fixe=False) -> pixels"},
    {"redessiner", bo_redessiner, METH_NOARGS, "demande un rafraichissement"},
    {"titre", bo_titre, METH_VARARGS, "titre(texte) : titre de la fenetre"},
    {"quitter", bo_quitter, METH_NOARGS, "termine le navigateur"},
    {"traiter_evenements", bo_traiter_evenements, METH_NOARGS,
     "laisse Qt traiter sa file pendant un travail long"},
    {nullptr, nullptr, 0, nullptr},
};

PyModuleDef bo_module = {
    PyModuleDef_HEAD_INIT, "bo",
    "Pont minimal entre le moteur web de Bouchaud OS et Qt.",
    -1, bo_methodes, nullptr, nullptr, nullptr, nullptr,
};

PyObject *initialise_bo()
{
    return PyModule_Create(&bo_module);
}

} // namespace

int main(int argc, char **argv)
{
    QApplication app(argc, argv);

    // Le module doit exister avant l'initialisation de l'interprete : un
    // executable statique ne peut pas charger d'extension apres coup.
    PyImport_AppendInittab("bo", initialise_bo);

    PyStatus statut;
    PyConfig config;
    PyConfig_InitIsolatedConfig(&config);
    config.parse_argv = 0;
    config.install_signal_handlers = 1;
    // `isolated` coupe les variables d'environnement, mais le navigateur a
    // besoin de trouver sa bibliotheque standard et ses propres modules.
    config.use_environment = 1;
    config.module_search_paths_set = 1;

    const char *prefixe = std::getenv("BO_PREFIXE");
    if (!prefixe)
        prefixe = "/usr";
    char chemin[512];
    std::snprintf(chemin, sizeof chemin, "%s/lib/python312.zip", prefixe);
    PyWideStringList_Append(&config.module_search_paths, Py_DecodeLocale(chemin, nullptr));
    std::snprintf(chemin, sizeof chemin, "%s/share/bo-navigateur", prefixe);
    PyWideStringList_Append(&config.module_search_paths, Py_DecodeLocale(chemin, nullptr));

    statut = Py_InitializeFromConfig(&config);
    PyConfig_Clear(&config);
    if (PyStatus_Exception(statut)) {
        std::fprintf(stderr, "[navigateur] initialisation de Python impossible\n");
        return 2;
    }

    Toile toile;
    g_toile = &toile;
    toile.setWindowTitle(QStringLiteral("Navigateur — Bouchaud OS"));
    toile.showFullScreen();

    if (QScreen *ecran = QGuiApplication::primaryScreen()) {
        std::printf("[navigateur] Qt %s, plateforme %s, ecran %dx%d\n", QT_VERSION_STR,
                    QGuiApplication::platformName().toUtf8().constData(),
                    ecran->geometry().width(), ecran->geometry().height());
    }
    std::printf("[navigateur] Python %s embarque\n", Py_GetVersion());
    std::fflush(stdout);

    // Demarrage du cote Python : c'est lui qui decide de la page initiale.
    PyObject *depart = appellePython(
        "_demarrer", Py_BuildValue("(s)", argc > 1 ? argv[1] : ""));
    if (!depart) {
        std::fprintf(stderr, "[navigateur] le module Python n'a pas demarre\n");
        return 3;
    }
    Py_DECREF(depart);

    // Battement : laisse le cote Python animer ou finir un chargement.
    QTimer battement;
    QObject::connect(&battement, &QTimer::timeout, [] {
        PyObject *r = appellePython("_tic", PyTuple_New(0));
        Py_XDECREF(r);
    });
    battement.start(100);

    const int code = app.exec();
    Py_FinalizeEx();
    return code;
}
