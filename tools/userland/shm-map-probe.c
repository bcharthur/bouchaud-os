/*
 * SHM-MAP-PROBE -- BOUCHAUD_SHM_PROJECTION_V1 (ABI native 1.7).
 *
 * Une SharedRegion projetee par bo_shm_map est la MEME memoire que celle que
 * voient bo_shm_read/bo_shm_write, les autres projections et les fils :
 *
 *   ecriture_noyau   bo_shm_write puis lecture par le pointeur
 *   ecriture_ring3   ecriture par le pointeur puis bo_shm_read
 *   deux_vues        deux projections de la meme region se voient
 *   fils             apres fork, le fils ecrit par la projection : le pere voit
 *   droits           sans MAP, ou sans WRITE pour une projection inscriptible :
 *                    ACCESS_DENIED ; prot invalide : INVALID_ARGUMENT
 *   lecture_seule    ecrire dans une projection RO tue (SIGSEGV), sans rien
 *                    modifier
 *   handle_ferme     la projection survit a la fermeture du handle
 *   unmap            apres bo_shm_unmap, l'acces tue (SIGSEGV)
 *   quota            creer des regions de 32 Mio finit par NO_SPACE (quota du
 *                    createur, BOUCHAUD_SHM_QUOTA_V1) ; pendant ce temps un
 *                    AUTRE processus peut encore en creer une (sans quota, le
 *                    premier avait epuise le tas du noyau) ; les fermer rend
 *                    la charge
 *
 *     SHM_MAP_OK                 SHM_MAP_FAIL raison=...
 */
#define _GNU_SOURCE
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
#include <bouchaud/sdk.h>

#define TAILLE (3 * 4096 + 100)

static int echec(const char *raison, long v)
{
    printf("SHM_MAP_FAIL raison=%s valeur=%ld\n", raison, v);
    return 1;
}

/* Le fils execute `acces` ; rend le signal qui l'a tue (0 s'il a survecu). */
static int signal_du_fils(volatile unsigned char *p, int ecrire)
{
    pid_t pid = fork();
    if (pid == 0) {
        if (ecrire)
            p[10] = 0x55;
        else
            (void)p[10];
        _exit(0);
    }
    int statut = 0;
    waitpid(pid, &statut, 0);
    return WIFSIGNALED(statut) ? WTERMSIG(statut) : 0;
}

int main(void)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    bo_i64 v = bo_version();
    printf("SHM_MAP_BEGIN abi=%lld.%lld\n", (long long)(v >> 16), (long long)(v & 0xffff));
    bo_i64 h = bo_shm_create(TAILLE);
    if (h < 0)
        return echec("shm_create", h);

    bo_i64 a = bo_shm_map((bo_handle_t)h, BO_SHM_PROT_READ | BO_SHM_PROT_WRITE);
    if (a < 0)
        return echec("map_rw", a);
    volatile unsigned char *p = (volatile unsigned char *)a;

    const char motif[] = "region-native";
    if (bo_shm_write((bo_handle_t)h, 4096 + 7, motif, sizeof motif) < 0)
        return echec("shm_write", 0);
    if (memcmp((const void *)(p + 4096 + 7), motif, sizeof motif) != 0)
        return echec("ecriture_noyau_invisible", 0);
    printf("SHM_MAP_ETAPE ecriture_noyau=ok\n");

    p[3 * 4096 + 50] = 0xA7;
    unsigned char lu = 0;
    if (bo_shm_read((bo_handle_t)h, 3 * 4096 + 50, &lu, 1) != 1 || lu != 0xA7)
        return echec("ecriture_ring3_invisible", lu);
    printf("SHM_MAP_ETAPE ecriture_ring3=ok\n");

    bo_i64 b = bo_shm_map((bo_handle_t)h, BO_SHM_PROT_READ);
    if (b < 0 || b == a)
        return echec("map_ro", b);
    volatile unsigned char *q = (volatile unsigned char *)b;
    p[2] = 0x42;
    if (q[2] != 0x42)
        return echec("deux_vues_divergent", q[2]);
    printf("SHM_MAP_ETAPE deux_vues=ok\n");

    pid_t pid = fork();
    if (pid == 0) {
        p[5] = 0x99;
        _exit(0);
    }
    int statut = 0;
    waitpid(pid, &statut, 0);
    if (!WIFEXITED(statut) || p[5] != 0x99)
        return echec("fils_invisible", p[5]);
    printf("SHM_MAP_ETAPE fils=ok\n");

    bo_i64 sans_map = bo_handle_dup((bo_handle_t)h, BO_RIGHT_READ | BO_RIGHT_WRITE | BO_RIGHT_INSPECT);
    bo_i64 sans_ecriture = bo_handle_dup((bo_handle_t)h, BO_RIGHT_READ | BO_RIGHT_MAP);
    if (sans_map < 0 || sans_ecriture < 0)
        return echec("dup", sans_map < 0 ? sans_map : sans_ecriture);
    if (bo_shm_map((bo_handle_t)sans_map, BO_SHM_PROT_READ) != -BO_E_ACCESS_DENIED)
        return echec("map_sans_droit_map", 0);
    if (bo_shm_map((bo_handle_t)sans_ecriture, BO_SHM_PROT_READ | BO_SHM_PROT_WRITE) != -BO_E_ACCESS_DENIED)
        return echec("map_rw_sans_droit_write", 0);
    if (bo_shm_map((bo_handle_t)h, 0) >= 0 || bo_shm_map((bo_handle_t)h, 4) >= 0)
        return echec("prot_invalide_acceptee", 0);
    printf("SHM_MAP_ETAPE droits=ok\n");

    int sig = signal_du_fils(q, 1);
    if (sig != SIGSEGV)
        return echec("ecriture_dans_projection_ro", sig);
    if (q[10] == 0x55)
        return echec("projection_ro_modifiee", q[10]);
    printf("SHM_MAP_ETAPE lecture_seule=ok\n");

    bo_handle_close((bo_handle_t)h);
    bo_handle_close((bo_handle_t)sans_map);
    bo_handle_close((bo_handle_t)sans_ecriture);
    p[100] = 0x3C;
    if (q[100] != 0x3C)
        return echec("handle_ferme", q[100]);
    printf("SHM_MAP_ETAPE handle_ferme=ok\n");

    if (bo_shm_unmap((void *)b) != 0)
        return echec("unmap", 0);
    sig = signal_du_fils(q, 0);
    if (sig != SIGSEGV)
        return echec("acces_apres_unmap", sig);
    if (p[100] != 0x3C)
        return echec("autre_vue_apres_unmap", p[100]);
    if (bo_shm_unmap((void *)b) == 0)
        return echec("double_unmap_accepte", 0);
    printf("SHM_MAP_ETAPE unmap=ok\n");

    bo_i64 tas[64];
    int n = 0;
    bo_i64 refus = 0;
    for (; n < 64; n++) {
        tas[n] = bo_shm_create(32u << 20);
        if (tas[n] < 0) { refus = tas[n]; break; }
    }
    printf("SHM_MAP_QUOTA regions_32mio=%d refus=%lld\n", n, (long long)refus);
    if (n == 64 || refus != -BO_E_NO_SPACE)
        return echec("quota_absent", refus);
    pid_t autre = fork();
    if (autre == 0)
        _exit(bo_shm_create(32u << 20) > 0 ? 0 : 1);
    int st = 0;
    waitpid(autre, &st, 0);
    if (!WIFEXITED(st) || WEXITSTATUS(st) != 0)
        return echec("quota_un_processus_affame_les_autres", n);
    for (int i = 0; i < n; i++)
        bo_handle_close((bo_handle_t)tas[i]);
    bo_i64 apres = bo_shm_create(32u << 20);
    if (apres < 0)
        return echec("quota_non_rendu", apres);
    bo_handle_close((bo_handle_t)apres);
    printf("SHM_MAP_ETAPE quota=ok\n");
    printf("SHM_MAP_OK\n");
    return 0;
}
