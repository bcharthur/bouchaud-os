/*
 * VFS-PROBE -- BOUCHAUD_VFS_MONTAGES_V1.
 *
 *   montages   /proc/mounts liste les montages /dev et /proc
 *   vivant     /proc/meminfo suit la memoire : MemFree baisse d'au moins
 *              24 Mio apres avoir touche 32 Mio (l'instantane de demarrage,
 *              lui, ne bougeait pas)
 *   uptime     /proc/uptime avance
 *   status     /proc/self/status porte le pid de l'appelant
 *   devfs      /dev/null s'ouvre et avale, /dev/zero rend des zeros
 *   union      /proc/cpuinfo (instantane RAMFS) reste lisible sous le montage
 *   inconnu    /proc/nexiste-pas : ENOENT
 *
 * BOUCHAUD_VFS_DOSSIERS_MONTES_V1 (getdents sur les montages) :
 *   liste_proc  readdir(/proc) montre l'appelant, un fils vivant, `self`,
 *               et garde cpuinfo (union avec l'instantane RAMFS)
 *   dossier_pid /proc/<fils> est un repertoire (stat S_ISDIR, opendir) qui
 *               contient `status` ; apres wait(), il a DISPARU (ENOENT, et
 *               absent de readdir)
 *   liste_dev   readdir(/dev) contient null, readdir(/dev/input) event0
 *
 *     VFS_PROBE_OK              VFS_PROBE_FAIL raison=...
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <signal.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static char tampon[16384];

static int echec(const char *r, long v) { printf("VFS_PROBE_FAIL raison=%s valeur=%ld\n", r, v); return 1; }

static long lit(const char *chemin)
{
    int fd = open(chemin, O_RDONLY);
    if (fd < 0) return -errno;
    long n = 0, r;
    while (n < (long)sizeof tampon - 1 && (r = read(fd, tampon + n, sizeof tampon - 1 - n)) > 0) n += r;
    close(fd);
    tampon[n] = 0;
    return n;
}

static long champ_kb(const char *nom)
{
    if (lit("/proc/meminfo") <= 0) return -1;
    char *p = strstr(tampon, nom);
    return p ? atol(p + strlen(nom)) : -1;
}

/* 1 si readdir(chemin) contient `nom`, 0 sinon, -errno si opendir echoue. */
static int contient(const char *chemin, const char *nom, int *entrees)
{
    DIR *d = opendir(chemin);
    if (!d) return -errno;
    int trouve = 0, n = 0;
    struct dirent *e;
    while ((e = readdir(d))) {
        n++;
        if (!strcmp(e->d_name, nom)) trouve++;
    }
    closedir(d);
    if (entrees) *entrees = n;
    return trouve;
}

static int dossiers_montes(void)
{
    char moi[16], chemin[64];
    snprintf(moi, sizeof moi, "%d", (int)getpid());
    int tube[2];
    if (pipe(tube) != 0) return echec("pipe", errno);
    pid_t fils = fork();
    if (fils < 0) return echec("fork", errno);
    if (fils == 0) { char c; close(tube[1]); if (read(tube[0], &c, 1) < 0) _exit(1); _exit(0); }
    close(tube[0]);
    char sien[16];
    snprintf(sien, sizeof sien, "%d", (int)fils);

    int n = 0;
    int a = contient("/proc", moi, &n), b = contient("/proc", sien, NULL);
    int c = contient("/proc", "self", NULL), d = contient("/proc", "cpuinfo", NULL);
    int m = contient("/proc", "meminfo", NULL);
    printf("VFS_PROBE_LISTE_PROC entrees=%d moi=%d fils=%d self=%d cpuinfo=%d meminfo=%d\n", n, a, b, c, d, m);
    if (a != 1 || b != 1 || c != 1 || d != 1 || m != 1) return echec("liste_proc", n);
    printf("VFS_PROBE_ETAPE liste_proc=ok\n");

    snprintf(chemin, sizeof chemin, "/proc/%d", (int)fils);
    struct stat st;
    if (stat(chemin, &st) != 0 || !S_ISDIR(st.st_mode)) return echec("pid_pas_dossier", errno);
    if (contient(chemin, "status", NULL) != 1) return echec("pid_sans_status", 0);
    snprintf(chemin, sizeof chemin, "/proc/%d/status", (int)fils);
    if (lit(chemin) <= 0) return echec("pid_status", 0);
    char attendu[32];
    snprintf(attendu, sizeof attendu, "Pid:\t%d\n", (int)fils);
    if (!strstr(tampon, attendu)) return echec("pid_status_pid", fils);

    if (write(tube[1], "x", 1) != 1) return echec("tube", errno);
    close(tube[1]);
    int etat;
    if (waitpid(fils, &etat, 0) != fils) return echec("waitpid", errno);
    snprintf(chemin, sizeof chemin, "/proc/%d", (int)fils);
    errno = 0;
    if (stat(chemin, &st) == 0 || errno != ENOENT) return echec("pid_survit", errno);
    if (contient("/proc", sien, NULL) != 0) return echec("pid_liste_apres_wait", 0);
    printf("VFS_PROBE_ETAPE dossier_pid=ok\n");

    if (contient("/dev", "null", &n) != 1) return echec("liste_dev", n);
    if (contient("/dev/input", "event0", NULL) != 1) return echec("liste_dev_input", 0);
    printf("VFS_PROBE_ETAPE liste_dev=ok\n");
    return 0;
}

int main(void)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    if (lit("/proc/mounts") <= 0) return echec("mounts_illisible", 0);
    if (!strstr(tampon, " /dev ") || !strstr(tampon, " /proc "))
        return echec("montages_absents", 0);
    printf("VFS_PROBE_ETAPE montages=ok\n");

    long avant = champ_kb("MemFree:");
    size_t n = 32u << 20;
    volatile char *p = malloc(n);
    for (size_t i = 0; i < n; i += 4096) p[i] = 1;
    long apres = champ_kb("MemFree:");
    printf("VFS_PROBE_MEMFREE avant_kb=%ld apres_kb=%ld\n", avant, apres);
    if (avant <= 0 || apres <= 0 || avant - apres < 24 * 1024)
        return echec("meminfo_fige", avant - apres);
    printf("VFS_PROBE_ETAPE vivant=ok\n");

    if (lit("/proc/uptime") <= 0) return echec("uptime", 0);
    double u0 = atof(tampon);
    usleep(300000);
    lit("/proc/uptime");
    if (atof(tampon) <= u0) return echec("uptime_fige", 0);
    printf("VFS_PROBE_ETAPE uptime=ok\n");

    if (lit("/proc/self/status") <= 0) return echec("status", 0);
    char attendu[32];
    snprintf(attendu, sizeof attendu, "Pid:\t%d\n", (int)getpid());
    if (!strstr(tampon, attendu)) return echec("status_pid", getpid());
    printf("VFS_PROBE_ETAPE status=ok\n");

    int fd = open("/dev/null", O_WRONLY);
    if (fd < 0 || write(fd, "x", 1) != 1) return echec("dev_null", errno);
    close(fd);
    fd = open("/dev/zero", O_RDONLY);
    char z[8] = {1, 1, 1, 1, 1, 1, 1, 1};
    if (fd < 0 || read(fd, z, 8) != 8 || z[0] || z[7]) return echec("dev_zero", errno);
    close(fd);
    printf("VFS_PROBE_ETAPE devfs=ok\n");

    if (lit("/proc/cpuinfo") <= 0 || !strstr(tampon, "processor")) return echec("union_cpuinfo", 0);
    printf("VFS_PROBE_ETAPE union=ok\n");

    if (open("/proc/nexiste-pas", O_RDONLY) >= 0 || errno != ENOENT) return echec("inconnu", errno);
    printf("VFS_PROBE_ETAPE inconnu=ok\n");
    if (dossiers_montes()) return 1;
    printf("VFS_PROBE_OK\n");
    return 0;
}
