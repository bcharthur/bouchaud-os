/*
 * audio-concurrence-probe : plusieurs processus sur /dev/dsp et la console en
 * meme temps.
 *
 * POURQUOI CETTE SONDE
 *
 * Le pilote AC97 gardait son etat dans treize `static mut` et la console VGA
 * sa pile de captures dans un autre : seul le gros verrou du noyau les
 * serialisait, parce que `write` sur ces descripteurs et `ioctl` le prenaient.
 * Le lot B9 les place sous leurs propres verrous et libere `ioctl`. Aucune
 * sonde existante n'ouvrait /dev/dsp depuis deux processus a la fois :
 * `audio-probe` est seul sur le peripherique. Celle-ci l'est a plusieurs.
 *
 *     audio-concurrence-probe [PROCESSUS] [TOURS]
 *
 * Chaque fils, en boucle : regle frequence/voies/format par ioctl, verifie que
 * les valeurs rendues sont dans ce que le pilote sait faire, lit la place et
 * le retard, ecrit un paquet de PCM en non bloquant, et ecrit une ligne sur la
 * console. Verdict du pere : `AUDIO_CONCURRENCE_OK processus=P tours=T` si
 * tous les fils sortent a 0 ; sinon `AUDIO_CONCURRENCE_FAIL` et code 1.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <unistd.h>

#define SNDCTL_DSP_SPEED      0xC0045002
#define SNDCTL_DSP_SETFMT     0xC0045005
#define SNDCTL_DSP_CHANNELS   0xC0045006
#define SNDCTL_DSP_GETOSPACE  0x800C500C
#define SNDCTL_DSP_GETODELAY  0x80045017

#define AFMT_U8      0x00000008
#define AFMT_S16_LE  0x00000010

struct audio_buf_info {
    int fragments;
    int fragstotal;
    int fragsize;
    int bytes;
};

static int fils(int numero, int tours)
{
    int dsp = open("/dev/dsp", O_WRONLY | O_NONBLOCK);
    if (dsp < 0) {
        printf("AUDIO_CONCURRENCE_ECHEC fils=%d ouverture errno=%d\n", numero, errno);
        return 1;
    }
    static int16_t pcm[2048];
    for (int i = 0; i < 2048; i++)
        pcm[i] = (int16_t)((i * 37 * (numero + 1)) & 0x7FFF);

    for (int t = 0; t < tours; t++) {
        int vitesse = 8000 + ((t * 7919 + numero * 104729) % 88000);
        if (ioctl(dsp, SNDCTL_DSP_SPEED, &vitesse) != 0 || vitesse < 4000 || vitesse > 96000) {
            printf("AUDIO_CONCURRENCE_ECHEC fils=%d tour=%d vitesse=%d\n", numero, t, vitesse);
            return 1;
        }
        int voies = 1 + ((t + numero) & 1);
        if (ioctl(dsp, SNDCTL_DSP_CHANNELS, &voies) != 0 || (voies != 1 && voies != 2)) {
            printf("AUDIO_CONCURRENCE_ECHEC fils=%d tour=%d voies=%d\n", numero, t, voies);
            return 1;
        }
        int format = (t & 2) ? AFMT_U8 : AFMT_S16_LE;
        if (ioctl(dsp, SNDCTL_DSP_SETFMT, &format) != 0
            || (format != AFMT_U8 && format != AFMT_S16_LE)) {
            printf("AUDIO_CONCURRENCE_ECHEC fils=%d tour=%d format=%#x\n", numero, t, format);
            return 1;
        }
        struct audio_buf_info place;
        memset(&place, 0, sizeof(place));
        if (ioctl(dsp, SNDCTL_DSP_GETOSPACE, &place) != 0
            || place.fragments < 0 || place.fragments > 32 || place.bytes < 0) {
            printf("AUDIO_CONCURRENCE_ECHEC fils=%d tour=%d place=%d/%d\n",
                   numero, t, place.fragments, place.bytes);
            return 1;
        }
        int retard = -1;
        if (ioctl(dsp, SNDCTL_DSP_GETODELAY, &retard) != 0 || retard < 0) {
            printf("AUDIO_CONCURRENCE_ECHEC fils=%d tour=%d retard=%d\n", numero, t, retard);
            return 1;
        }
        ssize_t n = write(dsp, pcm, sizeof(pcm));
        if (n < 0 && errno != EAGAIN) {
            printf("AUDIO_CONCURRENCE_ECHEC fils=%d tour=%d write errno=%d\n", numero, t, errno);
            return 1;
        }
        if (n > (ssize_t)sizeof(pcm)) {
            printf("AUDIO_CONCURRENCE_ECHEC fils=%d tour=%d write=%ld\n", numero, t, (long)n);
            return 1;
        }
        if (t % 50 == 0)
            printf("AUDIO_CONCURRENCE_PROGRES fils=%d tour=%d vitesse=%d voies=%d retard=%d\n",
                   numero, t, vitesse, voies, retard);
    }
    close(dsp);
    return 0;
}

int main(int argc, char **argv)
{
    int processus = argc > 1 ? atoi(argv[1]) : 4;
    int tours = argc > 2 ? atoi(argv[2]) : 400;
    if (processus < 1 || processus > 16 || tours < 1) {
        printf("AUDIO_CONCURRENCE_FAIL parametres\n");
        return 1;
    }
    pid_t pids[16];
    for (int i = 0; i < processus; i++) {
        pids[i] = fork();
        if (pids[i] < 0) {
            printf("AUDIO_CONCURRENCE_FAIL fork\n");
            return 1;
        }
        if (pids[i] == 0)
            _exit(fils(i, tours));
    }
    int echecs = 0;
    for (int i = 0; i < processus; i++) {
        int statut = 0;
        if (waitpid(pids[i], &statut, 0) != pids[i] || !WIFEXITED(statut) || WEXITSTATUS(statut) != 0)
            echecs++;
    }
    if (echecs) {
        printf("AUDIO_CONCURRENCE_FAIL echecs=%d processus=%d\n", echecs, processus);
        return 1;
    }
    printf("AUDIO_CONCURRENCE_OK processus=%d tours=%d\n", processus, tours);
    return 0;
}
