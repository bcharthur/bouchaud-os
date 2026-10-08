/*
 * Sonde : les horloges de l'invite avancent au rythme du temps reel.
 *
 * BOUCHAUD_HORLOGE_AUDIT_V1
 *
 * Sous TCG, les ticks IRQ0 retardent de 5 a 30 % sous charge (ticks_ms
 * 569524 contre mono_ms 626017, endurance 812f3941) ; sous KVM, le TSC
 * etait 4,5 fois trop rapide avant BOUCHAUD_TSC_SOURCE_V1. Ce que verra un
 * programme -- setTimeout, requestAnimationFrame, un <audio>, un TLS qui
 * juge l'expiration d'un certificat -- c'est CLOCK_MONOTONIC et
 * CLOCK_REALTIME. Pour 1, 10 et 60 s :
 *
 *   HORLOGE_DEBUT d=   puis nanosleep(d)   puis
 *   HORLOGE_FIN d= mono_us= reel_us= excedent_us= reel_ms=
 *
 * Verifie DANS l'invite (ce qu'il peut juger seul) :
 *   1. le sommeil n'est jamais plus court que demande (POSIX) ;
 *   2. il ne deborde pas de plus de 50 ms ;
 *   3. REALTIME et MONOTONIC avancent du meme pas (ecart <= 0,1 % + 2 ms) ;
 *   4. MONOTONIC ne recule jamais (2 000 000 lectures, sur tous les coeurs
 *      ou l'ordonnanceur met le fil).
 * Le juge EXTERIEUR -- le temps de l'hote entre DEBUT et FIN -- est
 * tools/ci/run_horloge.sh, qui horodate chaque ligne serie a son arrivee.
 *
 * Sortie : HORLOGE_INVITE_OK ou HORLOGE_INVITE_ECHEC n=...
 */
#define _GNU_SOURCE
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

static int echecs;

static int64_t ns(clockid_t horloge)
{
    struct timespec t;
    clock_gettime(horloge, &t);
    return (int64_t)t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void verifie(const char *quoi, int condition, long long valeur)
{
    printf("  %-62s %s (%lld)\n", quoi, condition ? "ok" : "ECHEC", valeur);
    if (!condition)
        echecs++;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    static const int durees[] = { 1, 10, 60 };
    int n = (int)(sizeof durees / sizeof durees[0]);
    // Un argument borne la serie (essai local rapide) : `horloge-probe 10`.
    int plafond = argc > 1 ? atoi(argv[1]) : 60;

    // 4. MONOTONIC ne recule jamais.
    int64_t precedent = ns(CLOCK_MONOTONIC), pire_recul = 0;
    for (int i = 0; i < 2000000; i++) {
        int64_t t = ns(CLOCK_MONOTONIC);
        if (precedent - t > pire_recul)
            pire_recul = precedent - t;
        precedent = t;
    }
    verifie("CLOCK_MONOTONIC ne recule jamais (2e6 lectures), recul ns", pire_recul == 0, pire_recul);

    for (int i = 0; i < n && durees[i] <= plafond; i++) {
        int d = durees[i];
        int64_t m0 = ns(CLOCK_MONOTONIC), r0 = ns(CLOCK_REALTIME);
        printf("HORLOGE_DEBUT d=%d\n", d);
        struct timespec demande = { d, 0 }, reste;
        while (nanosleep(&demande, &reste) != 0 && errno == EINTR)
            demande = reste;
        int64_t m1 = ns(CLOCK_MONOTONIC), r1 = ns(CLOCK_REALTIME);
        long long mono_us = (m1 - m0) / 1000, reel_us = (r1 - r0) / 1000;
        long long excedent_us = mono_us - (long long)d * 1000000LL;
        printf("HORLOGE_FIN d=%d mono_us=%lld reel_us=%lld excedent_us=%lld reel_ms=%lld\n",
               d, mono_us, reel_us, excedent_us, (long long)(r1 / 1000000));
        char quoi[96];
        snprintf(quoi, sizeof quoi, "d=%d s : le sommeil n'est pas plus court, excedent us", d);
        verifie(quoi, excedent_us >= 0, excedent_us);
        snprintf(quoi, sizeof quoi, "d=%d s : il ne deborde pas de plus de 50 ms, excedent us", d);
        verifie(quoi, excedent_us <= 50000, excedent_us);
        long long ecart = reel_us - mono_us;
        long long tolere = mono_us / 1000 + 2000;
        snprintf(quoi, sizeof quoi, "d=%d s : REALTIME suit MONOTONIC, ecart us", d);
        verifie(quoi, ecart <= tolere && -ecart <= tolere, ecart);
    }
    if (echecs == 0)
        printf("HORLOGE_INVITE_OK\n");
    else
        printf("HORLOGE_INVITE_ECHEC n=%d\n", echecs);
    return echecs ? 1 : 0;
}
