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
        root, "Libraries/LibGfx/PaintingSurface.h", "#include <LibGfx/Size.h>",
        """#include <LibGfx/Size.h>
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
#    include <AK/Optional.h>
#    include <LibGfx/Rect.h>
#endif""",
    )
    remplace(
        root, "Libraries/LibGfx/PaintingSurface.h",
        "    void read_into_bitmap(Bitmap&, IntPoint source_position = {}) const;",
        """    void read_into_bitmap(Bitmap&, IntPoint source_position = {}) const;
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    bool try_read_into_bitmap(Bitmap&, IntPoint source_position = {}, Optional<IntRect> destination = {}) const;
    Optional<IntRect> bouchaud_readback_damage;
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

bool PaintingSurface::try_read_into_bitmap(Bitmap& bitmap, IntPoint source_position, Optional<IntRect> destination) const
#endif
{""",
    )
    remplace(
        root, "Libraries/LibGfx/PaintingSurface.cpp",
        "    m_impl->surface->readPixels(pixmap, source_position.x(), source_position.y());",
        """#ifdef BOUCHAUD_VULKAN_COMPOSITOR
    if (destination.has_value()) {
        auto const& rect = *destination;
        auto const subset_rect = SkIRect::MakeXYWH(rect.x(), rect.y(), rect.width(), rect.height());
        SkPixmap subset;
        if (!pixmap.bounds().contains(subset_rect) || !pixmap.extractSubset(&subset, subset_rect))
            return false;
        return m_impl->surface->readPixels(subset, source_position.x(), source_position.y());
    }
    return m_impl->surface->readPixels(pixmap, source_position.x(), source_position.y());
#else
    m_impl->surface->readPixels(pixmap, source_position.x(), source_position.y());
#endif""",
    )
    remplace(
        root, "Services/Compositor/BackingStoreManager.cpp",
        "#include <AK/AnyOf.h>",
        """#include <AK/AnyOf.h>
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
#    include <AK/Array.h>
#    include <AK/Optional.h>
#    include <AK/StringBuilder.h>
#    include <AK/Time.h>
#    include <stdlib.h>
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
    // Chaque bitmap doit d'abord recevoir une image complète. Une erreur de
    // copie complète invalide la base jusqu'à la prochaine lecture réussie.
    surface->on_flush = [bitmap = move(bitmap), needs_full_readback = true](auto& surface) mutable {
        static bool const partial = getenv("BOUCHAUD_SKIA_PARTIAL_READBACK") != nullptr;
        auto const damage = surface.bouchaud_readback_damage;
        surface.bouchaud_readback_damage.clear();
        static bool const measure = getenv("BOUCHAUD_SKIA_TIMINGS") != nullptr;
        auto const begin = measure ? Optional<MonotonicTime> { MonotonicTime::now() } : Optional<MonotonicTime> {};
        bool const copied_subset = partial && !needs_full_readback && damage.has_value()
            && !damage->is_empty() && damage->size() != surface.size()
            && surface.try_read_into_bitmap(*bitmap, damage->location(), damage);
        bool const copied = copied_subset || surface.try_read_into_bitmap(*bitmap);
        needs_full_readback = !copied;
        auto const copied_size = copied_subset ? damage->size() : bitmap->size();
        if (copied_subset) {
            static bool published_subset = false;
            if (!published_subset) {
                dbgln("[LB:SKIA] readback=partial");
                published_subset = true;
            }
        }
        if (begin.has_value()) {
            auto const end = MonotonicTime::now();
            auto const elapsed = (end - *begin).to_microseconds();
            struct ReadbackSample {
                i64 end_us;
                i64 elapsed_us;
                u64 bytes;
            };
            static thread_local Array<ReadbackSample, 16> samples {};
            static thread_local size_t count = 0;
            samples[count++] = { end.nanoseconds() / 1000,
                elapsed, copied ? static_cast<u64>(copied_size.width()) * copied_size.height() * 4 : 0 };
            if (count == samples.size()) {
                StringBuilder line;
                for (auto const& sample : samples)
                    line.appendff(" {}:{}:{}", sample.end_us, sample.elapsed_us, sample.bytes);
                dbgln("[LB:SKIA:READBACK] end_us:wall_us:bytes{}", line.string_view());
                count = 0;
            }
        }
        if (!copied) {
            warnln("[LB:SKIA] readback=failed");
            return;
        }
        static bool published_readback = false;
        if (!published_readback) {
            dbgln("[LB:SKIA] readback=vulkan");
            published_readback = true;
        }
    };
#else
    surface->on_flush = [bitmap = move(bitmap)](auto& surface) {
        surface.read_into_bitmap(*bitmap);
    };
#endif""",
    )

    remplace(
        root, "Services/Compositor/BackingStoreManager.cpp",
        "        auto damage_rect = store.accumulated_damage;",
        """        auto damage_rect = store.accumulated_damage;
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
        // Le dommage est cumulé depuis la dernière utilisation de ce tampon.
        store.surface->bouchaud_readback_damage = damage_rect;
#endif""",
    )

    player = "Libraries/LibWeb/Painting/DisplayListPlayerSkia.cpp"
    remplace(
        root, player, "#include <AK/TemporaryChange.h>",
        """#include <AK/TemporaryChange.h>
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
#    include <AK/Array.h>
#    include <AK/StringBuilder.h>
#    include <AK/Time.h>
#    include <stdlib.h>
#endif""",
    )
    remplace(
        root, player,
        """    if (auto context = surface.skia_backend_context())
        context->flush_and_submit_async(&surface.sk_surface(), move(callback));
    else
        callback();
    surface.flush();""",
        """    if (auto context = surface.skia_backend_context()) {
#ifdef BOUCHAUD_VULKAN_COMPOSITOR
        static bool const measure = getenv("BOUCHAUD_SKIA_TIMINGS") != nullptr;
        if (measure) {
            struct Samples {
                Array<i64, 16> values {};
                size_t count = 0;

                void record(char const* stage, i64 duration)
                {
                    values[count++] = duration;
                    if (count != values.size())
                        return;
                    StringBuilder line;
                    for (auto const value : values)
                        line.appendff(" {}", value);
                    dbgln("[LB:SKIA:GPU] stage={} wall_us{}", stage, line.string_view());
                    count = 0;
                }
            };
            static thread_local Samples completions;
            static thread_local Samples submissions;
            auto const begin = MonotonicTime::now();
            context->flush_and_submit_async(&surface.sk_surface(), [begin, callback = move(callback)] {
                completions.record("completion_observed", (MonotonicTime::now() - begin).to_microseconds());
                callback();
            });
            submissions.record("flush_submit", (MonotonicTime::now() - begin).to_microseconds());
        } else
#endif
            context->flush_and_submit_async(&surface.sk_surface(), move(callback));
    } else {
        callback();
    }
    surface.flush();""",
    )


if __name__ == "__main__":
    main()
