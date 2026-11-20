#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <sys/mman.h>
#include <unistd.h>
#include <vector>

#include <vulkan/vulkan.h>
#include "include/core/SkCanvas.h"
#include "include/core/SkColor.h"
#include "include/core/SkImageInfo.h"
#include "include/core/SkPaint.h"
#include "include/core/SkSurface.h"
#include "include/gpu/ganesh/GrDirectContext.h"
#include "include/gpu/ganesh/SkSurfaceGanesh.h"
#include "include/gpu/ganesh/vk/GrVkDirectContext.h"
#include "include/gpu/vk/VulkanBackendContext.h"
#include "include/gpu/vk/VulkanExtensions.h"
#include "include/gpu/vk/VulkanMemoryAllocator.h"
#include "src/gpu/GpuTypesPriv.h"
#include "src/gpu/vk/vulkanmemoryallocator/VulkanMemoryAllocatorPriv.h"

static constexpr int dimension = 32;
using Pixels = std::array<uint8_t, dimension * dimension * 4>;

static bool dessine(VkInstance instance, VkPhysicalDevice physique, VkDevice device, uint32_t famille, Pixels& pixels)
{
    auto get_proc = [](const char* nom, VkInstance i, VkDevice d) -> PFN_vkVoidFunction {
        return d ? vkGetDeviceProcAddr(d, nom) : vkGetInstanceProcAddr(i, nom);
    };
    skgpu::VulkanExtensions extensions;
    extensions.init(get_proc, instance, physique, 0, nullptr, 0, nullptr);
    skgpu::VulkanBackendContext backend{};
    backend.fInstance = instance;
    backend.fPhysicalDevice = physique;
    backend.fDevice = device;
    backend.fGraphicsQueueIndex = famille;
    backend.fMaxAPIVersion = VK_API_VERSION_1_1;
    backend.fGetProc = get_proc;
    backend.fVkExtensions = &extensions;
    vkGetDeviceQueue(device, famille, 0, &backend.fQueue);
    backend.fMemoryAllocator = skgpu::VulkanMemoryAllocators::Make(backend, skgpu::ThreadSafe::kNo);
    if (!backend.fMemoryAllocator) return false;
    auto context = GrDirectContexts::MakeVulkan(backend);
    if (!context) return false;

    auto info = SkImageInfo::Make(dimension, dimension, kRGBA_8888_SkColorType, kPremul_SkAlphaType);
    auto surface = SkSurfaces::RenderTarget(context.get(), skgpu::Budgeted::kNo, info);
    if (!surface) return false;
    surface->getCanvas()->clear(SK_ColorRED);
    SkPaint peinture;
    peinture.setColor(SK_ColorGREEN);
    surface->getCanvas()->drawRect(SkRect::MakeLTRB(8, 8, 24, 24), peinture);
    context->flush(surface.get());
    if (!context->submit(GrSyncCpu::kYes)) return false;

    if (!surface->readPixels(info, pixels.data(), dimension * 4, 0, 0)) return false;
    for (int y = 0; y < dimension; y++) {
        for (int x = 0; x < dimension; x++) {
            const bool vert = x >= 8 && x < 24 && y >= 8 && y < 24;
            const auto* rgba = pixels.data() + 4 * (y * dimension + x);
            if (rgba[0] != (vert ? 0 : 255) || rgba[1] != (vert ? 255 : 0) || rgba[2] || rgba[3] != 255)
                return false;
        }
    }
    return true;
}

// Interface native de fenêtre du banc, même encodage que mire-gpu.
struct EnteteGui {
    uint32_t magic = 0x55474F42;
    uint16_t version = 1;
    uint16_t genre;
    uint32_t octets;
    uint32_t sequence;
};
static_assert(sizeof(EnteteGui) == 16);

static bool envoie_gui(int fd, uint16_t genre, uint32_t sequence, const void* payload, uint32_t octets)
{
    std::array<uint8_t, sizeof(EnteteGui) + 24> paquet{};
    if (octets > paquet.size() - sizeof(EnteteGui)) return false;
    EnteteGui entete{ .genre = genre, .octets = octets, .sequence = sequence };
    std::memcpy(paquet.data(), &entete, sizeof(entete));
    std::memcpy(paquet.data() + sizeof(entete), payload, octets);
    return write(fd, paquet.data(), sizeof(entete) + octets)
        == static_cast<ssize_t>(sizeof(entete) + octets);
}

static int parametre_gui(const char* nom)
{
    const char* valeur = std::getenv(nom);
    return valeur ? std::atoi(valeur) : -1;
}

static bool presente(const Pixels& pixels)
{
    const int canal = parametre_gui("BO_GUI_FD");
    if (canal < 0) return true;
    const int fd_surface = parametre_gui("BO_SURFACE_FD");
    const int largeur = parametre_gui("BO_SURFACE_WIDTH");
    const int hauteur = parametre_gui("BO_SURFACE_HEIGHT");
    const int pas = parametre_gui("BO_SURFACE_STRIDE");
    constexpr int zoom = 4;
    constexpr int cote = dimension * zoom;
    if (fd_surface < 0 || largeur < cote || hauteur < cote || pas < cote * 4) return false;
    const size_t taille_image = static_cast<size_t>(pas) * cote;
    auto* image = static_cast<uint8_t*>(mmap(nullptr, taille_image, PROT_READ | PROT_WRITE, MAP_SHARED, fd_surface, 0));
    if (image == MAP_FAILED) return false;

    for (int y = 0; y < cote; ++y) {
        auto* ligne = image + static_cast<size_t>(y) * pas;
        for (int x = 0; x < cote; ++x) {
            const auto* rgba = pixels.data() + 4 * ((y / zoom) * dimension + x / zoom);
            // Le bureau accepte BGRA8888. Convertir la zone publiée uniquement.
            ligne[4 * x] = rgba[2];
            ligne[4 * x + 1] = rgba[1];
            ligne[4 * x + 2] = rgba[0];
            ligne[4 * x + 3] = rgba[3];
        }
    }
    const uint32_t hello[] = {1, static_cast<uint32_t>(getpid())};
    constexpr char titre[] = "Skia Vulkan";
    const uint32_t zone[] = {1, 0, 0, 0, cote, cote};
    bool ok = envoie_gui(canal, 1, 1, hello, sizeof(hello))
        && envoie_gui(canal, 3, 2, titre, sizeof(titre) - 1)
        && envoie_gui(canal, 6, 3, zone, sizeof(zone));
    munmap(image, taille_image);
    if (!ok) return false;
    puts("SKIA_VULKAN_FENETRE_PRETE taille=128x128");
    std::fflush(stdout);
    // Le tampon partagé reste au gestionnaire de fenêtres. Le client écoute
    // jusqu'à la fermeture ou la rupture de son canal.
    uint8_t message[1024];
    ssize_t n;
    while ((n = read(canal, message, sizeof(message))) >= static_cast<ssize_t>(sizeof(EnteteGui))) {
        EnteteGui entete{};
        std::memcpy(&entete, message, sizeof(entete));
        if (entete.genre == 0x106) break;
    }
    return true;
}

int main()
{
    VkApplicationInfo app{};
    app.sType = VK_STRUCTURE_TYPE_APPLICATION_INFO;
    app.pApplicationName = "Bouchaud Skia Vulkan";
    app.apiVersion = VK_API_VERSION_1_1;
    VkInstanceCreateInfo config{};
    config.sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO;
    config.pApplicationInfo = &app;
    VkInstance instance{};
    if (vkCreateInstance(&config, nullptr, &instance) != VK_SUCCESS) return 1;
    uint32_t nombre = 0;
    bool ok = false;
    Pixels pixels{};
    if (vkEnumeratePhysicalDevices(instance, &nombre, nullptr) == VK_SUCCESS && nombre) {
        std::vector<VkPhysicalDevice> appareils(nombre);
        if (vkEnumeratePhysicalDevices(instance, &nombre, appareils.data()) == VK_SUCCESS) {
            const auto physique = appareils.front();
            uint32_t n = 0;
            vkGetPhysicalDeviceQueueFamilyProperties(physique, &n, nullptr);
            std::vector<VkQueueFamilyProperties> familles(n);
            vkGetPhysicalDeviceQueueFamilyProperties(physique, &n, familles.data());
            for (uint32_t i = 0; i < n; ++i) {
                if (!(familles[i].queueFlags & VK_QUEUE_GRAPHICS_BIT)) continue;
                const float priorite = 1;
                VkDeviceQueueCreateInfo queue{};
                queue.sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;
                queue.queueFamilyIndex = i;
                queue.queueCount = 1;
                queue.pQueuePriorities = &priorite;
                VkDeviceCreateInfo device_info{};
                device_info.sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO;
                device_info.queueCreateInfoCount = 1;
                device_info.pQueueCreateInfos = &queue;
                VkDevice device{};
                if (vkCreateDevice(physique, &device_info, nullptr, &device) == VK_SUCCESS) {
                    ok = dessine(instance, physique, device, i, pixels);
                    vkDestroyDevice(device, nullptr);
                }
                break;
            }
        }
    }
    vkDestroyInstance(instance, nullptr);
    if (ok) puts("SKIA_VULKAN_RGBA_OK pixels=1024");
    return ok && presente(pixels) ? 0 : 2;
}
