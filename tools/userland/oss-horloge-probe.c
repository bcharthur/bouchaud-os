/*
 * Sonde : l'horloge OSS de /dev/dsp avance au rythme du son joue.
 *
 * BOUCHAUD_OSS_HORLOGE_V1
 *
 * Ladybird (PlaybackStreamBouchaud) n'a pas d'autre horloge que celle-ci :
 *
 *     joue = trames ecrites - SNDCTL_DSP_GETODELAY
 *
 * et `HTMLMediaElement.currentTime` en depend. Au smoke 37491757410, apres
 * une premiere ecriture de 4800 trames, `currentTime` n'a jamais atteint
 * 0,5 s. La sonde reproduit EXACTEMENT la boucle du backend -- remplir
 * jusqu'a une cible de 100 ms (4800 trames) d'apres ODELAY et GETOSPACE,
 * relever toutes les 5 ms -- puis la meme chose par petites ecritures
 * (256 trames, la taille d'un quantum de melangeur), et verifie :
 *
 *   H1 ODELAY ne depasse JAMAIS ce qui a ete ecrit et pas encore joue :
 *      un delai plus grand que l'ecrit fige l'horloge a zero ;
 *   H2 l'horloge avance au rythme reel : sur 3 s, entre 80 % et 105 % du
 *      temps ecoule (QEMU `-audiodev none` consomme en temps reel) ;
 *   H3 meme chose par petites ecritures ;
 *   H4 SNDCTL_DSP_SYNC rend quand tout est joue : ODELAY = 0 ensuite ;
 *   H5 apres arret du moteur, une nouvelle ecriture repart (ODELAY > 0 puis
 *      redescend a 0) ;
 *   H6 GETBLKSIZE et GETOSPACE.fragsize annoncent la meme taille.
 *
 * Sortie : `OSS_HORLOGE_OK` ou `OSS_HORLOGE_ECHEC n=...`.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

#define SNDCTL_DSP_SYNC 0x00005001
#define SNDCTL_DSP_SPEED 0xC0045002
#define SNDCTL_DSP_GETBLKSIZE 0xC0045004
#define SNDCTL_DSP_SETFMT 0xC0045005
#define SNDCTL_DSP_CHANNELS 0xC0045006
#define SNDCTL_DSP_GETOSPACE 0x800C500C
#define SNDCTL_DSP_GETODELAY 0x80045017
#define AFMT_S16_LE 0x00000010

#define TAUX 48000
#define OCTETS_PAR_TRAME 4
#define CIBLE 4800

struct audio_buf_info {
    int fragments;
    int fragstotal;
    int fragsize;
    int bytes;
};

static int echecs = 0;
static int fd = -1;
static int16_t pcm[2 * 8192];

static void verifie(const char *quoi, int condition)
{
    printf("  %-66s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

static double maintenant(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}

static long odelay_trames(void)
{
    int octets = -1;
    if (ioctl(fd, SNDCTL_DSP_GETODELAY, &octets) != 0)
        return -1;
    return octets / OCTETS_PAR_TRAME;
}

static long libres_trames(void)
{
    struct audio_buf_info info;
    if (ioctl(fd, SNDCTL_DSP_GETOSPACE, &info) != 0)
        return -1;
    return info.bytes / OCTETS_PAR_TRAME;
}

static int ecrit_tout(long trames)
{
    static double phase = 0;
    for (long i = 0; i < trames; i++) {
        int16_t v = (int16_t)(8000 * sin(phase));
        phase += 2 * M_PI * 440.0 / TAUX;
        pcm[2 * i] = v;
        pcm[2 * i + 1] = v;
    }
    const char *p = (const char *)pcm;
    size_t reste = (size_t)trames * OCTETS_PAR_TRAME;
    int vides = 0;
    while (reste > 0) {
        ssize_t n = write(fd, p, reste);
        if (n < 0 && (errno == EINTR || errno == EAGAIN)) {
            usleep(1000);
            continue;
        }
        if (n <= 0) {
            // write() qui rend 0 : le backend bouclerait. Borne pour la sonde.
            if (++vides > 1000)
                return -1;
            usleep(1000);
            continue;
        }
        p += n;
        reste -= (size_t)n;
    }
    return 0;
}

/* La boucle du backend Ladybird. `quantum` borne chaque ecriture (0 = rien). */
static void phase(const char *nom, double duree, long quantum, int *h_delai, double *rapport)
{
    long ecrites = 0, joue = 0, pire_exces = 0, releves = 0, sous = 0;
    double t0 = maintenant(), debut_jeu = -1;
    while (maintenant() - t0 < duree) {
        long en_vol = odelay_trames();
        if (en_vol < 0)
            break;
        releves++;
        if (en_vol > ecrites - joue && en_vol - (ecrites - joue) > pire_exces)
            pire_exces = en_vol - (ecrites - joue);
        long j = ecrites > en_vol ? ecrites - en_vol : 0;
        if (j > joue)
            joue = j;
        if (debut_jeu >= 0 && en_vol == 0)
            sous++;
        if (en_vol < CIBLE) {
            long voulu = CIBLE - en_vol;
            long possible = libres_trames();
            if (possible > voulu)
                possible = voulu;
            if (possible > 8192)
                possible = 8192;
            // Par quanta (un melangeur rend 256 trames a la fois) : autant
            // d'ecritures -- donc de descripteurs -- que de quanta.
            while (possible > 0) {
                long n = quantum > 0 && possible > quantum ? quantum : possible;
                if (ecrit_tout(n) != 0)
                    break;
                ecrites += n;
                possible -= n;
                if (debut_jeu < 0)
                    debut_jeu = maintenant();
            }
        }
        usleep(5000);
    }
    double ecoule = debut_jeu >= 0 ? maintenant() - debut_jeu : 0;
    *rapport = ecoule > 0 ? ((double)joue / TAUX) / ecoule : 0;
    *h_delai = pire_exces == 0;
    printf("oss-horloge-probe %s : ecrites=%ld jouees=%ld ecoule_ms=%d rapport=%.3f releves=%ld "
           "pire_exces_odelay=%ld sous_alimentations=%ld\n",
        nom, ecrites, joue, (int)(ecoule * 1000), *rapport, releves, pire_exces, sous);
}

int main(void)
{
    fd = open("/dev/dsp", O_WRONLY);
    if (fd < 0) {
        printf("oss-horloge-probe : /dev/dsp errno=%d\nOSS_HORLOGE_ECHEC n=1\n", errno);
        return 1;
    }
    int v = AFMT_S16_LE;
    ioctl(fd, SNDCTL_DSP_SETFMT, &v);
    v = 2;
    ioctl(fd, SNDCTL_DSP_CHANNELS, &v);
    v = TAUX;
    ioctl(fd, SNDCTL_DSP_SPEED, &v);

    int blk = 0;
    struct audio_buf_info info;
    memset(&info, 0, sizeof info);
    ioctl(fd, SNDCTL_DSP_GETBLKSIZE, &blk);
    ioctl(fd, SNDCTL_DSP_GETOSPACE, &info);
    printf("oss-horloge-probe : blksize=%d fragsize=%d fragments=%d/%d octets_libres=%d\n",
        blk, info.fragsize, info.fragments, info.fragstotal, info.bytes);
    verifie("H6 GETBLKSIZE = GETOSPACE.fragsize", blk > 0 && blk == info.fragsize);

    int h1, h3;
    double r1, r3;
    phase("grandes", 3.0, 0, &h1, &r1);
    verifie("H1 ODELAY ne depasse jamais l'ecrit non joue (cible 100 ms)", h1);
    verifie("H2 horloge au rythme reel : 0,80 <= joue/ecoule <= 1,05", r1 >= 0.80 && r1 <= 1.05);
    // Vider avant la phase suivante : ses compteurs partent de zero.
    ioctl(fd, SNDCTL_DSP_SYNC, 0);
    phase("petites", 2.0, 256, &h3, &r3);
    verifie("H3a ODELAY honnete par ecritures de 256 trames", h3);
    verifie("H3b horloge au rythme reel par ecritures de 256 trames", r3 >= 0.80 && r3 <= 1.05);

    double t = maintenant();
    int s = ioctl(fd, SNDCTL_DSP_SYNC, 0);
    long apres = odelay_trames();
    printf("oss-horloge-probe : SYNC=%d en %d ms, ODELAY apres=%ld\n", s, (int)((maintenant() - t) * 1000), apres);
    verifie("H4 SYNC rend quand tout est joue (ODELAY = 0)", s == 0 && apres == 0);

    // H5 : le moteur s'est arrete ; une ecriture courte doit repartir et se jouer.
    usleep(200000);
    ecrit_tout(1000);
    long juste_apres = odelay_trames();
    usleep(300000);
    long plus_tard = odelay_trames();
    printf("oss-horloge-probe : relance ODELAY=%ld puis %ld\n", juste_apres, plus_tard);
    verifie("H5 relance apres arret : 0 < ODELAY <= 1000, puis 0", juste_apres > 0 && juste_apres <= 1000 && plus_tard == 0);

    close(fd);
    if (echecs == 0)
        printf("OSS_HORLOGE_OK\n");
    else
        printf("OSS_HORLOGE_ECHEC n=%d\n", echecs);
    return echecs == 0 ? 0 : 1;
}
