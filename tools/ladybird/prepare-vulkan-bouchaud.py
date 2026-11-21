#!/usr/bin/env python3
"""Configure the opt-in Bouchaud Skia/Vulkan compositor in the pinned worktree."""
from pathlib import Path
import sys


def remplace(root: Path, chemin: str, source: str, cible: str) -> None:
    fichier = root / chemin
    texte = fichier.read_text(encoding="utf-8")
    if cible in texte:
        return
    if texte.count(source) != 1:
        raise SystemExit(f"Ancre Vulkan introuvable dans {chemin}")
    fichier.write_text(texte.replace(source, cible, 1), encoding="utf-8")


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: prepare-vulkan-bouchaud.py LADYBIRD_WORKTREE")
    root = Path(sys.argv[1])
    deps = "Meta/CMake/check_for_dependencies.cmake"
    remplace(
        root, deps,
        "        if (((LINUX AND NOT ANDROID) OR BSD) AND GLSLANG_VALIDATOR)",
        "        if (NOT BOUCHAUD_VULKAN_COMPOSITOR AND ((LINUX AND NOT ANDROID) OR BSD) AND GLSLANG_VALIDATOR)",
    )
    remplace(
        root, deps, "\nfind_package(CURL REQUIRED)",
        """
if (BOUCHAUD_VULKAN_COMPOSITOR AND NOT HAS_VULKAN)
    message(FATAL_ERROR "Bouchaud Vulkan compositor requires Vulkan Loader, headers and VMA")
endif()
if (BOUCHAUD_VULKAN_COMPOSITOR)
    add_cxx_compile_definitions(BOUCHAUD_VULKAN_COMPOSITOR=1)
endif()

find_package(CURL REQUIRED)""",
    )

    # Do not inspect handles returned by an unsuccessful or truncated enumeration.
    vulkan = "Libraries/LibGfx/VulkanContext.cpp"
    remplace(
        root, vulkan,
        """    vkEnumeratePhysicalDevices(instance, &device_count, nullptr);""",
        """#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    if (auto result = vkEnumeratePhysicalDevices(instance, &device_count, nullptr); result != VK_SUCCESS) {
        dbgln("[LB:SKIA] vulkan_enumerate_count_error={}", static_cast<int>(result));
        return Error::from_string_literal("Physical device count failed");
    }
#else
    vkEnumeratePhysicalDevices(instance, &device_count, nullptr);
#endif""",
    )
    remplace(
        root, vulkan,
        """    vkEnumeratePhysicalDevices(instance, &device_count, devices.data());""",
        """#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    if (auto result = vkEnumeratePhysicalDevices(instance, &device_count, devices.data()); result != VK_SUCCESS) {
        dbgln("[LB:SKIA] vulkan_enumerate_devices_error={}", static_cast<int>(result));
        return Error::from_string_literal("Physical device enumeration failed");
    }
    if (device_count == 0)
        return Error::from_string_literal("No physical devices after enumeration");
    devices.resize(device_count);
#else
    vkEnumeratePhysicalDevices(instance, &device_count, devices.data());
#endif""",
    )
    remplace(
        root, vulkan,
        """    for (auto const& device : devices) {
        if (picked_device == VK_NULL_HANDLE)""",
        """    for (auto const& device : devices) {
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
        uint32_t queue_count = 0;
        vkGetPhysicalDeviceQueueFamilyProperties(device, &queue_count, nullptr);
        if (queue_count == 0)
            continue;
        Vector<VkQueueFamilyProperties> queues;
        queues.resize(queue_count);
        vkGetPhysicalDeviceQueueFamilyProperties(device, &queue_count, queues.data());
        bool has_graphics = false;
        for (uint32_t index = 0; index < queue_count; ++index) {
            if (queues[index].queueCount > 0 && (queues[index].queueFlags & VK_QUEUE_GRAPHICS_BIT)) {
                has_graphics = true;
                break;
            }
        }
        if (!has_graphics)
            continue;
#endif
        if (picked_device == VK_NULL_HANDLE)""",
    )
    remplace(
        root, vulkan,
        """    VERIFY_NOT_REACHED();
}

static ErrorOr<VkDevice> create_logical_device""",
        """#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    return Error::from_string_literal("No Vulkan physical device with a graphics queue");
#else
    VERIFY_NOT_REACHED();
#endif
}

static ErrorOr<VkDevice> create_logical_device""",
    )

    # The platform selects CPU if optional Vulkan initialisation fails.
    remplace(
        root, "Libraries/LibGfx/SkiaBackendContext.cpp",
        """    backend_context.fMemoryAllocator = create_skia_vulkan_memory_allocator(vulkan_context);
    VERIFY(backend_context.fMemoryAllocator);

    sk_sp<GrDirectContext> ctx = GrDirectContexts::MakeVulkan(backend_context);
    VERIFY(ctx);""",
        """    backend_context.fMemoryAllocator = create_skia_vulkan_memory_allocator(vulkan_context);
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    sk_sp<GrDirectContext> ctx;
    if (backend_context.fMemoryAllocator)
        ctx = GrDirectContexts::MakeVulkan(backend_context);
    if (!ctx) {
        backend_context.fMemoryAllocator.reset();
        vkDestroyDevice(vulkan_context.logical_device, nullptr);
        vkDestroyInstance(vulkan_context.instance, nullptr);
        return {};
    }
#else
    VERIFY(backend_context.fMemoryAllocator);
    sk_sp<GrDirectContext> ctx = GrDirectContexts::MakeVulkan(backend_context);
    VERIFY(ctx);
#endif""",
    )
    remplace(
        root, "Libraries/LibGfx/VulkanContext.cpp",
        """    auto* instance = TRY(create_instance(api_version));
    auto* physical_device = TRY(pick_physical_device(instance));

    uint32_t graphics_queue_family = 0;
    auto* logical_device = TRY(create_logical_device(physical_device, &graphics_queue_family));""",
        """    auto* instance = TRY(create_instance(api_version));
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    auto physical = pick_physical_device(instance);
    if (physical.is_error()) {
        vkDestroyInstance(instance, nullptr);
        return physical.release_error();
    }
    auto* physical_device = physical.release_value();
    uint32_t graphics_queue_family = 0;
    auto device = create_logical_device(physical_device, &graphics_queue_family);
    if (device.is_error()) {
        vkDestroyInstance(instance, nullptr);
        return device.release_error();
    }
    auto* logical_device = device.release_value();
#else
    auto* physical_device = TRY(pick_physical_device(instance));
    uint32_t graphics_queue_family = 0;
    auto* logical_device = TRY(create_logical_device(physical_device, &graphics_queue_family));
#endif""",
    )
    remplace(
        root, "Services/Compositor/main.cpp",
        "    auto skia_backend_context = Gfx::SkiaBackendContext::the_main_thread_context();",
        """    auto skia_backend_context = Gfx::SkiaBackendContext::the_main_thread_context();
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    dbgln("[LB:SKIA] backend={}", skia_backend_context ? "vulkan" : "cpu");
#endif""",
    )

    remplace(
        root, "Libraries/LibGfx/PaintingSurface.h",
        "    void read_into_bitmap(Bitmap&, IntPoint source_position = {}) const;",
        """    void read_into_bitmap(Bitmap&, IntPoint source_position = {}) const;
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    bool try_read_into_bitmap(Bitmap&, IntPoint source_position = {}) const;
#endif""",
    )
    remplace(
        root, "Libraries/LibGfx/PaintingSurface.cpp",
        """void PaintingSurface::read_into_bitmap(Bitmap& bitmap, IntPoint source_position) const
{""",
        """void PaintingSurface::read_into_bitmap(Bitmap& bitmap, IntPoint source_position) const
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
{
    try_read_into_bitmap(bitmap, source_position);
}

bool PaintingSurface::try_read_into_bitmap(Bitmap& bitmap, IntPoint source_position) const
#endif
{""",
    )
    remplace(
        root, "Libraries/LibGfx/PaintingSurface.cpp",
        "    m_impl->surface->readPixels(pixmap, source_position.x(), source_position.y());",
        """#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    return m_impl->surface->readPixels(pixmap, source_position.x(), source_position.y());
#else
    m_impl->surface->readPixels(pixmap, source_position.x(), source_position.y());
#endif""",
    )
    remplace(
        root, "Libraries/LibGfx/PaintingSurface.h",
        "    Function<void(PaintingSurface&)> on_flush;",
        """    Function<void(PaintingSurface&)> on_flush;
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    bool bouchaud_readback_succeeded { false };
#endif""",
    )
    remplace(
        root, "Services/Compositor/BackingStoreManager.cpp",
        "#include <AK/AnyOf.h>",
        """#include <AK/AnyOf.h>
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
#    include <AK/Time.h>
#endif""",
    )
    remplace(
        root, "Services/Compositor/BackingStoreManager.cpp",
        """    auto bitmap = buffer.bitmap();
    surface->on_flush = [bitmap = move(bitmap)](auto& surface) {
        surface.read_into_bitmap(*bitmap);
    };""",
        """    auto bitmap = buffer.bitmap();
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    // RenderTarget peut échouer après une initialisation Vulkan réussie.
    // Le repli dessine directement dans le bitmap partagé.
    if (!surface->skia_backend_context())
        return Gfx::PaintingSurface::wrap_bitmap(*bitmap);
#endif
    surface->on_flush = [bitmap = move(bitmap)](auto& surface) {
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
        auto const start = MonotonicTime::now();
        surface.bouchaud_readback_succeeded = surface.try_read_into_bitmap(*bitmap);
        if (!surface.bouchaud_readback_succeeded) {
            warnln("[LB:SKIA] readback=failed");
            return;
        }
        static u64 successful_readbacks = 0;
        if (++successful_readbacks == 1)
            dbgln("[LB:SKIA] readback=vulkan");
        if (successful_readbacks % 120 == 1) {
            dbgln("[LB:SKIA] readback_us={} readback_bytes={}",
                (MonotonicTime::now() - start).to_microseconds(),
                static_cast<u64>(bitmap->width()) * bitmap->height() * 4);
        }
#else
        surface.read_into_bitmap(*bitmap);
#endif
    };""",
    )

    remplace(
        root, "Services/Compositor/BackingStoreManager.cpp",
        """        auto buffer = Gfx::SharedImageBuffer::create(allocation.size);
        if (should_publish)""",
        """#ifdef BOUCHAUD_VULKAN_COMPOSITOR
        // Nested contexts are sampled as Skia surfaces by the same Compositor.
        // They have no bitmap consumer outside this process.
        if (!should_publish) {
            m_backing_stores.append({
                .surface = Gfx::PaintingSurface::create_with_size(allocation.size, Gfx::BitmapFormat::BGRA8888, Gfx::AlphaType::Premultiplied, skia_backend_context),
                .bitmap_id = allocation.bitmap_ids[i],
                .state = initial_buffer_state(i),
                .accumulated_damage = { {}, allocation.size },
            });
            continue;
        }
#endif
        auto buffer = Gfx::SharedImageBuffer::create(allocation.size);
        if (should_publish)""",
    )
    remplace(
        root, "Services/Compositor/CompositorState.cpp",
        """    context->did_finish_gpu_present(bitmap_id);
    if (context->presents_to_client()) {
        VERIFY(m_client);""",
        """    context->did_finish_gpu_present(bitmap_id);
    if (context->presents_to_client()) {
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
        // The Skia completion callback reached the event loop. Require the
        // corresponding Vulkan surface and successful readback before attesting.
        auto const& rendered = context->latest_rendered_surface();
        static bool reported_vulkan_render = false;
        if (!reported_vulkan_render && rendered && rendered->skia_backend_context() && rendered->bouchaud_readback_succeeded) {
            dbgln("LADYBIRD_RENDER_BACKEND=vulkan");
            reported_vulkan_render = true;
        }
#endif
        VERIFY(m_client);""",
    )


if __name__ == "__main__":
    main()
