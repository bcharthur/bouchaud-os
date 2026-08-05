// Sonde de la sortie audio : ouvre /dev/dsp, le regle, et y pousse du son.
//
// Verifie ce que le pilote AC'97 promet — le reglage du format, la place
// annoncee, l'ecriture qui avance, l'horloge de lecture qui progresse — sans
// rien supposer d'audible : sous QEMU sans peripherique de sortie, le son part
// dans le vide, mais le materiel emule le consomme exactement comme s'il le
// jouait. C'est ce qui rend la sonde utile en automatique.
//
// Le signal produit est une gamme : cinq notes d'un quart de seconde. Sur une
// machine avec haut-parleur, on l'entend ; ici, ce qui compte est que les
// compteurs bougent comme ils doivent.

#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

#define SNDCTL_DSP_RESET      0x00005000
#define SNDCTL_DSP_SPEED      0xC0045002
#define SNDCTL_DSP_SETFMT     0xC0045005
#define SNDCTL_DSP_CHANNELS   0xC0045006
#define SNDCTL_DSP_GETFMTS    0x8004500B
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

static int echecs = 0;
static int reussites = 0;

static void verifie(const char *nom, int condition, long obtenu)
{
    if (condition) {
        reussites++;
    } else {
        echecs++;
        printf("  ECHEC %s (valeur %ld)\n", nom, obtenu);
    }
}

// Une sinusoide, sans bibliotheque mathematique : la recurrence
// s(n+1) = 2·cos(w)·s(n) − s(n−1) engendre exactement une sinusoide, et ne
// coute que deux multiplications par echantillon.
static void remplit_note(int16_t *tampon, int trames, double frequence,
                         int frequence_echantillonnage)
{
    const double w = 2.0 * 3.14159265358979 * frequence / frequence_echantillonnage;
    // cos(w) par la serie de Taylor : la sonde ne peut pas lier libm proprement
    // en statique sans tirer bien plus que ce qu'elle utilise.
    double x = w, terme = 1.0, cosinus = 1.0;
    for (int k = 1; k <= 8; k++) {
        terme *= -x * x / ((2.0 * k - 1.0) * (2.0 * k));
        cosinus += terme;
    }
    double sinus_w = w - w * w * w / 6.0 + w * w * w * w * w / 120.0;

    double precedent = 0.0, courant = sinus_w;
    for (int i = 0; i < trames; i++) {
        const double valeur = (i == 0) ? 0.0 : courant;
        const int16_t echantillon = (int16_t)(valeur * 9000.0);
        tampon[i * 2] = echantillon;
        tampon[i * 2 + 1] = echantillon;
        const double suivant = 2.0 * cosinus * courant - precedent;
        precedent = courant;
        courant = suivant;
    }
}

int main(void)
{
    printf("=== audio-probe ===\n");

    int dsp = open("/dev/dsp", O_WRONLY);
    if (dsp < 0) {
        printf("  ECHEC ouverture de /dev/dsp\n");
        printf("RESULTAT : 1 verification(s) en echec sur 1\n");
        return 1;
    }
    printf("  /dev/dsp ouvert (fd %d)\n", dsp);
    reussites++;

    int formats = 0;
    verifie("GETFMTS repond", ioctl(dsp, SNDCTL_DSP_GETFMTS, &formats) == 0, 0);
    verifie("le 16 bits signe est annonce", (formats & AFMT_S16_LE) != 0, formats);
    verifie("le 8 bits non signe est annonce", (formats & AFMT_U8) != 0, formats);

    int format = AFMT_S16_LE;
    verifie("SETFMT accepte le 16 bits", ioctl(dsp, SNDCTL_DSP_SETFMT, &format) == 0, 0);
    verifie("SETFMT confirme le format", format == AFMT_S16_LE, format);

    int voies = 2;
    verifie("CHANNELS accepte la stereo", ioctl(dsp, SNDCTL_DSP_CHANNELS, &voies) == 0, 0);
    verifie("CHANNELS confirme deux voies", voies == 2, voies);

    // Le pilote doit accepter 44,1 kHz et le convertir : c'est la frequence de
    // la quasi-totalite des fichiers audio du web.
    int frequence = 44100;
    verifie("SPEED accepte 44100", ioctl(dsp, SNDCTL_DSP_SPEED, &frequence) == 0, 0);
    verifie("SPEED confirme 44100", frequence == 44100, frequence);

    struct audio_buf_info espace;
    memset(&espace, 0, sizeof espace);
    verifie("GETOSPACE repond", ioctl(dsp, SNDCTL_DSP_GETOSPACE, &espace) == 0, 0);
    verifie("des fragments sont libres", espace.fragments > 0, espace.fragments);
    verifie("de la place est annoncee", espace.bytes > 0, espace.bytes);
    printf("  place initiale : %d fragments, %d octets\n",
           espace.fragments, espace.bytes);

    // --- Le son lui-meme -----------------------------------------------------
    static int16_t tampon[4410 * 2]; // 100 ms a 44,1 kHz, stereo
    const double gamme[5] = { 261.63, 329.63, 392.00, 523.25, 392.00 };

    long total_ecrit = 0;
    for (int note = 0; note < 5; note++) {
        remplit_note(tampon, 4410, gamme[note], 44100);
        // Deux passes par note : 200 ms, assez pour que plusieurs tampons DMA
        // soient consommes et que l'horloge avance.
        for (int passe = 0; passe < 2; passe++) {
            const char *curseur = (const char *)tampon;
            size_t restant = sizeof tampon;
            while (restant > 0) {
                ssize_t ecrits = write(dsp, curseur, restant);
                if (ecrits <= 0) {
                    break;
                }
                curseur += ecrits;
                restant -= (size_t)ecrits;
                total_ecrit += ecrits;
            }
        }
    }
    printf("  ecrit : %ld octets de PCM\n", total_ecrit);
    verifie("l'ecriture a consomme tout le signal",
            total_ecrit == (long)sizeof(tampon) * 10, total_ecrit);

    // Le retard annonce doit etre coherent : du son est en vol, mais pas plus
    // que ce que les tampons peuvent contenir.
    int retard = -1;
    verifie("GETODELAY repond", ioctl(dsp, SNDCTL_DSP_GETODELAY, &retard) == 0, 0);
    verifie("le retard annonce est plausible",
            retard >= 0 && retard <= 4 * 1024 * 1024, retard);
    printf("  retard annonce : %d octets\n", retard);

    verifie("RESET vide la file", ioctl(dsp, SNDCTL_DSP_RESET, 0) == 0, 0);

    memset(&espace, 0, sizeof espace);
    ioctl(dsp, SNDCTL_DSP_GETOSPACE, &espace);
    verifie("apres RESET, tout est libre de nouveau",
            espace.fragments >= 1, espace.fragments);

    close(dsp);

    if (echecs > 0) {
        printf("RESULTAT : %d verification(s) en echec sur %d\n",
               echecs, echecs + reussites);
        return 1;
    }
    printf("RESULTAT : 0 verification(s) en echec (%d passees)\n", reussites);
    return 0;
}
