/*
 * Sonde : creer et supprimer beaucoup plus de fichiers qu'il n'y a de nœuds
 * dans le RAMFS, sans que le noyau le prenne pour un debordement.
 *
 * BOUCHAUD_RAMFS_COMPTE_NOEUDS_V1
 *
 * Le RAMFS a 4 096 nœuds. Le miroir `USED_NODES_RELAXED` etait incremente a
 * chaque allocation mais jamais decremente par unlink / rmdir / renommage qui
 * ecrase : apres ~4 096 creations CUMULEES, l'assertion d'allocation faisait
 * paniquer le noyau (os-primitives 38442948204, persist-course-probe).
 *
 * Ici, en boucle et avec au plus quelques fichiers vivants a la fois :
 *   - 6 000 creat + unlink ;
 *   - 1 000 mkdir + rmdir ;
 *   - 1 000 renommages qui ecrasent une cible existante.
 *
 * Sortie : NOEUDS_CHURN_OK cycles=8000  ou  NOEUDS_CHURN_ECHEC etape=... i=...
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int echec(const char *etape, int i)
{
    printf("NOEUDS_CHURN_ECHEC etape=%s i=%d errno=%d (%s)\n", etape, i, errno, strerror(errno));
    return 1;
}

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    const char *f = "/tmp/noeuds-churn-f";
    const char *g = "/tmp/noeuds-churn-g";
    const char *d = "/tmp/noeuds-churn-d";
    unlink(f);
    unlink(g);
    rmdir(d);
    int cycles = 0;
    for (int i = 0; i < 6000; i++, cycles++) {
        int fd = open(f, O_CREAT | O_WRONLY | O_TRUNC, 0644);
        if (fd < 0)
            return echec("creat", i);
        if (write(fd, "x", 1) != 1)
            return echec("write", i);
        close(fd);
        if (unlink(f) != 0)
            return echec("unlink", i);
    }
    for (int i = 0; i < 1000; i++, cycles++) {
        if (mkdir(d, 0755) != 0)
            return echec("mkdir", i);
        if (rmdir(d) != 0)
            return echec("rmdir", i);
    }
    for (int i = 0; i < 1000; i++, cycles++) {
        int a = open(f, O_CREAT | O_WRONLY | O_TRUNC, 0644);
        int b = open(g, O_CREAT | O_WRONLY | O_TRUNC, 0644);
        if (a < 0 || b < 0)
            return echec("creat2", i);
        close(a);
        close(b);
        /* Ecrase g : son nœud doit etre rendu. */
        if (rename(f, g) != 0)
            return echec("rename", i);
        if (unlink(g) != 0)
            return echec("unlink2", i);
    }
    printf("NOEUDS_CHURN_OK cycles=%d\n", cycles);
    return 0;
}
