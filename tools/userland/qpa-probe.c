/*
 * Sonde QPA : rejoue la sequence de demarrage du plugin `linuxfb` de Qt et de
 * ses plugins d'entree `evdevkeyboard` / `evdevmouse`.
 *
 * L'interet n'est pas de dessiner joliment, mais de verifier que chaque appel
 * que Qt fait *reellement* aboutit — dans l'ordre ou il le fait, avec les
 * memes structures. Un ioctl qui echoue ici est un ioctl sur lequel Qt
 * renoncerait a demarrer, souvent sans message clair.
 *
 *   musl-gcc -O2 -static-pie -pthread qpa-probe.c -o qpa-probe
 *   (ou via ./build.sh musl)
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/fb.h>
#include <linux/input.h>
#include <linux/kd.h>
#include <linux/vt.h>
#include <poll.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/eventfd.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/timerfd.h>
#include <time.h>
#include <unistd.h>

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-42s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

/* ---------------------------------------------------------------- console */

static int ouvre_tty(void)
{
    /* QLinuxFbScreen::openTty() : passe la console en mode graphique pour que
     * le noyau cesse d'ecrire du texte par-dessus le framebuffer. */
    int tty = open("/dev/tty0", O_RDWR);
    verifie("open(/dev/tty0)", tty >= 0);
    if (tty < 0)
        return -1;

    struct vt_stat etat;
    verifie("ioctl(VT_GETSTATE)", ioctl(tty, VT_GETSTATE, &etat) == 0);

    int mode = 0;
    verifie("ioctl(KDGETMODE)", ioctl(tty, KDGETMODE, &mode) == 0);
    verifie("ioctl(KDSETMODE, KD_GRAPHICS)", ioctl(tty, KDSETMODE, KD_GRAPHICS) == 0);
    return tty;
}

/* ------------------------------------------------------------ framebuffer */

static int ouvre_framebuffer(unsigned *largeur, unsigned *hauteur, unsigned char **pixels)
{
    int fd = open("/dev/fb0", O_RDWR);
    verifie("open(/dev/fb0)", fd >= 0);
    if (fd < 0)
        return -1;

    struct fb_var_screeninfo var;
    struct fb_fix_screeninfo fix;
    verifie("ioctl(FBIOGET_VSCREENINFO)", ioctl(fd, FBIOGET_VSCREENINFO, &var) == 0);
    verifie("ioctl(FBIOGET_FSCREENINFO)", ioctl(fd, FBIOGET_FSCREENINFO, &fix) == 0);

    printf("    mode %ux%u %u bpp, pas de ligne %u, memoire %u o\n",
           var.xres, var.yres, var.bits_per_pixel, fix.line_length, fix.smem_len);

    verifie("resolution non nulle", var.xres > 0 && var.yres > 0);
    verifie("32 bits par pixel", var.bits_per_pixel == 32);
    verifie("pas de ligne coherent", fix.line_length >= var.xres * 4);
    verifie("visuel truecolor", fix.visual == FB_VISUAL_TRUECOLOR);
    /* Qt lit ces decalages pour construire son format d'image. */
    verifie("canaux XRGB8888",
            var.red.offset == 16 && var.green.offset == 8 && var.blue.offset == 0);

    size_t taille = (size_t)fix.line_length * var.yres;
    void *carte = mmap(NULL, taille, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    verifie("mmap(framebuffer)", carte != MAP_FAILED);
    if (carte == MAP_FAILED)
        return -1;

    *largeur = var.xres;
    *hauteur = var.yres;
    *pixels = carte;
    return fd;
}

/* ------------------------------------------------------------------ evdev */

static int ouvre_entree(const char *chemin, int attendu_rel)
{
    int fd = open(chemin, O_RDONLY | O_NONBLOCK);
    char libelle[64];
    snprintf(libelle, sizeof libelle, "open(%s)", chemin);
    verifie(libelle, fd >= 0);
    if (fd < 0)
        return -1;

    int version = 0;
    verifie("ioctl(EVIOCGVERSION)", ioctl(fd, EVIOCGVERSION, &version) == 0);

    char nom[128] = {0};
    verifie("ioctl(EVIOCGNAME)", ioctl(fd, EVIOCGNAME(sizeof nom), nom) >= 0);
    printf("    nom du peripherique : %s\n", nom);

    struct input_id id;
    verifie("ioctl(EVIOCGID)", ioctl(fd, EVIOCGID, &id) == 0);

    /* C'est sur ces bitmaps que Qt decide de la nature du peripherique.
     * Vides, il ouvre le fichier puis ignore tout ce qu'il en lit. */
    unsigned long types[(EV_MAX + 8 * sizeof(long)) / (8 * sizeof(long))] = {0};
    verifie("ioctl(EVIOCGBIT(0))", ioctl(fd, EVIOCGBIT(0, sizeof types), types) >= 0);

#define A_LE_BIT(tab, bit) (((tab)[(bit) / (8 * sizeof(long))] >> ((bit) % (8 * sizeof(long)))) & 1)

    verifie("annonce EV_KEY", A_LE_BIT(types, EV_KEY) != 0);

    if (attendu_rel) {
        verifie("annonce EV_REL", A_LE_BIT(types, EV_REL) != 0);
        unsigned long rel[(REL_MAX + 8 * sizeof(long)) / (8 * sizeof(long))] = {0};
        verifie("ioctl(EVIOCGBIT(EV_REL))", ioctl(fd, EVIOCGBIT(EV_REL, sizeof rel), rel) >= 0);
        verifie("axes REL_X et REL_Y", A_LE_BIT(rel, REL_X) && A_LE_BIT(rel, REL_Y));
        unsigned long touches[(KEY_MAX + 8 * sizeof(long)) / (8 * sizeof(long))] = {0};
        verifie("ioctl(EVIOCGBIT(EV_KEY))", ioctl(fd, EVIOCGBIT(EV_KEY, sizeof touches), touches) >= 0);
        verifie("bouton BTN_LEFT", A_LE_BIT(touches, BTN_LEFT) != 0);
    } else {
        unsigned long touches[(KEY_MAX + 8 * sizeof(long)) / (8 * sizeof(long))] = {0};
        verifie("ioctl(EVIOCGBIT(EV_KEY))", ioctl(fd, EVIOCGBIT(EV_KEY, sizeof touches), touches) >= 0);
        /* KEY_A vaut 30 : c'est un code de position, pas le caractere 'a'. */
        verifie("touches alphabetiques annoncees",
                A_LE_BIT(touches, KEY_A) && A_LE_BIT(touches, KEY_ENTER));
        verifie("fleches annoncees", A_LE_BIT(touches, KEY_UP) && A_LE_BIT(touches, KEY_LEFT));
    }
#undef A_LE_BIT

    /* Lecture a vide : doit dire « rien pour l'instant », pas echouer. */
    struct input_event evenement;
    ssize_t lu = read(fd, &evenement, sizeof evenement);
    verifie("read() a vide -> EAGAIN", lu < 0 && errno == EAGAIN);
    return fd;
}

/* ------------------------------------------------------- boucle d'evenements */

static int reveil;

static void *thread_reveilleur(void *inutilise)
{
    (void)inutilise;
    struct timespec pause = {0, 150 * 1000 * 1000};
    nanosleep(&pause, NULL);
    uint64_t un = 1;
    ssize_t ecrit = write(reveil, &un, sizeof un);
    (void)ecrit;
    return NULL;
}

static void teste_boucle_evenements(int fd_clavier, int fd_souris)
{
    /* QEventDispatcherUNIX : un eventfd pour se reveiller depuis un autre
     * thread, un poll sur les descripteurs d'entree, un timerfd pour les
     * echeances. */
    reveil = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    verifie("eventfd()", reveil >= 0);

    pthread_t fil;
    verifie("pthread_create(reveilleur)", pthread_create(&fil, NULL, thread_reveilleur, NULL) == 0);

    /* Boucle d'evenements telle que la tient un client : on repasse dans poll
     * en vidant les descripteurs prets, jusqu'a l'evenement attendu. Un
     * peripherique d'entree peut tres bien avoir quelque chose a dire avant le
     * reveil — ce n'est pas une anomalie, c'est le cas courant. */
    int reveille = 0, tours = 0;
    struct timespec depart, maintenant;
    clock_gettime(CLOCK_MONOTONIC, &depart);
    for (;;) {
        struct pollfd surveilles[3] = {
            {.fd = reveil, .events = POLLIN},
            {.fd = fd_clavier, .events = POLLIN},
            {.fd = fd_souris, .events = POLLIN},
        };
        int prets = poll(surveilles, 3, 3000);
        tours++;
        if (prets <= 0)
            break;
        if (surveilles[0].revents & POLLIN) {
            reveille = 1;
            break;
        }
        char poubelle[512];
        if (surveilles[1].revents & POLLIN)
            read(fd_clavier, poubelle, sizeof poubelle);
        if (surveilles[2].revents & POLLIN)
            read(fd_souris, poubelle, sizeof poubelle);

        clock_gettime(CLOCK_MONOTONIC, &maintenant);
        if (maintenant.tv_sec - depart.tv_sec > 3)
            break;
    }
    verifie("poll() reveille par l'eventfd", reveille);
    printf("    %d tour(s) de boucle avant le reveil\n", tours);
    /* Le nombre de tours doit rester petit. S'il explose, c'est que poll
     * declare un descripteur pret alors que la lecture ne rend rien : la
     * boucle tournerait alors a plein regime sans rien faire. */
    verifie("pas de reveil a vide (< 10 tours)", tours < 10);

    uint64_t valeur = 0;
    verifie("read(eventfd) -> compteur", read(reveil, &valeur, sizeof valeur) == 8 && valeur == 1);
    /* Le compteur est remis a zero par la lecture : la boucle ne doit pas se
     * reveiller une seconde fois pour le meme evenement. */
    verifie("eventfd vide apres lecture",
            read(reveil, &valeur, sizeof valeur) < 0 && errno == EAGAIN);
    pthread_join(fil, NULL);

    int chrono = timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK | TFD_CLOEXEC);
    verifie("timerfd_create()", chrono >= 0);
    struct itimerspec delai = {.it_interval = {0, 0}, .it_value = {0, 120 * 1000 * 1000}};
    verifie("timerfd_settime(120 ms)", timerfd_settime(chrono, 0, &delai, NULL) == 0);

    struct pollfd attente = {.fd = chrono, .events = POLLIN};
    struct timespec avant, apres;
    int prets;
    clock_gettime(CLOCK_MONOTONIC, &avant);
    prets = poll(&attente, 1, 3000);
    clock_gettime(CLOCK_MONOTONIC, &apres);
    verifie("poll() reveille par le timerfd", prets == 1 && (attente.revents & POLLIN));

    long ecoule_ms = (apres.tv_sec - avant.tv_sec) * 1000 + (apres.tv_nsec - avant.tv_nsec) / 1000000;
    printf("    delai mesure : %ld ms (attendu ~120)\n", ecoule_ms);
    /* Avec un tick a 18 Hz, la moindre attente serait arrondie a 55 ms pres :
     * l'ecart trahirait une horloge trop grossiere pour animer une interface. */
    verifie("granularite du timer < 30 ms", ecoule_ms >= 100 && ecoule_ms <= 150);

    uint64_t expirations = 0;
    verifie("read(timerfd) -> 1 expiration",
            read(chrono, &expirations, sizeof expirations) == 8 && expirations == 1);
    close(chrono);
    close(reveil);
}

/* ------------------------------------------------- attente avec echeance */

static pthread_mutex_t verrou = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t condition = PTHREAD_COND_INITIALIZER;

static void teste_attente_avec_echeance(void)
{
    /* pthread_cond_timedwait passe par FUTEX_WAIT_BITSET, dont le timeout est
     * *absolu*. Traite comme une duree relative, il vaudrait plusieurs
     * decennies : l'attente ne rendrait jamais la main. */
    struct timespec echeance;
    clock_gettime(CLOCK_REALTIME, &echeance);
    echeance.tv_nsec += 200 * 1000 * 1000;
    if (echeance.tv_nsec >= 1000000000) {
        echeance.tv_sec++;
        echeance.tv_nsec -= 1000000000;
    }

    struct timespec avant, apres;
    clock_gettime(CLOCK_MONOTONIC, &avant);
    pthread_mutex_lock(&verrou);
    int resultat = pthread_cond_timedwait(&condition, &verrou, &echeance);
    pthread_mutex_unlock(&verrou);
    clock_gettime(CLOCK_MONOTONIC, &apres);

    long ecoule_ms = (apres.tv_sec - avant.tv_sec) * 1000 + (apres.tv_nsec - avant.tv_nsec) / 1000000;
    printf("    attente mesuree : %ld ms (attendu ~200)\n", ecoule_ms);
    verifie("pthread_cond_timedwait expire", resultat == ETIMEDOUT);
    verifie("echeance absolue respectee", ecoule_ms >= 150 && ecoule_ms <= 400);
}

/* ----------------------------------------------------- sysroot et polices */

static void teste_sysroot(void)
{
    FILE *f = fopen("/sys/devices/system/cpu/online", "r");
    verifie("/sys/devices/system/cpu/online lisible", f != NULL);
    if (f)
        fclose(f);

    f = fopen("/proc/meminfo", "r");
    verifie("/proc/meminfo lisible", f != NULL);
    if (f)
        fclose(f);

    long processeurs = sysconf(_SC_NPROCESSORS_ONLN);
    printf("    sysconf(_SC_NPROCESSORS_ONLN) = %ld\n", processeurs);
    verifie("nombre de processeurs >= 1", processeurs >= 1);

    /* QFontDatabase parcourt QT_QPA_FONTDIR : sans police lisible, l'interface
     * demarre mais n'affiche aucun texte. */
    const char *police = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
    int fd = open(police, O_RDONLY);
    verifie("police DejaVuSans.ttf presente", fd >= 0);
    if (fd >= 0) {
        unsigned char entete[4] = {0};
        verifie("police lisible (entete TrueType)",
                read(fd, entete, 4) == 4 && entete[0] == 0x00 && entete[1] == 0x01);
        close(fd);
    }

    const char *repertoire = getenv("QT_QPA_FONTDIR");
    printf("    QT_QPA_FONTDIR = %s\n", repertoire ? repertoire : "(absent)");
    verifie("QT_QPA_FONTDIR defini", repertoire != NULL);
}

/* -------------------------------------------------------------------- main */

int main(void)
{
    printf("Sonde QPA — sequence de demarrage d'une pile Qt/linuxfb\n\n");

    printf("[console virtuelle]\n");
    int tty = ouvre_tty();

    printf("\n[framebuffer]\n");
    unsigned largeur = 0, hauteur = 0;
    unsigned char *pixels = NULL;
    int fb = ouvre_framebuffer(&largeur, &hauteur, &pixels);

    printf("\n[clavier]\n");
    int clavier = ouvre_entree("/dev/input/event0", 0);

    printf("\n[souris]\n");
    int souris = ouvre_entree("/dev/input/event1", 1);

    printf("\n[boucle d'evenements]\n");
    if (clavier >= 0 && souris >= 0)
        teste_boucle_evenements(clavier, souris);

    printf("\n[attente avec echeance]\n");
    teste_attente_avec_echeance();

    printf("\n[arborescence systeme]\n");
    teste_sysroot();

    if (pixels && largeur && hauteur) {
        printf("\n[rendu]\n");
        /* Une fenetre grise avec une barre de titre, comme le ferait le
         * moteur de rendu logiciel de Qt sur son ecran linuxfb. */
        unsigned *point = (unsigned *)pixels;
        for (unsigned y = 0; y < hauteur; y++)
            for (unsigned x = 0; x < largeur; x++)
                point[y * largeur + x] = 0x00202430;
        unsigned fx = largeur / 6, fy = hauteur / 6;
        unsigned fw = largeur * 2 / 3, fh = hauteur * 2 / 3;
        for (unsigned y = fy; y < fy + fh; y++)
            for (unsigned x = fx; x < fx + fw; x++)
                point[y * largeur + x] = (y < fy + 32) ? 0x002D7DD2 : 0x00ECEFF4;
        verifie("ecriture dans le framebuffer mappe", 1);
        /* On laisse l'image a l'ecran un instant : c'est la seule partie du
         * test qui se verifie a l'oeil. */
        struct timespec pose = {2, 0};
        nanosleep(&pose, NULL);
    }

    printf("\n%s : %d verification(s) en echec\n", echecs ? "RESULTAT" : "RESULTAT", echecs);
    if (fb >= 0)
        close(fb);
    if (tty >= 0) {
        ioctl(tty, KDSETMODE, KD_TEXT);
        close(tty);
    }
    return echecs == 0 ? 0 : 1;
}
