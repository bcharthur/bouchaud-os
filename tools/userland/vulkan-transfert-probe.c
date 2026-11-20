/* Standalone Vulkan loader probe. Software devices require --allow-cpu.
 * A virtual device's type does not establish the physical host renderer. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>
#include <vulkan/vulkan.h>

#define ESSAIE(appel) do { \
    VkResult statut = (appel); \
    if (statut != VK_SUCCESS) { \
        fprintf(stderr, "VULKAN_ECHEC %s statut=%d\n", #appel, statut); \
        goto fin; \
    } \
} while (0)

static int transfert(int cpu, uint32_t motif, int rendezvous)
{
    VkInstance instance = VK_NULL_HANDLE;
    VkDevice device = VK_NULL_HANDLE;
    VkPhysicalDevice physique = VK_NULL_HANDLE;
    VkPhysicalDevice *appareils = NULL;
    VkQueueFamilyProperties *familles = NULL;
    VkBuffer buffer = VK_NULL_HANDLE;
    VkDeviceMemory memoire = VK_NULL_HANDLE;
    VkCommandPool pool = VK_NULL_HANDLE;
    VkFence fence = VK_NULL_HANDLE;
    uint32_t famille = 0;
    int code = 1;
    int en_vol = 0;
    const VkDeviceSize taille = 4096;

    const VkApplicationInfo app = {
        .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO, .pApplicationName = "bouchaud-vulkan-probe",
        .apiVersion = VK_API_VERSION_1_0,
    };
    const VkInstanceCreateInfo info = {
        .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, .pApplicationInfo = &app,
    };
    ESSAIE(vkCreateInstance(&info, NULL, &instance));
    uint32_t n = 0;
    ESSAIE(vkEnumeratePhysicalDevices(instance, &n, NULL));
    if (n == 0) { code = 77; goto fin; }
    appareils = calloc(n, sizeof(*appareils));
    if (!appareils) goto fin;
    ESSAIE(vkEnumeratePhysicalDevices(instance, &n, appareils));
    for (uint32_t i = 0; i < n && !physique; ++i) {
        VkPhysicalDeviceProperties props;
        vkGetPhysicalDeviceProperties(appareils[i], &props);
        printf("VULKAN_DEVICE nom=%s type=%u vendor=%04x device=%04x\n",
               props.deviceName, props.deviceType, props.vendorID, props.deviceID);
        if (props.deviceType == VK_PHYSICAL_DEVICE_TYPE_CPU && !cpu) continue;
        uint32_t nombre = 0;
        vkGetPhysicalDeviceQueueFamilyProperties(appareils[i], &nombre, NULL);
        if (!nombre) continue;
        familles = calloc(nombre, sizeof(*familles));
        if (!familles) goto fin;
        vkGetPhysicalDeviceQueueFamilyProperties(appareils[i], &nombre, familles);
        for (uint32_t j = 0; j < nombre; ++j) {
            if (familles[j].queueCount && (familles[j].queueFlags & VK_QUEUE_GRAPHICS_BIT)) {
                famille = j;
                physique = appareils[i];
                break;
            }
        }
        free(familles);
        familles = NULL;
    }
    if (!physique) { code = 77; goto fin; }

    const float priorite = 1.0f;
    const VkDeviceQueueCreateInfo file = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO, .queueFamilyIndex = famille,
        .queueCount = 1, .pQueuePriorities = &priorite,
    };
    const VkDeviceCreateInfo dev_info = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO, .queueCreateInfoCount = 1,
        .pQueueCreateInfos = &file,
    };
    ESSAIE(vkCreateDevice(physique, &dev_info, NULL, &device));
    if (rendezvous >= 0) {
        char signal;
        if (send(rendezvous, "p", 1, MSG_NOSIGNAL) != 1
            || read(rendezvous, &signal, 1) != 1 || signal != 'g')
            goto fin;
    }
    VkQueue queue;
    vkGetDeviceQueue(device, famille, 0, &queue);
    const VkBufferCreateInfo buf_info = {
        .sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, .size = taille,
        .usage = VK_BUFFER_USAGE_TRANSFER_DST_BIT, .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
    };
    ESSAIE(vkCreateBuffer(device, &buf_info, NULL, &buffer));
    VkMemoryRequirements besoins;
    VkPhysicalDeviceMemoryProperties types;
    vkGetBufferMemoryRequirements(device, buffer, &besoins);
    vkGetPhysicalDeviceMemoryProperties(physique, &types);
    const VkMemoryPropertyFlags masque = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    uint32_t type = 0;
    for (; type < types.memoryTypeCount; ++type) {
        if ((besoins.memoryTypeBits & (UINT32_C(1) << type))
            && (types.memoryTypes[type].propertyFlags & masque) == masque) break;
    }
    if (type == types.memoryTypeCount) goto fin;
    const VkMemoryAllocateInfo alloue = {
        .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO, .allocationSize = besoins.size, .memoryTypeIndex = type,
    };
    ESSAIE(vkAllocateMemory(device, &alloue, NULL, &memoire));
    ESSAIE(vkBindBufferMemory(device, buffer, memoire, 0));
    const VkCommandPoolCreateInfo pool_info = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO, .queueFamilyIndex = famille,
    };
    ESSAIE(vkCreateCommandPool(device, &pool_info, NULL, &pool));
    const VkCommandBufferAllocateInfo cmd_info = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO, .commandPool = pool,
        .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY, .commandBufferCount = 1,
    };
    VkCommandBuffer cmd;
    ESSAIE(vkAllocateCommandBuffers(device, &cmd_info, &cmd));
    const VkCommandBufferBeginInfo debut = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO, .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
    };
    ESSAIE(vkBeginCommandBuffer(cmd, &debut));
    vkCmdFillBuffer(cmd, buffer, 0, taille, motif);
    const VkBufferMemoryBarrier barriere = {
        .sType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER,
        .srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT, .dstAccessMask = VK_ACCESS_HOST_READ_BIT,
        .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED, .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
        .buffer = buffer, .size = taille,
    };
    vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_HOST_BIT,
                         0, 0, NULL, 1, &barriere, 0, NULL);
    ESSAIE(vkEndCommandBuffer(cmd));
    const VkFenceCreateInfo fence_info = { .sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
    ESSAIE(vkCreateFence(device, &fence_info, NULL, &fence));
    const VkSubmitInfo soumission = {
        .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO, .commandBufferCount = 1, .pCommandBuffers = &cmd,
    };
    ESSAIE(vkQueueSubmit(queue, 1, &soumission, fence));
    en_vol = 1;
    ESSAIE(vkWaitForFences(device, 1, &fence, VK_TRUE, UINT64_C(5000000000)));
    en_vol = 0;
    void *map;
    ESSAIE(vkMapMemory(device, memoire, 0, taille, 0, &map));
    const uint32_t *mots = map;
    code = 0;
    for (size_t i = 0; i < taille / sizeof(*mots); ++i) {
        if (mots[i] != motif) { code = 1; break; }
    }
    vkUnmapMemory(device, memoire);
    if (!code) printf("VULKAN_TRANSFERT_OK octets=4096 verification=exacte motif=%08x\n", motif);

fin:
    /* On timeout, leave live submissions to process teardown. */
    if (en_vol) return code;
    if (fence) vkDestroyFence(device, fence, NULL);
    if (pool) vkDestroyCommandPool(device, pool, NULL);
    if (buffer) vkDestroyBuffer(device, buffer, NULL);
    if (memoire) vkFreeMemory(device, memoire, NULL);
    if (device) vkDestroyDevice(device, NULL);
    if (instance) vkDestroyInstance(instance, NULL);
    free(familles);
    free(appareils);
    return code;
}


static int parallele(const uint32_t motifs[2])
{
    int canaux[2];
    pid_t enfants[2];
    int lances = 0;
    int code = 1;
    for (int i = 0; i < 2; ++i) {
        int paire[2];
        if (socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, paire) != 0)
            goto fin;
        pid_t enfant = fork();
        if (enfant == 0) {
            close(paire[0]);
            for (int j = 0; j < i; ++j)
                close(canaux[j]);
            int verdict = transfert(1, motifs[i], paire[1]);
            fflush(NULL);
            close(paire[1]);
            _exit(verdict);
        }
        close(paire[1]);
        if (enfant < 0) {
            close(paire[0]);
            goto fin;
        }
        enfants[i] = enfant;
        canaux[i] = paire[0];
        ++lances;
    }
    char signal[2];
    if (read(canaux[0], &signal[0], 1) != 1 || signal[0] != 'p'
        || read(canaux[1], &signal[1], 1) != 1 || signal[1] != 'p')
        goto fin;
    if (send(canaux[0], "g", 1, MSG_NOSIGNAL) == 1
        && send(canaux[1], "g", 1, MSG_NOSIGNAL) == 1)
        code = 0;

fin:
    for (int i = 0; i < lances; ++i)
        close(canaux[i]);
    for (int i = 0; i < lances; ++i) {
        int statut;
        if (waitpid(enfants[i], &statut, 0) != enfants[i] || !WIFEXITED(statut) || WEXITSTATUS(statut))
            code = 1;
    }
    if (!code)
        puts("VULKAN_TRANSFERT_DEUX_PROCESSUS_OK");
    return code;
}

int main(int argc, char **argv)
{
    const uint32_t motifs[] = { UINT32_C(0x5a17c3e9), UINT32_C(0xa18e3c16) };
    if (argc == 2 && !strcmp(argv[1], "--allow-cpu-deux-processus"))
        return parallele(motifs);

    const int cpu = argc == 2 && !strcmp(argv[1], "--allow-cpu");
    if (argc > 1 && !cpu)
        return 2;
    int code = transfert(cpu, motifs[0], -1);
    return code ? code : transfert(cpu, motifs[1], -1);
}
