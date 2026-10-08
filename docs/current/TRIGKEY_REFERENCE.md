# TRIGKEY Speed S5 — référence physique

| Composant | Valeur observée |
|---|---|
| CPU | AMD Ryzen 7 5800H with Radeon Graphics |
| CPU logiques | 16 |
| RAM utilisable | ~11,5 Gio observés |
| GOP | 1920×1080, 32 bpp |
| Ethernet | Realtek `10ec:8168` |
| Wi-Fi | Intel `8086:2723` AX200 |
| NVMe | contrôleur Kingston `2646:*` |
| USB | 2 contrôleurs xHCI AMD `1022:1639` |
| iGPU | AMD `1002:1638` Cezanne |

Le framebuffer physique repose actuellement sur GOP. Aucun pilote AMD accéléré
n'est revendiqué.

Le RTL8111/8168 est le chemin Ethernet physique de référence.

Le NVMe physique est distinct de l'ATA utilisé dans certains bancs QEMU.

AC'97 QEMU ne prouve pas l'audio HDA matériel.

Le virtio-gpu QEMU ne prouve pas l'iGPU AMD.
