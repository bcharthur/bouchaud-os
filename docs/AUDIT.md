# Audit des couches — Bouchaud OS + Nautile (juillet 2026)

Méthode : lecture de chaque couche, compilation contre la cible réelle
(`x86_64-bouchaud_os.json`), et pour chaque bibliothèque externe candidate un
**test empirique** (compile-t-elle vraiment en no_std contre cette cible ?)
suivi d'une **vérification pixel-exacte** en harnais std quand c'est du
décodage (mêmes fichiers sources que le noyau, comparés à la référence).

Barème : ✅ solide · 🟡 fonctionne avec des manques connus · 🔴 embryonnaire.

---

## Couche par couche

### arch/x86_64 — ✅ socle sain
GDT/IDT/ports/instructions via les crates standard de l'OSdev Rust
(`x86_64`, `pic8259`). **Manques** : APIC/IOAPIC (on est sur le PIC 8259
hérité), SMP (mono-cœur), ACPI (pas d'extinction propre ni d'énumération).

### kernel — 🔴 la couche la plus mince de l'OS
- `heap`/`memory` : allocateur `linked_list_allocator` sur toute la RAM
  mappée, bascule d'arène au boot. Correct.
- `timer` : PIT + TSC calibré, millisecondes réelles indépendantes de la
  vitesse QEMU. Correct.
- `dmesg`/`panic` : journal circulaire + handler. Correct.
- `scheduler` : **placeholder assumé** — coopératif, `yield_now()` est un
  no-op, pas de préemption, pas de changement de contexte, pas de piles par
  tâche. `process`/`syscall`/`handle` : embryonnaires.
- **Priorité OS n°1** : ordonnanceur préemptif round-robin sur IRQ0
  (sauvegarde/restauration de registres, une pile par tâche). Sans lui, un
  chargement de page fige tout le bureau — c'est le plafond de verre actuel.

### drivers — 🟡 suffisant pour QEMU, rien pour le matériel réel
e1000 (réseau), VGA/framebuffer, clavier, souris, série : OK pour QEMU.
**Manques** : disque (`disk.rs` = 20 lignes, pas d'ATA/AHCI/NVMe → aucune
persistance), USB, audio, virtio (perfs QEMU bien meilleures que l'émulation
e1000/VGA).

### fs — 🔴 RAM uniquement
`ramfs` avec permissions basiques. Tout est perdu à l'extinction.
**Manques** : driver bloc + FS persistant (FAT32 est le plus simple ;
crates candidates : `fatfs` — à tester en no_std), VFS, montage.

### net — ✅ remarquablement complet
Du lien à l'application : Ethernet/ARP/IPv4/ICMP/DHCP/DNS, TCP maison,
**TLS 1.3 maison** (X25519, AES-GCM, ChaCha20-Poly1305, X.509 RSA/ECDSA
P-256/384), HTTP/1.1 avec gzip/deflate/**brotli** maison, HTTP/2 (HPACK)
négocié par ALPN, cache mémoire, budget réseau par page (5 s).
`smoltcp` compilé et branché sur une commande de test (`smoltest`) mais pas
encore chemin par défaut.
**Manques** : IPv6, reprise de session TLS (0-RTT/tickets), cookies
persistants, WebSocket, QUIC/HTTP-3 (hors de portée raisonnable).
**Reco** : après un `smoltest` concluant en exécution réelle, basculer
`fetch_document` sur smoltcp (retransmission/fenêtres RFC 793 correctes,
là où le TCP maison est fragile sur pertes).

### wasm — ✅ `wasmi` 0.31 no_std, fonctionne.

### gui — 🟡 fonctionnel
Framebuffer double-buffer, gestionnaire de fenêtres (drag/resize/z-order),
apps natives. **Manques** : rectangles sales (on repeint tout à chaque
frame), pas de compositing par fenêtre.

### browser/Nautile — 🟡 le plus gros investissement, des manques ciblés

| Sous-système | État | Détail |
|---|---|---|
| HTML | ✅ | tokenizer/tree-builder maison, entités, auto-close |
| CSS | 🟡 | cascade + index par clé (id/classe/tag), calc(), var(), nth/attributs/combinateurs ; manque @media complet, ::before/::after, animations |
| Layout flex | ✅ | **taffy** (vrai algorithme) branché sur le rendu réel |
| Layout grid | ✅ | **taffy grid branché par cet audit** (pistes fr/px/%/minmax/repeat, spans) ; heuristique en repli |
| Position/couches | 🟡 | absolute/fixed/z-index avec plafond MAX_LAYERS=256 ; hauteurs % imbriquées toujours résolues contre le viewport (limitation single-pass connue) |
| Images | ✅ | PNG (maison + **zune-png** entrelacé), JPEG (maison + **zune-jpeg** progressif), GIF 1re frame, BMP, WebP **VP8L ET VP8 lossy** (vp8.rs, bit-exact vs libwebp sur 11 images) ; manquent : AVIF, GIF animé, alpha ALPH |
| Polices | ✅ | **fontdue** primaire (cet audit) + rasterizer maison en repli ; manque : @font-face (webfonts), fallback multi-polices, shaping (rustybuzz validé no_std, à brancher) |
| JS | 🟡 | interpréteur maison ~4600 lignes, DOM/événements/modules ; async/await *parsés mais synchrones*, pas de vraie event loop ni de vrais timers |
| Réseau page | ✅ | budget 5 s, CSS externe, sous-ressources |

---

## Bibliothèques externes : testées empiriquement sur cette cible

### Intégrées (cet audit)
| Crate | Rôle | Vérification |
|---|---|---|
| `zune-jpeg` 0.5 | JPEG **progressif** (SOF2), en repli du décodeur maison | 0/5917 px d'écart vs libjpeg (harnais) |
| `zune-png` 0.4 | PNG **entrelacé** (Adam7), en repli du maison | 0/1650 px, exact |
| `fontdue` 0.9 | rendu de police primaire (AA mature, cmap 4/12, composites) | conversion métriques vérifiée (ascent/ymin/avance, rendu accents) |
| `hashbrown` 0.15 | HashMap O(1) : scopes JS (chemin le plus chaud de l'interpréteur) + tables Huffman WebP | WebP re-vérifié pixel-exact après bascule |
| `taffy` 0.12 + feature **grid** | vrai CSS Grid dans `grid_inner` | parsing des pistes vérifié valeur par valeur |

### Déjà en place (sessions précédentes)
`taffy` (flex), `smoltcp` (TCP expérimental), `wasmi` (WebAssembly).

### Validées no_std mais PAS branchées (prochaines étapes naturelles)
- `rustybuzz` 0.20 : shaping OpenType (ligatures, arabe/indic, kerning) —
  compile sur la cible ; le brancher = passer le pipeline texte des `char`
  aux glyph IDs.

### Testées et REJETÉES (dépendance transitive exigeant `std`)
| Crate | Bloqueur |
|---|---|
| `boa_engine` (JS) | `serde_core`, ~5800 erreurs |
| `cssparser` (Servo) | `phf` avec feature `std` forcée (unification cargo) |
| `image-webp` | `byteorder-lite` |
| `png` 0.17 | `fdeflate`/`miniz_oxide` en mode std |
| `gif` 0.13 | `weezl` |

---

## Priorités recommandées (ordre de valeur/effort)

**Navigateur**
1. ~~WebP VP8 lossy~~ — **FAIT** : décodeur keyframe complet écrit à la main
   (vp8.rs : lecteur booléen RFC 6386, segments, tokens DCT, prédiction intra
   16x16/4x4/chroma, IDCT/IWHT bit-exacts, filtre de boucle simple+normal,
   suréchantillonnage fancy, YUV→RGB virgule fixe libwebp), vérifié
   **bit-exact** contre libwebp sur 11 images (1 px → 120 000 px, q10 → q100).
2. ~~@media queries~~ — **FAIT** : conditions évaluées contre le viewport
   (min/max-width/height, orientation, print exclu, prefers-color-scheme).
3. ~~Cookies~~ — **FAIT** : pot en mémoire, envoyé sur h1 + h2.
4. Event loop JS : les files micro/macrotâches existent déjà (js/mod.rs:944) ;
   reste à pomper les timers longs APRÈS le premier rendu (progressif).
5. **@font-face** : télécharger la police de la page et la charger dans
   fontdue (il accepte des bytes arbitraires) — gros gain de fidélité.
6. **rustybuzz** : shaping (déjà validé no_std).
7. Reprise de session TLS : moins de round-trips.

**OS**
6. **Ordonnanceur préemptif** (voir kernel) — débloque le reste (réseau en
   tâche de fond, UI fluide pendant les chargements).
7. **Persistance** : driver ATA + FAT32 (`fatfs` à tester).
8. virtio-net/virtio-gpu pour QEMU.

---

*Audit réalisé et intégrations appliquées le 2026-07-08 sur la branche
`claude/bouchaud-os-audit-erast2`. Toutes les intégrations compilent contre
la cible réelle et gardent le chemin maison en repli : le boot ne peut pas
être cassé par une régression de crate externe.*
