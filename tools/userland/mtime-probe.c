/*
 * Sonde : la date de modification d'un fichier est SA date, pas l'heure du stat.
 *
 * BOUCHAUD_MTIME_STABLE_V1
 *
 * `stat` rendait l'heure courante comme st_mtime de tout fichier et de tout
 * repertoire. fontconfig valide ses caches en comparant la date du
 * repertoire de polices : aucun cache n'etait valide, et toutes les 30 s il
 * rechargeait sa configuration sous les objets que Skia tenait encore --
 * WebContent en faute dans FcValueCanonicalize (cr2=0xffff413f021ee480, WPT,
 * run 37622716750). Verifie :
 *   1. un repertoire non modifie garde la meme st_mtime a 2 s d'intervalle
 *      (stat, fstat, statx) ;
 *   2. y creer une entree fait avancer sa st_mtime ;
 *   3. ecrire dans un fichier fait avancer sa st_mtime, pas celle d'un autre.
 * Sortie : une ligne par verification, puis MTIME_STABLE_OK ou
 * MTIME_STABLE_ECHEC n=...
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

static int echecs;

static void verifie(const char *quoi, int condition, long avant, long apres)
{
    printf("  %-58s %s (%ld -> %ld)\n", quoi, condition ? "ok" : "ECHEC", avant, apres);
    if (!condition)
        echecs++;
}

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    const char *dossier = "/tmp/mtime-probe";
    mkdir(dossier, 0755);
    struct stat a, b;
    stat(dossier, &a);
    sleep(2);
    stat(dossier, &b);
    verifie("repertoire intact : meme st_mtime a 2 s (stat)", a.st_mtime == b.st_mtime, a.st_mtime, b.st_mtime);

    int fd = open(dossier, O_RDONLY | O_DIRECTORY);
    struct stat f;
    fstat(fd, &f);
    close(fd);
    verifie("fstat du meme repertoire : meme st_mtime", f.st_mtime == a.st_mtime, a.st_mtime, f.st_mtime);

    // statx brut (musl 1.2.4 n'a ni l'enveloppe ni la structure) :
    // stx_mtime.tv_sec est a l'octet 112 d'un tampon de 256.
    unsigned char x[256];
    memset(x, 0, sizeof x);
    long rx = syscall(332, AT_FDCWD, dossier, 0, 0x40 /* STATX_MTIME */, x);
    int64_t stx_mtime;
    memcpy(&stx_mtime, x + 112, sizeof stx_mtime);
    verifie("statx : stx_mtime renseigne et egal", rx == 0 && stx_mtime == (int64_t)a.st_mtime, a.st_mtime,
        (long)stx_mtime);

    sleep(1);
    int g = open("/tmp/mtime-probe/fichier", O_CREAT | O_WRONLY | O_TRUNC, 0644);
    struct stat c;
    stat(dossier, &c);
    verifie("creer une entree fait avancer la st_mtime du repertoire", c.st_mtime > b.st_mtime, b.st_mtime, c.st_mtime);

    struct stat d1, d2, autre1, autre2;
    fstat(g, &d1);
    stat("/tmp", &autre1);
    sleep(1);
    (void)!write(g, "x", 1);
    fstat(g, &d2);
    stat("/tmp", &autre2);
    close(g);
    verifie("ecrire fait avancer la st_mtime du fichier", d2.st_mtime > d1.st_mtime, d1.st_mtime, d2.st_mtime);
    verifie("ecrire ne touche pas /tmp", autre1.st_mtime == autre2.st_mtime, autre1.st_mtime, autre2.st_mtime);

    unlink("/tmp/mtime-probe/fichier");
    rmdir(dossier);
    if (echecs == 0)
        printf("MTIME_STABLE_OK\n");
    else
        printf("MTIME_STABLE_ECHEC n=%d\n", echecs);
    return echecs != 0;
}
