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
#include <QtCore/QFile>
#include <QtCore/QSet>
#include <QtCore/QStringList>
#include <QtCore/QtPlugin>

#include <brotli/decode.h>

#include <linux/fb.h>
#include <fcntl.h>
#include <poll.h>
#include <sys/time.h>
#include <dirent.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#include <algorithm>
#include <cmath>
#include <cstdarg>
#include <cstdint>
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

/// Destination de pixels de l'hote.
///
/// Etape actuelle : `/dev/fb0`, mappe une seconde fois a cote de linuxfb. Le
/// widget Qt ne depend alors plus du backing-store pour rendre les pixels.
/// Etape suivante : le WM passera un memfd herite dans `BO_SURFACE_FD`; le meme
/// code peindra cette surface partagee sans toucher au framebuffer physique.
class SortieBouchaud
{
public:
    ~SortieBouchaud() { ferme(); }

    bool ouvre()
    {
        if (pixels_)
            return true;
        if (ouvreSurfacePartagee())
            return true;
        return ouvreFramebuffer();
    }

    bool active() const { return pixels_ != nullptr; }
    bool framebuffer() const { return mode_ == Mode::Framebuffer; }
    const char *nom() const
    {
        if (mode_ == Mode::SurfacePartagee) return "surface-partagee";
        if (mode_ == Mode::Framebuffer) return "/dev/fb0";
        return "QPA";
    }

    void presente(const QImage &source)
    {
        if (!pixels_ || source.isNull())
            return;
        const int lignes = std::min(hauteur_, source.height());
        const int octets = std::min(pas_, source.width() * 4);
        for (int y = 0; y < lignes; ++y)
            std::memcpy(pixels_ + size_t(y) * size_t(pas_), source.constScanLine(y), size_t(octets));
    }

    void remplit(uint32_t rgb)
    {
        if (!pixels_)
            return;
        for (int y = 0; y < hauteur_; ++y) {
            uint32_t *ligne = reinterpret_cast<uint32_t *>(pixels_ + size_t(y) * size_t(pas_));
            for (int x = 0; x < largeur_; ++x)
                ligne[x] = rgb;
        }
    }

private:
    enum class Mode { Aucun, SurfacePartagee, Framebuffer };

    static int entierEnv(const char *nom, int defaut = -1)
    {
        const char *texte = std::getenv(nom);
        if (!texte || !*texte)
            return defaut;
        char *fin = nullptr;
        const long valeur = std::strtol(texte, &fin, 10);
        return fin && *fin == '\0' && valeur >= 0 && valeur <= 0x7fffffff
            ? int(valeur) : defaut;
    }

    bool mappe(int fd, int largeur, int hauteur, int pas, Mode mode)
    {
        if (fd < 0 || largeur <= 0 || hauteur <= 0 || pas < largeur * 4)
            return false;
        const size_t taille = size_t(pas) * size_t(hauteur);
        void *carte = mmap(nullptr, taille, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        if (carte == MAP_FAILED)
            return false;
        fd_ = fd;
        pixels_ = static_cast<unsigned char *>(carte);
        taille_ = taille;
        largeur_ = largeur;
        hauteur_ = hauteur;
        pas_ = pas;
        mode_ = mode;
        return true;
    }

    bool ouvreSurfacePartagee()
    {
        const int fd = entierEnv("BO_SURFACE_FD");
        if (fd < 0)
            return false;
        const int largeur = entierEnv("BO_SURFACE_WIDTH");
        const int hauteur = entierEnv("BO_SURFACE_HEIGHT");
        const int pas = entierEnv("BO_SURFACE_STRIDE", largeur > 0 ? largeur * 4 : -1);
        if (!mappe(fd, largeur, hauteur, pas, Mode::SurfacePartagee)) {
            std::fprintf(stderr, "[bo] BO_SURFACE_FD present mais surface invalide\n");
            return false;
        }
        std::printf("[bo] sortie graphique : surface partagee fd=%d %dx%d stride=%d\n",
                    fd_, largeur_, hauteur_, pas_);
        return true;
    }

    bool ouvreFramebuffer()
    {
        const int fd = open("/dev/fb0", O_RDWR);
        if (fd < 0)
            return false;
        fb_var_screeninfo var{};
        fb_fix_screeninfo fix{};
        if (ioctl(fd, FBIOGET_VSCREENINFO, &var) != 0
            || ioctl(fd, FBIOGET_FSCREENINFO, &fix) != 0
            || var.bits_per_pixel != 32
            || var.red.offset != 16 || var.green.offset != 8 || var.blue.offset != 0
            || !mappe(fd, int(var.xres), int(var.yres), int(fix.line_length), Mode::Framebuffer)) {
            close(fd);
            return false;
        }
        std::printf("[bo] sortie graphique : /dev/fb0 %dx%d stride=%d\n",
                    largeur_, hauteur_, pas_);
        return true;
    }

    void ferme()
    {
        if (pixels_)
            munmap(pixels_, taille_);
        if (fd_ >= 0)
            close(fd_);
        fd_ = -1;
        pixels_ = nullptr;
        taille_ = 0;
        largeur_ = hauteur_ = pas_ = 0;
        mode_ = Mode::Aucun;
    }

    int fd_ = -1;
    unsigned char *pixels_ = nullptr;
    size_t taille_ = 0;
    int largeur_ = 0;
    int hauteur_ = 0;
    int pas_ = 0;
    Mode mode_ = Mode::Aucun;
};

// --- Pont avec le gestionnaire de fenetres ----------------------------------
//
// Le navigateur n'est plus un programme qui prend l'ecran : c'est une fenetre du
// bureau de Bouchaud OS. Deux consequences, et ce sont les deux seules choses
// que ce pont doit gerer.
//
// 1. **Les entrees ne viennent plus d'evdev.** Le noyau refuse `/dev/input/*` a
//    un processus qui a un ecran virtuel — sinon le bureau et Qt se
//    partageraient la meme file de scancodes, chacun en volant la moitie a
//    l'autre. Les touches, les clics et la molette arrivent donc par ce canal,
//    deja convertis dans le repere de la fenetre.
//
// 2. **Le compositeur doit savoir quand une trame est prete.** Sans ce signal il
//    ne peut que recomposer a intervalle fixe, c'est-a-dire recopier 2,6 Mio
//    pour rien la plupart du temps.
//
// Le canal est lu depuis le battement de service, pas depuis un
// `QSocketNotifier` : le battement existe deja, il tourne toutes les 16 ms, et
// il ne depend pas de la facon dont le distributeur d'evenements de Qt surveille
// les descripteurs. Une dependance de moins sur un chemin que l'on ne peut
// tester que sur la machine cible.

namespace protocole {

constexpr uint32_t MAGIC = 0x55474F42; // "BOGU" en petit-boutiste
constexpr uint16_t VERSION = 1;
constexpr size_t TAILLE_ENTETE = 16;
constexpr uint32_t FENETRE = 1;
constexpr uint32_t CHARGE_MAX = 4096;

enum Genre : uint16_t {
    Hello = 1,
    CreateWindow = 2,
    SetTitle = 3,
    Damage = 4,
    Close = 5,
    FrameReady = 6,
    Surface = 0x100,
    Configure = 0x101,
    Focus = 0x102,
    Key = 0x103,
    Pointer = 0x104,
    Wheel = 0x105,
    CloseRequest = 0x106,
};

/// Bits du champ `modificateurs` d'un message `Key`. Definis cote noyau dans
/// `window_manager::modificateur` ; `tools/verifie-protocole-gui.py` refuse un
/// desaccord entre les trois implementations.
enum Modificateur : uint32_t {
    ModShift = 1,
    ModCtrl = 2,
    ModAlt = 4,
    ModAltGr = 8,
};

/// Codes de touche du protocole. Ce ne sont pas des codes evdev : le bureau
/// envoie une touche deja interpretee selon sa disposition clavier.
enum CodeTouche : uint32_t {
    ToucheCaractere = 0,
    ToucheEntree = 1,
    ToucheRetour = 2,
    ToucheTabulation = 3,
    ToucheHaut = 4,
    ToucheBas = 5,
    ToucheGauche = 6,
    ToucheDroite = 7,
    ToucheEchap = 8,
};

inline uint32_t litU32(const unsigned char *o, size_t d)
{
    return uint32_t(o[d]) | (uint32_t(o[d + 1]) << 8) | (uint32_t(o[d + 2]) << 16)
        | (uint32_t(o[d + 3]) << 24);
}
inline int32_t litI32(const unsigned char *o, size_t d) { return int32_t(litU32(o, d)); }

inline void poseU32(unsigned char *o, size_t d, uint32_t v)
{
    o[d] = v & 0xFF;
    o[d + 1] = (v >> 8) & 0xFF;
    o[d + 2] = (v >> 16) & 0xFF;
    o[d + 3] = (v >> 24) & 0xFF;
}

} // namespace protocole

class PontFenetre
{
public:
    bool ouvre()
    {
        const char *texte = std::getenv("BO_GUI_FD");
        if (!texte || !*texte)
            return false;
        char *fin = nullptr;
        const long valeur = std::strtol(texte, &fin, 10);
        if (!fin || *fin != '\0' || valeur < 0)
            return false;
        fd_ = int(valeur);
        // Non bloquant, par les deux chemins que le noyau connait. Ce n'est pas
        // de la superstition : une lecture bloquante sur ce descripteur attend
        // jusqu'a deux secondes, et le battement de service passe ici toutes les
        // 16 ms. Un seul de ces appels qui ne prend pas, et la boucle
        // d'evenements de Qt avance cent fois moins vite — page qui ne charge
        // pas, minuteries qui n'echoient pas, clics qui n'arrivent jamais. Le
        // `poll` de `pompe` est la troisieme ceinture.
        const int drapeaux = fcntl(fd_, F_GETFL, 0);
        if (drapeaux >= 0 && fcntl(fd_, F_SETFL, drapeaux | O_NONBLOCK) == 0)
            nonBloquant_ = true;
        int un = 1;
        if (ioctl(fd_, FIONBIO, &un) == 0)
            nonBloquant_ = true;
        std::printf("[bo] pont gestionnaire de fenetres : fd=%d (non bloquant : %s)\n",
                    fd_, nonBloquant_ ? "oui" : "NON");
        return true;
    }

    bool actif() const { return fd_ >= 0; }

    /// Dit bonjour et annonce la fenetre. Le bureau connait deja la geometrie —
    /// c'est lui qui l'a choisie — mais un client qui se presente est un client
    /// dont on sait qu'il parle le protocole, et non un programme qui peint dans
    /// la surface en ignorant tout le reste.
    void salue(int largeur, int hauteur)
    {
        if (!actif())
            return;
        unsigned char bonjour[8];
        protocole::poseU32(bonjour, 0, protocole::VERSION);
        protocole::poseU32(bonjour, 4, uint32_t(getpid()));
        envoie(protocole::Hello, bonjour, sizeof(bonjour));

        unsigned char fenetre[16];
        protocole::poseU32(fenetre, 0, protocole::FENETRE);
        protocole::poseU32(fenetre, 4, uint32_t(largeur));
        protocole::poseU32(fenetre, 8, uint32_t(hauteur));
        protocole::poseU32(fenetre, 12, 0);
        envoie(protocole::CreateWindow, fenetre, sizeof(fenetre));
    }

    /// Annonce une trame prete. C'est le seul message qui autorise le
    /// compositeur a lire la surface.
    void trameLivree(int x, int y, int largeur, int hauteur)
    {
        if (!actif() || largeur <= 0 || hauteur <= 0)
            return;
        unsigned char charge[24];
        protocole::poseU32(charge, 0, protocole::FENETRE);
        protocole::poseU32(charge, 4, 0); // tampon 0 : simple tampon pour l'instant
        protocole::poseU32(charge, 8, uint32_t(x));
        protocole::poseU32(charge, 12, uint32_t(y));
        protocole::poseU32(charge, 16, uint32_t(largeur));
        protocole::poseU32(charge, 20, uint32_t(hauteur));
        envoie(protocole::FrameReady, charge, sizeof(charge));
        ++trames_;
    }

    void titre(const QByteArray &texte)
    {
        if (!actif())
            return;
        const QByteArray borne = texte.left(96);
        envoie(protocole::SetTitle,
               reinterpret_cast<const unsigned char *>(borne.constData()),
               size_t(borne.size()));
    }

    /// Lit tout ce que le bureau a envoye et le distribue. Appelee par le
    /// battement de service.
    void pompe();

private:
    void envoie(uint16_t genre, const unsigned char *charge, size_t taille)
    {
        unsigned char entete[protocole::TAILLE_ENTETE];
        protocole::poseU32(entete, 0, protocole::MAGIC);
        entete[4] = protocole::VERSION & 0xFF;
        entete[5] = (protocole::VERSION >> 8) & 0xFF;
        entete[6] = genre & 0xFF;
        entete[7] = (genre >> 8) & 0xFF;
        protocole::poseU32(entete, 8, uint32_t(taille));
        protocole::poseU32(entete, 12, ++serie_);

        // Un message est ecrit d'un bloc, en-tete et charge ensemble : deux
        // ecritures pourraient etre separees par un `EAGAIN` et laisser le
        // bureau devant un en-tete sans corps. Si le canal est plein, on
        // abandonne le message entier — jamais sa moitie.
        unsigned char tampon[protocole::TAILLE_ENTETE + protocole::CHARGE_MAX];
        if (taille > protocole::CHARGE_MAX)
            return;
        std::memcpy(tampon, entete, sizeof(entete));
        if (taille)
            std::memcpy(tampon + sizeof(entete), charge, taille);
        const ssize_t ecrit = ::write(fd_, tampon, sizeof(entete) + taille);
        (void)ecrit;
    }

    void traite(uint16_t genre, const unsigned char *charge, size_t taille);

    int fd_ = -1;
    uint32_t serie_ = 0;
    /// Reception partielle : un message peut arriver en deux morceaux.
    QByteArray tampon_;
    /// Etat precedent des boutons, pour distinguer un survol d'un clic.
    uint32_t boutons_ = 0;
    bool nonBloquant_ = false;

public:
    /// Compteurs du battement, pour le journal.
    unsigned long tics_ = 0;
    unsigned long trames_ = 0;
    unsigned long evenements_ = 0;
};

PontFenetre g_pont;

/// Backbuffer raster alloue par `mmap` anonyme plutot que par QImage.
///
/// Si l'ancien avertissement venait d'une allocation du backing-store Qt, ce
/// chemin l'evite. QImage ne fait qu'envelopper des pages deja allouees.
class TrameRaster
{
public:
    ~TrameRaster() { reset(); }

    bool assure(int largeur, int hauteur)
    {
        if (largeur <= 0 || hauteur <= 0)
            return false;
        if (pixels_ && largeur == largeur_ && hauteur == hauteur_)
            return true;
        reset();
        largeur_ = largeur;
        hauteur_ = hauteur;
        pas_ = largeur * 4;
        taille_ = size_t(pas_) * size_t(hauteur_);
        void *carte = mmap(nullptr, taille_, PROT_READ | PROT_WRITE,
                           MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (carte == MAP_FAILED) {
            pixels_ = nullptr;
            taille_ = 0;
            return false;
        }
        pixels_ = static_cast<unsigned char *>(carte);
        image_ = QImage(pixels_, largeur_, hauteur_, pas_, QImage::Format_RGB32);
        if (image_.isNull()) {
            reset();
            return false;
        }
        return true;
    }

    QImage &image() { return image_; }

private:
    void reset()
    {
        image_ = QImage();
        if (pixels_)
            munmap(pixels_, taille_);
        pixels_ = nullptr;
        taille_ = 0;
        largeur_ = hauteur_ = pas_ = 0;
    }

    unsigned char *pixels_ = nullptr;
    size_t taille_ = 0;
    int largeur_ = 0;
    int hauteur_ = 0;
    int pas_ = 0;
    QImage image_;
};

/// La famille de repli, celle dont on sait qu'elle couvre le Latin accentue.
static const char *const FAMILLE_REPLI = "DejaVu Sans";
static const char *const FAMILLE_REPLI_FIXE = "DejaVu Sans Mono";

/// Les familles que Qt connait, en minuscules. Reconstruit a la demande.
///
/// `QFontDatabase::families()` reconstruit une liste a chaque appel ; la
/// consulter une fois par glyphe couterait plus cher que de peindre. Le cache
/// est vide par `bo.police()`, la seule chose qui puisse en ajouter.
static QSet<QString> g_familles;
static bool g_familles_a_jour = false;

void oublieFamilles() { g_familles_a_jour = false; }

bool familleConnue(const QString &nom)
{
    if (nom.isEmpty())
        return false;
    if (!g_familles_a_jour) {
        g_familles.clear();
        const QStringList liste = QFontDatabase().families();
        for (const QString &famille : liste)
            g_familles.insert(famille.toLower());
        g_familles_a_jour = true;
    }
    return g_familles.contains(nom.toLower());
}

/// S'assure que Qt a de VRAIES polices, et les charge lui-meme sinon.
///
/// # Le defaut que cette fonction avait
///
/// Elle se contentait de compter : `if (familles > 0) return;`. Or Qt rend
/// toujours au moins une famille -- son repli interne, une bitmap ASCII sans
/// aucune lettre accentuee. Le compte etait donc satisfait, DejaVu n'etait
/// JAMAIS chargee, et toute page s'affichait dans cette bitmap : des lettres
/// carrees, et un `.notdef` -- le fameux carre vide -- a la place de chaque
/// `e` accentue. C'est ce que montraient les captures d'ecran.
///
/// Compter ne dit rien. Il faut demander la police par son NOM.
///
/// # Ce que Qt fait, et ce qu'il ne fait pas
///
/// Ce Qt est construit sans fontconfig : c'est `QBasicFontDatabase` qui cherche
/// les fichiers, en balayant `QT_QPA_FONTDIR`. Quand ce balayage ne donne rien —
/// repertoire absent, format refuse, variable perdue en route — il n'y a **aucun
/// message** : `QPainter::drawText` continue de reussir et peint dans le repli.
///
/// `addApplicationFontFromData` ne depend d'aucun balayage. Les polices sont
/// deposees par le noyau (`src/kernel/sysroot.rs`), elles sont donc toujours la.
void assurePolices()
{
    oublieFamilles();
    if (!familleConnue(QString::fromLatin1(FAMILLE_REPLI))) {
        const char *dossier = std::getenv("QT_QPA_FONTDIR");
        if (!dossier || !*dossier)
            dossier = "/usr/share/fonts/truetype/dejavu";

        int charges = 0;
        if (DIR *repertoire = opendir(dossier)) {
            while (struct dirent *entree = readdir(repertoire)) {
                const size_t longueur = std::strlen(entree->d_name);
                if (longueur < 5)
                    continue;
                const char *extension = entree->d_name + longueur - 4;
                if (std::strcmp(extension, ".ttf") && std::strcmp(extension, ".otf"))
                    continue;
                char chemin[512];
                std::snprintf(chemin, sizeof chemin, "%s/%s", dossier, entree->d_name);
                QFile fichier(QString::fromUtf8(chemin));
                if (!fichier.open(QIODevice::ReadOnly))
                    continue;
                if (QFontDatabase::addApplicationFontFromData(fichier.readAll()) >= 0)
                    ++charges;
            }
            closedir(repertoire);
        }
        oublieFamilles();
        std::printf("[bo] polices : %s absente, %d fichier(s) charge(s) depuis %s\n",
                    FAMILLE_REPLI, charges, dossier);
    }

    const QStringList liste = QFontDatabase().families();
    const bool repli = familleConnue(QString::fromLatin1(FAMILLE_REPLI));
    std::printf("[bo] polices : %d famille(s), %s=%s\n", liste.size(),
                FAMILLE_REPLI, repli ? "presente" : "ABSENTE");
    for (int index = 0; index < liste.size() && index < 8; ++index)
        std::printf("[bo] police %d : %s\n", index,
                    liste.at(index).toUtf8().constData());

    if (!repli) {
        std::fprintf(stderr, "[bo] AUCUNE POLICE LATINE : les lettres accentuees "
                             "s'afficheront en carres\n");
        return;
    }

    // La police par defaut de l'application, et les substitutions des familles
    // generiques du web. Sans elles, une page qui demande « Google Sans » ou
    // « -apple-system » -- c'est-a-dire presque toutes -- retombait sur le repli
    // interne de Qt, pas sur DejaVu.
    QApplication::setFont(QFont(QString::fromLatin1(FAMILLE_REPLI), 12));
    static const char *const generiques[] = {
        "sans-serif", "serif", "system-ui", "-apple-system", "BlinkMacSystemFont",
        "Segoe UI", "Roboto", "Helvetica", "Helvetica Neue", "Arial",
        "Google Sans", "Noto Sans", "Liberation Sans", "Ubuntu", "Cantarell",
    };
    for (const char *nom : generiques)
        QFont::insertSubstitution(QString::fromLatin1(nom),
                                  QString::fromLatin1(FAMILLE_REPLI));
    static const char *const fixes[] = {
        "monospace", "ui-monospace", "Courier", "Courier New", "Consolas",
        "Menlo", "Monaco", "Roboto Mono", "Liberation Mono",
    };
    for (const char *nom : fixes)
        QFont::insertSubstitution(QString::fromLatin1(nom),
                                  QString::fromLatin1(FAMILLE_REPLI_FIXE));
}

void diagnostiqueRasterQt()
{
    QImage interne(64, 64, QImage::Format_RGB32);
    QPainter peintre;
    const bool active = !interne.isNull() && peintre.begin(&interne);
    if (active) {
        peintre.fillRect(interne.rect(), QColor(30, 90, 180));
        peintre.end();
    }
    std::printf("[bo] raster Qt : QImage=%s paintEngine=%s QPainter=%s\n",
                interne.isNull() ? "nulle" : "ok",
                (!interne.isNull() && interne.paintEngine()) ? "ok" : "absent",
                active ? "actif" : "inactif");

    // Une mesure de controle : une largeur nulle pour un mot non vide veut dire
    // qu'aucune police n'est utilisable, et donc qu'aucun texte ne sera peint.
    const double largeur = QFontMetricsF(QFont()).horizontalAdvance(
        QStringLiteral("Bouchaud"));
    std::printf("[bo] mesure texte : \"Bouchaud\" = %.1f px (%s)\n", largeur,
                largeur > 1.0 ? "police utilisable" : "AUCUNE POLICE UTILISABLE");
}

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
    // police du systeme.
    //
    // Mais seulement si Qt la CONNAIT. Laisser une famille inconnue a la
    // substitution de Qt, c'est ce qui ramenait le repli interne -- une bitmap
    // ASCII sans lettres accentuees -- au lieu de DejaVu. La resolution est
    // donc faite ici, ou l'on sait ce que la base contient.
    if (!famille.isEmpty() && familleConnue(famille))
        f.setFamily(famille);
    else if (fixe)
        f.setFamily(QString::fromLatin1(FAMILLE_REPLI_FIXE));
    else
        f.setFamily(QString::fromLatin1(FAMILLE_REPLI));
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
        sortie.ouvre();
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
        // Sur Bouchaud OS on ne depend plus du backing-store du QWidget pour
        // produire les pixels. On peint dans une QImage enveloppant des pages
        // mmappees puis on copie vers /dev/fb0 (ou la future surface du WM).
        if (sortie.active()) {
            if (!trame.assure(width(), height())) {
                if (!avertiTrame_) {
                    std::fprintf(stderr, "[bo] trame raster : mmap/QImage impossible\n");
                    avertiTrame_ = true;
                }
                sortie.remplit(0x00301020);
                return;
            }

            QImage &image = trame.image();
            image.fill(QColor(255, 255, 255));
            QPainter p(&image);
            if (!p.isActive()) {
                if (!avertiRaster_) {
                    std::fprintf(stderr,
                                 "[bo] QPainter(QImage) inactif : moteur raster Qt indisponible\n");
                    avertiRaster_ = true;
                }
                sortie.remplit(0x00301020);
                return;
            }
            p.setRenderHint(QPainter::Antialiasing, true);
            p.setRenderHint(QPainter::TextAntialiasing, true);
            peintPage(p);
            p.end();
            sortie.presente(image);
            // Le degat est la trame entiere : le moteur ne dit pas encore
            // quelles regions il a refaites. Le message porte deja le
            // rectangle, c'est donc le seul endroit a changer le jour ou il le
            // dira — le protocole, lui, n'aura pas a bouger.
            g_pont.trameLivree(0, 0, image.width(), image.height());

            // linuxfb peut flusher son propre backing-store juste apres le
            // paintEvent. Une seconde copie au tour d'evenements suivant fait
            // gagner notre frame sans bloquer la boucle Qt.
            QTimer::singleShot(0, this, [this] {
                if (sortie.active()) {
                    sortie.presente(trame.image());
                    g_pont.trameLivree(0, 0, trame.image().width(), trame.image().height());
                }
            });
            return;
        }

        // CI/offscreen et hote Linux classique : conserver le chemin QWidget.
        QPainter p(this);
        if (!p.isActive()) {
            if (!avertiWidget_) {
                std::fprintf(stderr, "[bo] QPainter(QWidget) inactif\n");
                avertiWidget_ = true;
            }
            return;
        }
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setRenderHint(QPainter::TextAntialiasing, true);
        p.fillRect(rect(), QColor(255, 255, 255));
        peintPage(p);
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
    void peintPage(QPainter &p)
    {
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
    SortieBouchaud sortie;
    TrameRaster trame;
    bool avertiTrame_ = false;
    bool avertiRaster_ = false;
    bool avertiWidget_ = false;

    QVector<QRectF> pileRognage;
    /// Etats de peintre empiles par « transforme » et « opacite ».
    int enveloppes = 0;
};

// --- Pont : reception -------------------------------------------------------
//
// Definie ici, apres `Toile` et `appelle` : la distribution des evenements passe
// par les memes rappels Python que les gestionnaires de Qt, et il n'y a donc
// qu'un seul chemin d'entree dans le moteur, quel que soit le systeme hote.

void PontFenetre::pompe()
{
    if (!actif())
        return;
    unsigned char bloc[4096];
    for (;;) {
        // On ne lit que ce que `poll` declare lisible. Une lecture speculative
        // sur un descripteur qui n'aurait pas pris le mode non bloquant
        // arreterait la boucle d'evenements de Qt — c'est-a-dire tout le
        // navigateur — pour un message qui n'existe pas.
        struct pollfd surveille;
        surveille.fd = fd_;
        surveille.events = POLLIN;
        surveille.revents = 0;
        if (::poll(&surveille, 1, 0) <= 0 || !(surveille.revents & POLLIN))
            break;
        const ssize_t lu = ::read(fd_, bloc, sizeof(bloc));
        if (lu > 0) {
            tampon_.append(reinterpret_cast<const char *>(bloc), int(lu));
            continue;
        }
        break; // EAGAIN, fin de fichier, ou erreur : on traite ce qu'on a.
    }

    for (;;) {
        if (size_t(tampon_.size()) < protocole::TAILLE_ENTETE)
            return;
        const unsigned char *octets =
            reinterpret_cast<const unsigned char *>(tampon_.constData());
        if (protocole::litU32(octets, 0) != protocole::MAGIC) {
            // Flux desynchronise : rien de bon ne peut en sortir, et repartir
            // d'un octet plus loin ne ferait qu'inventer des messages.
            std::fprintf(stderr, "[bo] pont : flux invalide, canal abandonne\n");
            tampon_.clear();
            fd_ = -1;
            return;
        }
        const uint16_t genre = uint16_t(octets[6]) | (uint16_t(octets[7]) << 8);
        const uint32_t taille = protocole::litU32(octets, 8);
        if (taille > protocole::CHARGE_MAX) {
            tampon_.clear();
            fd_ = -1;
            return;
        }
        const size_t total = protocole::TAILLE_ENTETE + taille;
        if (size_t(tampon_.size()) < total)
            return;
        traite(genre, octets + protocole::TAILLE_ENTETE, taille);
        tampon_.remove(0, int(total));
    }
}

void PontFenetre::traite(uint16_t genre, const unsigned char *charge, size_t taille)
{
    ++evenements_;
    switch (genre) {
    case protocole::Pointer: {
        if (taille < 16)
            return;
        const int x = protocole::litI32(charge, 4);
        const int y = protocole::litI32(charge, 8);
        const uint32_t boutons = protocole::litU32(charge, 12);
        const bool appuyait = boutons_ & 1;
        const bool appuie = boutons & 1;
        boutons_ = boutons;
        // Le front montant fait le clic ; le reste est du survol. Le bureau
        // envoie une position absolue a chaque mouvement, ce qui rend la perte
        // d'un message sans consequence — mais pas celle d'un front.
        if (appuie && !appuyait) {
            appelle("clic", "(ii)", x, y);
            if (g_toile)
                g_toile->update();
        } else {
            appelle("survol", "(ii)", x, y);
        }
        return;
    }
    case protocole::Wheel: {
        if (taille < 8)
            return;
        appelle("molette", "(i)", protocole::litI32(charge, 4));
        if (g_toile)
            g_toile->update();
        return;
    }
    case protocole::Key: {
        if (taille < 20)
            return;
        const uint32_t code = protocole::litU32(charge, 4);
        const uint32_t modificateurs = protocole::litU32(charge, 8);
        const uint32_t unicode = protocole::litU32(charge, 12);
        const uint32_t appui = protocole::litU32(charge, 16);
        if (!appui)
            return;

        int qtKey = 0;
        QString texte;
        switch (code) {
        case protocole::ToucheEntree: qtKey = Qt::Key_Return; break;
        case protocole::ToucheRetour: qtKey = Qt::Key_Backspace; break;
        case protocole::ToucheTabulation: qtKey = Qt::Key_Tab; break;
        case protocole::ToucheHaut: qtKey = Qt::Key_Up; break;
        case protocole::ToucheBas: qtKey = Qt::Key_Down; break;
        case protocole::ToucheGauche: qtKey = Qt::Key_Left; break;
        case protocole::ToucheDroite: qtKey = Qt::Key_Right; break;
        case protocole::ToucheEchap: qtKey = Qt::Key_Escape; break;
        default:
            if (!unicode)
                return;
            texte = QString(QChar(ushort(unicode)));
            // Qt designe les lettres par leur majuscule : le moteur compare
            // `code == K_L` pour Ctrl+L, et minuscule ne correspondrait a rien.
            qtKey = texte.at(0).toUpper().unicode();
            break;
        }
        // Le bureau rapporte desormais l'etat des modificateurs : le pilote
        // PS/2 suit Shift, Ctrl, Alt et AltGr et le champ n'est plus toujours
        // nul. Les valeurs sont nommees des deux cotes, et la barriere du
        // protocole verifie qu'elles concordent.
        // `Modificateur` vit dans `namespace protocole` ; ce pont est dans le
        // namespace anonyme. Sans qualification, la compilation echoue --
        // « `Modificateur` has not been declared » -- et c'est ce qui laissait
        // le job `qt pixel` rouge.
        int qtMods = 0;
        if (modificateurs & protocole::Modificateur::ModShift)
            qtMods |= Qt::ShiftModifier;
        if (modificateurs & protocole::Modificateur::ModCtrl)
            qtMods |= Qt::ControlModifier;
        if (modificateurs & protocole::Modificateur::ModAlt)
            qtMods |= Qt::AltModifier;
        appelle("touche", "(isi)", qtKey, texte.toUtf8().constData(), qtMods);
        if (g_toile)
            g_toile->update();
        return;
    }
    case protocole::CloseRequest:
        appelle("fermeture");
        QCoreApplication::quit();
        return;
    case protocole::Configure:
    case protocole::Surface:
    case protocole::Focus:
        // La geometrie est imposee par le bureau et connue par la surface : ces
        // messages ne servent aujourd'hui qu'a confirmer ce que l'on a deja.
        return;
    default:
        return;
    }
}

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
        // Plein ecran : sous le gestionnaire de fenetres de Bouchaud OS,
        // « l'ecran » est la surface de la fenetre, pas la dalle. Qt dimensionne
        // donc la toile exactement sur la zone utile que le bureau a allouee.
        g_toile->showFullScreen();
        g_toile->setFocus();
        if (g_pont.actif()) {
            g_pont.salue(g_toile->width(), g_toile->height());
            g_pont.titre(QByteArray(titre));
        }
    }
    // Battement de service : c'est par lui qu'un fil Python fait executer du
    // travail sur le fil principal. Qt n'accepte d'etre touche que depuis
    // celui-ci ; un `QFontMetrics` construit ailleurs suffit a faire tomber le
    // programme.
    static QTimer *battement = nullptr;
    if (!battement) {
        battement = new QTimer();
        QObject::connect(battement, &QTimer::timeout, [] {
            // Le battement du moteur d'abord, le pont ensuite. L'ordre inverse
            // paraissait meilleur — traiter une touche avant de redessiner fait
            // gagner une trame — mais il fait dependre le cœur du navigateur du
            // bon fonctionnement du pont : si celui-ci se bloque, les minuteries
            // de la page ne battent plus et rien ne charge. Le moteur doit
            // pouvoir vivre meme sans gestionnaire de fenetres.
            ++g_pont.tics_;
            appelle("tic");
            g_pont.pompe();

            // Un releve toutes les cinq secondes, en face de celui du noyau : si
            // les deux comptes divergent, on saura de quel cote regarder.
            if (g_pont.tics_ % 312 == 0) {
                std::printf("[bo] battement : %lu tics, %lu trames, %lu evenements recus\n",
                            g_pont.tics_, g_pont.trames_, g_pont.evenements_);
                std::fflush(stdout);
            }
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
    // La base vient de changer : `familleConnue` doit la relire, sinon la
    // police que la page vient de livrer serait jugee inconnue et remplacee par
    // le repli — c'est-a-dire jamais utilisee.
    oublieFamilles();
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

/// `bo.debrotli(octets) -> bytes`
///
/// Decompresse un flux brotli. Le navigateur lie deja `libbrotlidec` — freetype
/// en depend — mais Python n'y avait pas acces. C'est ce qui manquait pour lire
/// une police en WOFF2, c'est-a-dire la quasi-totalite de celles que les sites
/// livrent aujourd'hui, dont les polices d'icones.
PyObject *bo_debrotli(PyObject *, PyObject *args)
{
    Py_buffer entree;
    if (!PyArg_ParseTuple(args, "y*", &entree))
        return nullptr;

    // La taille decompressee n'est pas connue d'avance : on double le tampon
    // tant que le decodeur en reclame. Une police depasse rarement quelques
    // centaines de kilo-octets, d'ou le plafond.
    const size_t plafond = 64u * 1024u * 1024u;
    size_t capacite = std::max<size_t>(size_t(entree.len) * 4, 64u * 1024u);
    QByteArray sortie;
    BrotliDecoderResult resultat = BROTLI_DECODER_RESULT_NEEDS_MORE_OUTPUT;

    while (resultat == BROTLI_DECODER_RESULT_NEEDS_MORE_OUTPUT && capacite <= plafond) {
        sortie.resize(int(capacite));
        size_t taille = capacite;
        resultat = BrotliDecoderDecompress(
            size_t(entree.len), static_cast<const uint8_t *>(entree.buf), &taille,
            reinterpret_cast<uint8_t *>(sortie.data()));
        if (resultat == BROTLI_DECODER_RESULT_SUCCESS) {
            sortie.resize(int(taille));
            PyBuffer_Release(&entree);
            return PyBytes_FromStringAndSize(sortie.constData(), sortie.size());
        }
        capacite *= 2;
    }
    PyBuffer_Release(&entree);
    PyErr_SetString(PyExc_ValueError, "bo.debrotli : flux illisible ou trop gros");
    return nullptr;
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
    {"debrotli", bo_debrotli, METH_VARARGS,
     "debrotli(octets) -> bytes : decompresse un flux brotli"},
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
    // Le pont s'ouvre **avant** QApplication : le descripteur vient du
    // gestionnaire de fenetres via l'environnement, et le savoir des la premiere
    // ligne permet de journaliser sous quel regime on tourne — fenetre du
    // bureau, ou programme seul devant un framebuffer.
    g_pont.ouvre();

    QApplication app(argc, argv);
    assurePolices();
    diagnostiqueRasterQt();

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
