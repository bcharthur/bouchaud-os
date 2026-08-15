/*
 * Temoin M6 Bouchaud OS.
 *
 * Ce programme n'est pas un rasteriseur maison : il appelle directement
 * Gfx::Bitmap et Gfx::Painter d'upstream. Painter::create() instancie le
 * PainterSkia reel et PaintingSurface enveloppe notre bitmap en surface Skia.
 *
 * Le test est volontairement sans police ni fichier externe : il mesure le
 * chemin de rendu CPU avant d'ajouter les ressources de WebContent.
 */

#include <AK/Assertions.h>
#include <LibGfx/Bitmap.h>
#include <LibGfx/Color.h>
#include <LibGfx/Painter.h>
#include <LibGfx/Rect.h>

#include <cstdio>
#include <cstdint>

static unsigned echecs;
static unsigned passes;

static void verifie(bool condition, char const* message)
{
    if (condition) {
        ++passes;
        std::printf("  ok     %s\n", message);
    } else {
        ++echecs;
        std::printf("  ECHEC  %s\n", message);
    }
}

static uint64_t fnv1a(unsigned char const* data, size_t size)
{
    uint64_t hash = 1469598103934665603ULL;
    for (size_t i = 0; i < size; ++i) {
        hash ^= data[i];
        hash *= 1099511628211ULL;
    }
    return hash;
}

int main()
{
    std::puts("== temoin LibGfx : Skia CPU ==");

    auto bitmap_or_error = Gfx::Bitmap::create(
        Gfx::BitmapFormat::BGRA8888,
        Gfx::IntSize { 320, 200 });

    if (bitmap_or_error.is_error()) {
        std::puts("  ECHEC  creation du bitmap");
        std::printf("RESULTAT : 1 verification(s) en echec (0 passees)\n");
        return 1;
    }

    auto bitmap = bitmap_or_error.release_value();
    verifie(bitmap->width() == 320 && bitmap->height() == 200,
        "Bitmap BGRA8888 320x200 cree par LibGfx");

    auto painter = Gfx::Painter::create(bitmap);
    verifie(!!painter, "PainterSkia CPU cree");

    painter->clear_rect(Gfx::FloatRect { 0, 0, 320, 200 }, Gfx::Color::White);
    verifie(bitmap->get_pixel(5, 5) == Gfx::Color::White,
        "clear_rect a produit des pixels blancs");

    painter->fill_rect(Gfx::FloatRect { 32, 24, 128, 80 }, Gfx::Color::Red);
    painter->fill_rect(Gfx::FloatRect { 176, 96, 96, 72 }, Gfx::Color::Blue);

    bool zones =
        bitmap->get_pixel(40, 40) == Gfx::Color::Red
        && bitmap->get_pixel(200, 120) == Gfx::Color::Blue
        && bitmap->get_pixel(8, 190) == Gfx::Color::White;
    verifie(zones, "fill_rect Skia a produit rouge, bleu et fond intact");

    auto hash = fnv1a(bitmap->scanline_u8(0), bitmap->data_size());
    std::printf("         pixels : %zu octets, fnv1a64=%016llx\n",
        bitmap->data_size(),
        static_cast<unsigned long long>(hash));

    std::printf("\nRESULTAT : %u verification(s) en echec (%u passees)\n",
        echecs, passes);
    return echecs == 0 ? 0 : 1;
}
