/*
 * Sonde : une ligne ecrite par un programme sort-elle entiere sur la console
 * serie pendant que le noyau ecrit les siennes depuis d'autres coeurs ?
 *
 * BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1
 *
 * Run 38482972584 (KVM, banc des services) : la ligne
 *     [LB] SERVICE_CRASH_REQUEST onglet=1 service=ImageDecoder pid=16 kill=ok ...
 * est sortie octet contre octet avec le prefixe de journal d'une ligne du
 * noyau : `pid=1\x1b6[ 90kmi[l...`. Deux coeurs ecrivaient COM1 en meme
 * temps, l'un sans le jeton serie.
 *
 * Ici, E ecrivains emettent chacun L lignes longues et VERIFIABLES (charge
 * utile derivee du numero, numero repete en fin de ligne), chaque ligne en un
 * seul write(2) ; pendant ce temps, un brasseur cree et fait mourir C
 * processus, moitie par sortie normale, moitie par faute de page : le noyau
 * ecrit alors PROCESS_EXIT / PROCESS_FAULT / FAULT_FATAL depuis d'autres
 * coeurs. Le banc relit le journal et exige les E*L lignes intactes.
 *
 * Usage : serie-entrelacement-probe [ecrivains] [lignes] [processus] [charge] [pas_ms]
 *         (defaut 3 200 120 160 0 ; charge <= 1900 octets par ligne : une ligne
 *         longue tient le jeton longtemps, comme un releve du noyau ; pas_ms :
 *         pause entre deux lignes d'un ecrivain -- des producteurs LENTS, qui
 *         ne remplissent jamais l'anneau de la console : le banc du fil gele,
 *         BOUCHAUD_CONSOLE_FIL_SECOURS_V1, ou personne d'autre que le fil ne
 *         vide)
 * Sortie : SERIE_ENTRELACEMENT_FIN ecrivains=E lignes=L processus=C
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define CHARGE_MAX 1900

static int charge_octets = 160;
static int pas_ms = 0;

static void ligne(int e, int n, char *tampon, size_t taille)
{
    char charge[CHARGE_MAX + 1];
    for (int i = 0; i < charge_octets; i++)
        charge[i] = (char)('a' + (n * 7 + e * 3 + i) % 26);
    charge[charge_octets] = 0;
    snprintf(tampon, taille, "SERIE_LIGNE e=%d n=%04d %s FIN_%04d\n", e, n, charge, n);
}

static void ecrivain(int e, int lignes)
{
    char tampon[CHARGE_MAX + 64];
    for (int n = 0; n < lignes; n++) {
        ligne(e, n, tampon, sizeof tampon);
        size_t longueur = strlen(tampon);
        if (write(1, tampon, longueur) != (ssize_t)longueur)
            _exit(2);
        if (pas_ms > 0)
            usleep((useconds_t)pas_ms * 1000u);
    }
    _exit(0);
}

static void brasseur(int processus)
{
    for (int i = 0; i < processus; i++) {
        pid_t p = fork();
        if (p == 0) {
            if (i & 1)
                *(volatile int *)(unsigned long)(0x45 + i) = 1; /* faute voulue */
            _exit(3);
        }
        if (p > 0)
            waitpid(p, NULL, 0);
    }
    _exit(0);
}

int main(int argc, char **argv)
{
    int ecrivains = argc > 1 ? atoi(argv[1]) : 3;
    int lignes = argc > 2 ? atoi(argv[2]) : 200;
    int processus = argc > 3 ? atoi(argv[3]) : 120;
    if (argc > 4)
        charge_octets = atoi(argv[4]);
    if (charge_octets < 1 || charge_octets > CHARGE_MAX)
        charge_octets = 160;
    if (argc > 5)
        pas_ms = atoi(argv[5]);
    if (pas_ms < 0 || pas_ms > 1000)
        pas_ms = 0;
    pid_t b = fork();
    if (b == 0)
        brasseur(processus);
    for (int e = 0; e < ecrivains; e++) {
        if (fork() == 0)
            ecrivain(e, lignes);
    }
    int statut, echecs = 0;
    pid_t fils;
    while ((fils = wait(&statut)) > 0) {
        if (fils != b && !(WIFEXITED(statut) && WEXITSTATUS(statut) == 0))
            echecs++;
    }
    printf("SERIE_ENTRELACEMENT_FIN ecrivains=%d lignes=%d processus=%d ecrivains_en_echec=%d\n",
           ecrivains, lignes, processus, echecs);
    return echecs ? 1 : 0;
}
