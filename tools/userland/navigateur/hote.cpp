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

#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>

Q_IMPORT_PLUGIN(QLinuxFbIntegrationPlugin)

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

    // Le module doit exister avant l'initialisation de l'interprete : un
    // executable statique ne peut pas charger d'extension apres coup.
    PyImport_AppendInittab("bo", initialise_bo);

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
