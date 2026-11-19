#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <signal.h>
#include <sys/wait.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <unistd.h>

#define VERSION_VENUS _IOR('B', 0xaa, uint32_t)
#define DEMAPPE _IO('B', 0xab)
struct requete { uint32_t octets, version; };
#define VERSION_TAILLE _IOWR('B', 0xac, struct requete)

// Linux uapi drm/virtgpu_drm.h, ioctl GETPARAM et GET_CAPS.
struct drm_param { uint64_t param, valeur; };
struct drm_caps { uint32_t id, version; uint64_t adr; uint32_t taille, pad; };
#define DRM_GETPARAM _IOWR('d', 0x43, struct drm_param)
#define DRM_GET_CAPS _IOWR('d', 0x49, struct drm_caps)
struct drm_context_init { uint32_t n, pad; uint64_t adr; };
struct drm_context_param { uint64_t cle, valeur; };
#define DRM_CONTEXT_INIT _IOWR('d', 0x4b, struct drm_context_init)

static int verifie_capset(int fd)
{
    uint64_t masque = 0;
    struct drm_param p = {7, (uintptr_t)&masque};
    if (ioctl(fd, DRM_GETPARAM, &p) || !(masque & (1ull << 4))) return 1;
    uint32_t octets[40] = {0};
    struct drm_caps c = {4, 1, (uintptr_t)octets, sizeof(octets), 0};
    if (!ioctl(fd, DRM_GET_CAPS, &c) || errno != EINVAL) return 2;
    c.version = 0;
    if (ioctl(fd, DRM_GET_CAPS, &c) || octets[0] != 1 || octets[4] != 1) return 3;
    p.param = 6;
    if (ioctl(fd, DRM_GETPARAM, &p) || masque != 1) return 4;
    puts("VIRTGPU_VENUS_DRM_CAPS_OK");
    return 0;
}

static int verifie_contexte(int fd)
{
    struct drm_context_param params[] = {{1, 4}, {2, 2}};
    struct drm_context_init init = {2, 0, (uintptr_t)params};
    if (!ioctl(fd, DRM_CONTEXT_INIT, &init) || errno != EINVAL) return 1;
    init.adr = 1;
    if (!ioctl(fd, DRM_CONTEXT_INIT, &init) || errno != EFAULT) return 2;
    init.adr = (uintptr_t)params;
    params[1].valeur = 1;
    if (ioctl(fd, DRM_CONTEXT_INIT, &init)) return 3;
    if (!ioctl(fd, DRM_CONTEXT_INIT, &init) || errno != EEXIST) return 4;
    puts("VIRTGPU_VENUS_DRM_CONTEXT_OK");
    return 0;
}

struct drm_execbuffer {
    uint32_t flags, taille;
    uint64_t commande, bo_handles;
    uint32_t n_bo;
    int32_t fence;
    uint32_t anneau, sync_stride, n_in, n_out;
    uint64_t in, out;
};
#define DRM_EXECBUFFER _IOWR('d', 0x42, struct drm_execbuffer)
typedef uint32_t reponse_venus[128];
#define LIT_REPONSE _IOR('B', 0xad, reponse_venus)

static int execute(int fd, uint32_t *flux, uint32_t octets, reponse_venus reponse)
{
    struct drm_execbuffer req = {.flags = 4, .taille = octets, .commande = (uintptr_t)flux, .fence = -1};
    return ioctl(fd, DRM_EXECBUFFER, &req) || ioctl(fd, LIT_REPONSE, reponse);
}

static int remplit_buffer(int fd, uint32_t appareil, uint32_t device,
                          uint32_t buffer, uint32_t besoin, uint32_t types)
{
    reponse_venus r;
    uint32_t proprietes[] = {179, 0, 0, 0, 8, 1, appareil, 0, 1, 0, 32, 0, 16, 0};
    if (execute(fd, proprietes, sizeof(proprietes), r) || r[0] != 8 || r[1] != 1
        || r[2] || r[3] > 32 || r[4] != 32 || r[5])
        return 1;
    uint32_t type = 0;
    while (type < r[3] && (!(types & (1u << type)) || (r[6 + 2 * type] & 6) != 6))
        ++type;
    if (type == r[3]) return 2;

    enum { MEMOIRE = 0x401, QUEUE = 0x402, POOL = 0x403, COMMANDE = 0x404, FENCE = 0x405 };
    // VkMemoryAllocateInfo, mémoire hôte visible et cohérente.
    uint32_t alloue[] = {
        179, 0, 0, 0, 21, 1, device, 0,
        1, 0, 5, 0, 0, besoin, 0, type, 0, 0, 1, 0, MEMOIRE, 0
    };
    if (execute(fd, alloue, sizeof(alloue), r) || r[0] != 21 || r[1] || r[4] != MEMOIRE)
        return 3;
    uint32_t lie[] = {179, 0, 0, 0, 28, 1, device, 0, buffer, 0, MEMOIRE, 0, 0, 0};
    if (execute(fd, lie, sizeof(lie), r) || r[0] != 28 || r[1]) return 4;
    // GetDeviceQueue2 exige VkDeviceQueueTimelineInfoMESA sur Venus.
    uint32_t queue[] = {
        179, 0, 0, 0, 155, 1, device, 0,
        1, 0, 1000145003, 1, 0, 1000384005, 0, 0, 1, 0, 0, 0,
        1, 0, QUEUE, 0
    };
    if (execute(fd, queue, sizeof(queue), r) || r[0] != 155 || r[1] != 1
        || r[2] || r[3] != QUEUE) return 5;
    uint32_t pool[] = {
        179, 0, 0, 0, 85, 1, device, 0,
        1, 0, 39, 0, 0, 0, 0, 0, 0, 1, 0, POOL, 0
    };
    if (execute(fd, pool, sizeof(pool), r) || r[0] != 85 || r[1] || r[4] != POOL)
        return 6;
    uint32_t commande[] = {
        179, 0, 0, 0, 88, 1, device, 0,
        1, 0, 40, 0, 0, POOL, 0, 0, 1, 1, 0, COMMANDE, 0
    };
    if (execute(fd, commande, sizeof(commande), r) || r[0] != 88 || r[1]
        || r[2] != 1 || r[3] || r[4] != COMMANDE) return 7;
    uint32_t debut[] = {
        179, 0, 0, 0, 90, 1, COMMANDE, 0, 1, 0, 42, 0, 0, 1, 0, 0
    };
    if (execute(fd, debut, sizeof(debut), r) || r[0] != 90 || r[1]) return 8;
    uint32_t remplissage[] = {
        179, 0, 0, 0, 118, 1, COMMANDE, 0, buffer, 0, 0, 0, 4096, 0, 0x5a1740d3
    };
    if (execute(fd, remplissage, sizeof(remplissage), r) || r[0] != 118) return 9;
    uint32_t fin[] = {179, 0, 0, 0, 91, 1, COMMANDE, 0};
    if (execute(fd, fin, sizeof(fin), r) || r[0] != 91 || r[1]) return 10;
    uint32_t cloture[] = {
        179, 0, 0, 0, 35, 1, device, 0, 1, 0, 8, 0, 0, 0, 0, 0, 1, 0, FENCE, 0
    };
    if (execute(fd, cloture, sizeof(cloture), r) || r[0] != 35 || r[1] || r[4] != FENCE)
        return 11;
    uint32_t soumet[] = {
        179, 0, 0, 0, 18, 1, QUEUE, 0, 1, 1, 0,
        4, 0, 0, 0, 0, 0, 0, 0,           // VkSubmitInfo sans attente
        1, 1, 0, COMMANDE, 0, 0, 0, 0, FENCE, 0
    };
    if (execute(fd, soumet, sizeof(soumet), r) || r[0] != 18 || r[1]) return 12;
    uint32_t attends[] = {
        179, 0, 0, 0, 39, 1, device, 0, 1, 1, 0, FENCE, 0, 1, 100000000, 0
    };
    if (execute(fd, attends, sizeof(attends), r) || r[0] != 39 || r[1]) return 13;
    puts("VIRTGPU_VENUS_VKFILL_FENCE_OK octets=4096");
    uint32_t detruit_fence[] = {179, 0, 0, 0, 36, 1, device, 0, FENCE, 0, 0, 0};
    if (execute(fd, detruit_fence, sizeof(detruit_fence), r) || r[0] != 36) return 14;
    uint32_t detruit_pool[] = {179, 0, 0, 0, 86, 1, device, 0, POOL, 0, 0, 0};
    if (execute(fd, detruit_pool, sizeof(detruit_pool), r) || r[0] != 86) return 15;
    // L'appelant détruit le buffer lié avant de libérer MEMOIRE.
    return 0;
}

static int verifie_buffer(int fd, uint32_t appareil)
{
    reponse_venus r;
    // Première famille du renderer de banc. Vérifier sa capacité avant
    // d'utiliser l'index zéro pour créer une queue de transfert.
    uint32_t familles[] = {179, 0, 0, 0, 7, 1, appareil, 0, 1, 0, 1, 1, 0};
    if (execute(fd, familles, sizeof(familles), r) || r[0] != 7
        || r[1] != 1 || r[2] || r[3] != 1 || r[4] != 1 || r[5]
        || !(r[6] & 7) || r[7] == 0)
        return 1;
    const uint32_t device = 0x200;
    const uint32_t buffer = 0x300;
    uint32_t cree_device[] = {
        179, 0, 0, 0, 11, 1, appareil, 0,
        1, 0, 3, 0, 0, 0,            // VkDeviceCreateInfo
        1, 1, 0,                    // un VkDeviceQueueCreateInfo
        2, 0, 0, 0, 0, 1, 1, 0, 0x3f800000,
        0, 0, 0, 0, 0, 0, 0, 0,     // ni layers, extensions, ni features
        0, 0, 1, 0, device, 0
    };
    if (execute(fd, cree_device, sizeof(cree_device), r) || r[0] != 11
        || r[1] || r[2] != 1 || r[3] || r[4] != device || r[5])
        return 2;
    uint32_t cree_buffer[] = {
        179, 0, 0, 0, 50, 1, device, 0,
        1, 0, 12, 0, 0, 0,          // VkBufferCreateInfo
        4096, 0, 2, 0, 0, 0, 0,     // taille, TRANSFER_DST, EXCLUSIVE
        0, 0, 1, 0, buffer, 0
    };
    if (execute(fd, cree_buffer, sizeof(cree_buffer), r) || r[0] != 50
        || r[1] || r[2] != 1 || r[3] || r[4] != buffer || r[5])
        return 3;
    uint32_t besoins[] = {179, 0, 0, 0, 30, 1, device, 0, buffer, 0, 1, 0};
    if (execute(fd, besoins, sizeof(besoins), r) || r[0] != 30 || r[1] != 1
        || r[2] || r[3] < 4096 || r[4] || !r[5] || r[6] || !r[7])
        return 4;
    int fill = remplit_buffer(fd, appareil, device, buffer, r[3], r[7]);
    if (fill) {
        printf("VIRTGPU_VENUS_VKFILL_ECHEC etape=%d\n", fill);
        return 7;
    }
    uint32_t detruit_buffer[] = {179, 0, 0, 0, 51, 1, device, 0, buffer, 0, 0, 0};
    if (execute(fd, detruit_buffer, sizeof(detruit_buffer), r) || r[0] != 51) return 5;
    uint32_t libere[] = {179, 0, 0, 0, 22, 1, device, 0, 0x401, 0, 0, 0};
    if (execute(fd, libere, sizeof(libere), r) || r[0] != 22) return 8;
    uint32_t detruit_device[] = {179, 0, 0, 0, 12, 1, device, 0, 0, 0};
    if (execute(fd, detruit_device, sizeof(detruit_device), r) || r[0] != 12) return 6;
    puts("VIRTGPU_VENUS_VKBUFFER_OK octets=4096");
    return 0;
}

static int verifie_instances(int fd)
{
    struct drm_execbuffer req = {.flags = 4, .taille = 4, .commande = 1, .fence = -1};
    if (!ioctl(fd, DRM_EXECBUFFER, &req) || errno != EFAULT) return 1;
    req.anneau = 1;
    if (!ioctl(fd, DRM_EXECBUFFER, &req) || errno != EINVAL) return 2;
    for (uint32_t handle = 1; handle <= 2; ++handle) {
        // vkSetReplyCommandStreamMESA puis vkCreateInstance. Les champs
        // Venus (dont les pointeurs de présence 64 bits) sont sérialisés
        // par l'application, indépendamment du noyau.
        uint32_t creation[] = {
            178, 0, 1, 0, 0xb107, 0, 0, sizeof(reponse_venus), 0,
            0, 1, 1, 0, 1, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 1, 0, handle, 0
        };
        reponse_venus r;
        if (execute(fd, creation, sizeof(creation), r)) {
            perror("VENUS_CREATE_INSTANCE");
            return 3;
        }
        if (r[0] || r[1] || r[2] != 1 || r[3] || r[4] != handle || r[5]) {
            printf("VENUS_CREATE_REPLY %x %x %x %x %x %x\n", r[0], r[1], r[2], r[3], r[4], r[5]);
            return 4;
        }
        // vkEnumeratePhysicalDevices, d'abord par cardinalité, puis
        // avec un emplacement d'identifiant alloué par le client.
        uint32_t liste[] = {179, 0, 0, 0, 2, 1, handle, 0, 1, 0, 0, 0, 0};
        if (execute(fd, liste, sizeof(liste), r) || r[0] != 2 || r[1]
            || r[2] != 1 || r[3] || !r[4] || r[5] || r[6])
            return 6;
        uint32_t total = r[4];
        uint32_t appareil = 0x100 + handle;
        uint32_t identifiants[] = {
            179, 0, 0, 0, 2, 1, handle, 0, 1, 0, 1, 1, 0, appareil, 0
        };
        if (execute(fd, identifiants, sizeof(identifiants), r) || r[0] != 2
            || r[1] != (total > 1 ? 5 : 0) || r[2] != 1 || r[3] || r[4] != 1
            || r[5] != 1 || r[6] || r[7] != appareil || r[8])
            return 7;
        printf("VIRTGPU_VENUS_APPAREILS compte=%" PRIu32 " handle=%" PRIu32 "\n", total, appareil);
        int buffer = verifie_buffer(fd, appareil);
        if (buffer) {
            printf("VIRTGPU_VENUS_VKBUFFER_ECHEC etape=%d\n", buffer);
            return 8;
        }
        // Réinitialiser le flux de réponse afin que le premier mot
        // appartienne bien à cette commande après la clôture.
        uint32_t destruction[] = {179, 0, 0, 0, 1, 1, handle, 0, 0, 0};
        if (execute(fd, destruction, sizeof(destruction), r) || r[0] != 1) return 5;
    }
    puts("VIRTGPU_VENUS_DRM_INSTANCES_OK cycles=2");
    return 0;
}

static int verifie_proprietaire(int fd, uint32_t reference)
{
    int notification[2];
    if (pipe(notification)) return 1;
    pid_t enfant = fork();
    if (enfant < 0) return 2;
    if (!enfant) {
        close(fd);
        close(notification[0]);
        int rendu = open("/dev/dri/renderD128", O_RDWR);
        struct requete q = {65536, 0};
        if (rendu < 0 || ioctl(rendu, VERSION_TAILLE, &q) || q.version != reference)
            _exit(1);
        if (write(notification[1], "!", 1) != 1) _exit(2);
        for (;;) pause();
    }
    close(notification[1]);
    char octet;
    ssize_t lu = read(notification[0], &octet, 1);
    close(notification[0]);
    struct requete q = {4096, 0};
    int isole = lu == 1 && ioctl(fd, VERSION_TAILLE, &q) == -1 && errno == EBUSY
                          && ioctl(fd, DEMAPPE) == -1 && errno == EBUSY;
    int statut;
    if (kill(enfant, SIGKILL) || waitpid(enfant, &statut, 0) != enfant
        || !WIFSIGNALED(statut) || WTERMSIG(statut) != SIGKILL || !isole)
        return 3;
    if (ioctl(fd, VERSION_TAILLE, &q) || q.version != reference || ioctl(fd, DEMAPPE))
        return 4;
    puts("VIRTGPU_VENUS_PROPRIETAIRE_OK");
    return 0;
}

int main(int argc, char **argv)
{
    (void)argv;
    int fd = open("/dev/dri/renderD128", O_RDWR);
    if (fd < 0 || verifie_capset(fd) || verifie_contexte(fd)) return 1;
    if (!ioctl(fd, VERSION_VENUS, (void *)1) || errno != EFAULT) return 2;
    uint32_t reference = 0;
    for (unsigned i = 0; i < 3; ++i) {
        uint32_t version = 0;
        if (ioctl(fd, VERSION_VENUS, &version)) {
            perror("VERSION_VENUS");
            return 3;
        }
        if (version >> 22 != 1 || (i && version != reference)) return 4;
        reference = version;
    }
    printf("VIRTGPU_VENUS_VERSION_RING3_OK appels=3 api=%" PRIu32 ".%" PRIu32 ".%" PRIu32 "\n",
           reference >> 22, (reference >> 12) & 0x3ff, reference & 0xfff);
    if (argc > 1) {
        if (verifie_instances(fd)) return 16;
        if (ioctl(fd, DEMAPPE)) {
            perror("VENUS_DEMAPPE");
            return 6;
        }
        uint32_t version = 0;
        if (ioctl(fd, VERSION_VENUS, &version) || version != reference) return 7;
        if (ioctl(fd, DEMAPPE)) return 8;
        const uint32_t tailles[] = {4096, 65536, 1048576, 4096};
        for (unsigned i = 0; i < sizeof(tailles) / sizeof(tailles[0]); ++i) {
            struct requete invalide = {4097, 0};
            if (!ioctl(fd, VERSION_TAILLE, &invalide) || errno != EINVAL) return 9;
            invalide.octets = 1048576 + 4096;
            if (!ioctl(fd, VERSION_TAILLE, &invalide) || errno != EINVAL) return 10;
            struct requete valide = {tailles[i], 0};
            if (ioctl(fd, VERSION_TAILLE, &valide) || valide.version != reference) return 11;
            invalide.octets = tailles[i] == 4096 ? 65536 : 4096;
            if (!ioctl(fd, VERSION_TAILLE, &invalide) || errno != EINVAL) return 12;
            valide.version = 0;
            if (ioctl(fd, VERSION_TAILLE, &valide) || valide.version != reference) return 13;
            if (ioctl(fd, DEMAPPE)) return 14;
        }
        puts("VIRTGPU_VENUS_TAILLES_OK cycles=4");
        if (verifie_proprietaire(fd, reference)) return 15;
        puts("VIRTGPU_VENUS_DEMAPPE_OK");
    }
    return close(fd) != 0;
}
