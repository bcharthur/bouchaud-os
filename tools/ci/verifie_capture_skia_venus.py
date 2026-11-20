#!/usr/bin/env python3
"""Verify the red and green Skia/Vulkan scene in a QEMU GTK capture."""
import sys
from PIL import Image


def verifie(image: Image.Image) -> tuple[int, int, int, int] | None:
    pixels = image.convert("RGB").load()
    rouge = (255, 0, 0)
    vert = (0, 255, 0)
    for taille in (128, 64):
        quarter = taille // 4
        # GTK peut interpoler les bords du framebuffer. Exiger une couleur
        # exacte dans les régions à plus de deux pixels de chaque arête.
        indices = [
            i for i in range(2, taille - 2)
            if abs(i - quarter) > 2 and abs(i - 3 * quarter) > 2
        ]
        count = len(indices) ** 2
        for y in range(1, image.height - taille):
            for x in range(1, image.width - taille):
                if pixels[x, y] != rouge or pixels[x - 1, y] == rouge:
                    continue
                if pixels[x + quarter + 3, y + quarter + 3] != vert:
                    continue
                if all(
                    pixels[x + i, y + j]
                    == (vert if quarter <= i < 3 * quarter and quarter <= j < 3 * quarter else rouge)
                    for j in indices for i in indices
                ):
                    return x, y, taille, count
    return None


if __name__ == "__main__":
    with Image.open(sys.argv[1]) as capture:
        resultat = verifie(capture)
    if resultat is None:
        raise SystemExit("Motif Vulkan absent ou incorrect dans la capture")
    x, y, taille, count = resultat
    print(f"SKIA_VULKAN_CAPTURE_OK pixels_exacts={count} zone={taille}x{taille}@{x},{y}")
