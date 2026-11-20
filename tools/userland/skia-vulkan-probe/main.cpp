#include <array>
#include <cstdint>
#include <cstdio>
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

static bool dessine(VkInstance instance, VkPhysicalDevice physique, VkDevice device, uint32_t famille)
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

    constexpr int dimension = 32;
    auto info = SkImageInfo::Make(dimension, dimension, kRGBA_8888_SkColorType, kPremul_SkAlphaType);
    auto surface = SkSurfaces::RenderTarget(context.get(), skgpu::Budgeted::kNo, info);
    if (!surface) return false;
    surface->getCanvas()->clear(SK_ColorRED);
    SkPaint peinture;
    peinture.setColor(SK_ColorGREEN);
    surface->getCanvas()->drawRect(SkRect::MakeLTRB(8, 8, 24, 24), peinture);
    context->flush(surface.get());
    if (!context->submit(GrSyncCpu::kYes)) return false;

    std::array<uint8_t, dimension * dimension * 4> pixels{};
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
                    ok = dessine(instance, physique, device, i);
                    vkDestroyDevice(device, nullptr);
                }
                break;
            }
        }
    }
    vkDestroyInstance(instance, nullptr);
    if (ok) puts("SKIA_VULKAN_RGBA_OK pixels=1024");
    return ok ? 0 : 2;
}
