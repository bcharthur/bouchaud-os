# Registre de convergence — défauts bloquants (ouvert le 21 octobre 2026)

Source de vérité des défauts bloquants, avec [P_LOTS_REGISTER.md](P_LOTS_REGISTER.md)
pour l'état des lots. Registre **fermé** : aucun nouveau chantier tant que les
P0 ne sont pas clos.

## Règles

1. **Une fiche par défaut**, avec six rubriques : preuve initiale ; mécanisme
   causal **démontré** (ou « inconnu ») ; correctif ; **reproduction négative
   avant correctif** (le même banc rougit sans le correctif) ; validation après
   correctif ; **limites de la preuve**.
2. Un défaut intermittent qui **ne se reproduit plus n'est pas résolu** :
   « non reproduit en N essais » est une mesure, pas une clôture.
3. Aucune clôture sur un seul run vert ; un `NON_QUALIFIANT` reste non
   qualifiant ; `P13_STRICT=0` n'est pas une qualification stricte ; aucun
   seuil ne bouge pour faire passer un critère.
4. Deux niveaux de test : bancs ciblés locaux pendant le développement ;
   campagnes complètes seulement sur un HEAD cohérent, poussé groupé, sans
   pousser pendant une campagne.

Statuts : `OUVERT` (cause inconnue) · `CAUSE_CONFIRMEE` · `EN_COURS`
(correctif local, non poussé) · `CORRIGE_TCG` (correctif + bancs TCG verts,
KVM à mesurer) · `KVM_VERIFIE` · `QUALIFIE` (critère officiel inchangé,
plusieurs campagnes indépendantes) · `CLOS`.

## Synthèse

| # | Défaut | Statut | Ce qui manque |
|---|---|---|---|
| C1 | Console série : émission interruptions masquées | `CORRIGE_TCG` | KVM : l'étape « Console série » d'os-primitives a échoué sur `3adcea3c` (flot saturé KVM, 586/720 : expliqué par C1f/C1h) ; A/B console on/off KVM |
| C1a–C1e | Défauts trouvés par les bancs de C1 | `CORRIGE_TCG` | KVM |
| C1f | Saturation : verrou de l'anneau sans borne, puis lignes perdues et repli sous flot saturé (KVM SMP4 586/720) | `CORRIGE_TCG` | CI KVM sur le HEAD final (saturation, verrou 3 s/12 s, paniques 2 000 lignes SMP1/2/4) |
| C1g | Queue de 5–6 ms au repos en SMP1 KVM, hors du sommeil | `OUVERT` | granularité fausse corrigée (`cdba65db`) ; A/B console on/off KVM sur le HEAD final ; cause |
| C1h | Copie VGA texte d'un `write(2)` : 762 ms interruptions masquées | `CORRIGE_TCG` | KVM (`recopie_pire_us`, `tenu_pire_us`) |
| C1i | Ligne différée qui double l'anneau (ordre d'un écrivain rompu) | `CORRIGE_TCG` | KVM (paniques 2 000 lignes) |
| C2 | Reprise SMP4 KVM : 1 226 µs | `OUVERT` | segments KVM (run 39602362503) |
| C3 | Attente interactive jusqu'au quantum | `OUVERT` | KVM après C1 ; renoncements |
| C4 | A/B non qualifiable | `CAUSE_CONFIRMEE` | 3 campagnes en parallèle, décision humaine |
| C5 | SIGSEGV compta-stress / persist-course | `OUVERT` | reproduction déterministe |
| C6 | Surcomptage CPU (I2) | `OUVERT` | reproduction |
| C6a | Chevauchement de tranche après idle | `CORRIGE_TCG` | injection, KVM |
| C7 | Compositor résiduel | `OUVERT` | preuve de mort de tous les processus |
| C8 | Croissance mémoire WebContent | `CORRIGE_TCG` (cause non confirmée) | plusieurs campagnes |
| C9 | Panique `courant.rs:276` | `OUVERT` | contention de verrous, `held_at` |
| C10 | Assertion LibJS `Value.cpp:484` | `OUVERT` | invariant violé ou corruption |
| C11 | `irq_depth()` toujours nul | `CAUSE_CONFIRMEE` | comptage fiable des entrées/sorties d'IRQ |
| C12 | Préfixe du journal : accès CMOS | `CORRIGE_TCG` | horodatage sous KVM |

## P0 — bloquent P13

### C1 — Console série : lignes émises interruptions masquées

- **Preuve initiale** : KVM SMP4 `serie_pire_us` 15–20 ms par bras (run
  39503091750) ; TCG os-primitives : émission masquée pire 28,6 ms, 5 497
  émissions > 2 ms, attente du jeton 67 ms.
- **Mécanisme démontré** : le coût du port (une sortie de VM par octet sous
  KVM, ~87 µs/octet à 115 200 bauds) payé interruptions masquées par chaque
  émetteur, jeton tenu, après les lignes différées de tous les cœurs.
- **Correctif** : `c6206281` (BOUCHAUD_CONSOLE_ASYNCHRONE_V1) — anneau de
  64 Kio, fil `console`, morceaux de 16 octets préemption coupée et
  interruptions ouvertes, enregistrement de tête reprenable, producteur qui
  aide quand l'anneau est plein, panique/extinction/arrêt qui vident.
- **Reproduction négative** : même binaire, `console-async off` (chemin
  d'avant) : bras témoin de l'attribution KVM et banc panique `off`.
- **Validation après** : TCG os-primitives : émission masquée pire 6,5 ms
  (démarrage seulement), 36 émissions > 2 ms, attente du jeton 0 ; panique
  SMP4 8/8, SMP2, SMP1, synchrone ; entrelacement SMP1/2/4 ; détenteur du
  jeton suspendu 3 s et 12 s. KVM A/B réveil précis (`3adcea3c` contre
  `571b4978`, un run chacun) : pire au repos SMP1 6,7–18,7 ms → 6,6–7,5 ms.
- **Limites** : l'étape « Console série » d'os-primitives a échoué sur
  `3adcea3c` — le flot saturé KVM (586/720, repli) ; mécanismes et
  correctifs en C1f, C1h, C1i ; voir aussi C1g.

### C1a à C1e — défauts trouvés par les bancs de C1 (corrigés avant `c6206281`)

| # | Défaut | Reproduction négative | Mécanisme et correctif | Validation |
|---|---|---|---|---|
| C1a | anneau plein dans un contexte qui ne peut pas céder | `run_console_panique.sh` : 190/600 lignes perdues | repli dans l'anneau du cœur (4 Kio) → le producteur aide à vider et retente sa place après chaque essai | 600/600 |
| C1b | ordre d'un écrivain rompu | `+ panic-test` avant des lignes plus anciennes ; 543 avant 197 | alternance anneau / différées → différées seulement anneau vide (ou 1 morceau sur 32), jamais au milieu d'un enregistrement entamé — **insuffisant**, voir C1i | panique ×5 SMP4 |
| C1c | SMP1 : attente à vide de 5 s, repli, lignes restées à l'extinction | journal coupé au milieu d'un préfixe ; `retard_pire_us=5010333` | contexte de démarrage qui ne peut pas céder, jeton tenu par le fil préempté → émet lui-même ; enregistrement reprenable | entrelacement SMP1 |
| C1d | panique : second lecteur de l'anneau, autre cœur qui écrit | SMP4 : 22 doublons, 285 lignes abîmées | seul le cœur qui panique lit l'anneau, prend et garde le jeton ; les autres diffèrent | panique ×5 SMP4, SMP2, SMP1 |
| C1e | panique SMP4 : ligne d'un autre cœur au milieu d'un enregistrement entamé | 1 run sur 5 : `DHCP_RX_ANNEAU` dans la rafale 238 | un détenteur légitime n'achevait pas l'enregistrement entamé si la panique venait de commencer → l'achève toujours (le cœur qui panique attend jusqu'à 200 ms) | panique ×8 SMP4 |

Limites communes : TCG uniquement ; la négative de C1a à C1e est le banc
rouge sur l'image d'avant le correctif, pas une injection.

### C1f — Saturation du port : verrou sans borne, lignes perdues, repli

- **Preuve initiale** : (a) revue du 21/10 — `range_console` bouclait sur
  `CONSOLE_VERROU` interruptions masquées sans borne ; (b) os-primitives KVM
  SMP4 `3adcea3c` (run 39602362503), flot saturé (6 écrivains, lignes de
  1 800 octets) : 586/720 intactes, 135 perdues, 108 différées, **repli de
  sécurité**, `retard_pire_us=12254315`.
- **Reproduction sous TCG** : `BANC_SERIE_LENTE_NS=5000` (5 µs par octet sur
  COM1, le coût d'une sortie de VM) : 494/720, 323 perdues, 92 inversions
  d'ordre, ou 720/720 avec repli après 2,0–2,4 s de fil muet selon le run.
- **Mécanismes démontrés** (relevés au repli : phase et état du fil, occupant,
  durée de détention, appel système, adresse du dernier battement de chaque
  cœur, `BOUCHAUD_CONSOLE_REPLI_COEUR`) :
  1. verrou de l'anneau attendu masqué sans borne (banc
     `BANC_VERROU_SUSPENDU_MS=3000` : 2,92 s masquées, repli) ;
  2. **perte** : anneau plein, chaque producteur aidait à vider puis
     retentait ; la place libérée était prise par un autre, il renonçait à
     1 s et sa ligne débordait l'anneau de 4 Kio de son cœur ;
  3. **repli** : le fil `console`, **prêt** dans la file d'un cœur, n'y était
     pas élu pendant 1,4–2 s ; l'occupant, un écrivain en `write(2)`,
     restait en mode noyau (dernier battement `differee_appel`) dans
     `vga::avec_console` — la copie VGA texte, voir C1h ;
  4. verrou suspendu 12 s : l'attente cédée renonçait à 5 s (417 perdues).
- **Correctifs** :
  - BOUCHAUD_CONSOLE_VERROU_BORNE_V1 : attente masquée bornée à 1 ms (une
    détention suspendue ne coûte qu'une attente), puis anneau du cœur ;
    écrivain cédable : attente **interruptions ouvertes en cédant**, borne
    60 s (12 s de suspension couvertes, jamais de blocage permanent) ;
    BOUCHAUD_CONSOLE_REPLI_COOPERATIF_V1 et REPLI_EXPLIQUE_V1 (diagnostic).
  - BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 : un producteur cédable fait la
    queue (tickets, `file_place.rs`) sans échéance tant que la console
    avance ; la tête vide elle-même par morceaux ; les autres dorment
    (attente non interruptible : une tâche condamnée ne meurt pas avec son
    ticket) ; tête figée sautée après 2 s. Une **réserve d'urgence** de
    8 Kio est laissée aux contextes qui ne peuvent pas attendre (IRQ,
    verrou tenu, fil noyau), seuls à passer par l'aide bornée.
  - C1h (copie VGA) et C1i (ordre des différées).
- **Reproduction négative** : même bancs sur l'image d'avant (`apres22`) —
  saturation 494/720 et 323 perdues ; verrou 3 s 2,92 s masquées (borne
  retirée) ; verrou 12 s à 5 s de borne : 417 perdues ; mutations du garde
  (`verifie-console-asynchrone.py`, 60 négatifs ; propriété `file_place`,
  5 mutations détectées sur 64 graines).
- **Validation après** (TCG, image finale) : saturation 5 µs/octet SMP1,
  SMP2, SMP4 : 720/720, 0 perdue, 0 doublon, 0 inversion, aucun repli ;
  producteur au-delà de 115 200 bauds (87 µs/octet) 120/120 ; contre-pression
  mesurée (SMP4 5 µs/octet : 1 385 attentes, moyenne 0,33 s, pire 0,71 s ;
  87 µs/octet : moyenne 0,92 s, pire 2,4 s) ; verrou suspendu 3 s et 12 s,
  détenteur du jeton suspendu 3 s et 12 s : 600/600, 0 perdue, console
  asynchrone ; paniques SMP1/2/4 et `off`.
- **Limites** : TCG ; un vrai vCPU retiré par l'hôte n'est pas simulable,
  seulement son effet ; la contre-pression coûte de la latence aux
  écrivains (mesurée, jamais une perte) ; à valider sous KVM sur le HEAD
  final (os-primitives : saturation TCG, verrou 12 s KVM, paniques 2 000
  lignes KVM SMP1/2/4, et l'admission des pertes du jeton 12 s levée).

### C1h — Copie VGA texte d'un `write(2)` : 762 ms interruptions masquées

- **Preuve initiale** : relevé du repli de C1f — adresses du dernier
  battement dans `vga::avec_console` (`try_lock`, `write_fmt`).
- **Mécanisme démontré** (BOUCHAUD_VGA_TEXTE_MESURE_V1) : chaque cellule
  écrite en MMIO, chaque défilement relisant et recopiant l'écran octet par
  octet (7 680 accès), sous un `SpinLockIrq`. Banc de saturation TCG SMP4 :
  verrou tenu **762 ms masqué**, attente 2,68 s.
- **Correctif** : `1cb8e08a` (BOUCHAUD_VGA_TEXTE_OMBRE_V1) — écran en RAM,
  recopie des lignes modifiées en mots de 32 bits **hors** de la section
  masquée, un publieur à la fois.
- **Reproduction négative** : même banc, image d'avant : 762 ms / 2,68 s.
- **Validation après** : tenu 3,7–10,1 ms (TCG, lignes de 1 800 octets),
  attente ≤ 5,4 ms, recopie hors section ≤ 56 ms, aucun repli ; rendu identique **au pixel**
  (`screendump`, 300 lignes). Le défilement matériel (CRTC) a été rejeté :
  écran du BIOS sous QEMU.
- **Limites** : coût KVM inconnu (une sortie de VM par mot recopié) ; la
  recopie reste à la charge de l'écrivain, hors section masquée.

### C1i — Ligne différée qui double l'anneau

- **Preuve initiale** : `run_console_panique.sh 2000`, TCG SMP4 : la 1 615
  entre la 1 281 et la 1 282 (2 runs sur 4) ; banc de saturation d'avant :
  92 inversions par écrivain (nouveau contrôle de l'analyseur).
- **Mécanisme démontré** : une ligne différée (attente masquée du verrou
  > 1 ms, aide à échéance) passait devant l'anneau « une fois tous les 32
  morceaux », avant des lignes plus anciennes du même écrivain.
- **Correctif** (BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V1) : chaque ligne
  différée porte son **repère** (position d'écriture de l'anneau) et ne sort
  que lorsque la lecture l'a atteint — fusion exacte, en asynchrone, en
  synchrone et en panique.
- **Reproduction négative** : image d'avant, 2 échecs sur 4 ; mutations
  du garde (repère ignoré, plus récente d'abord, après la tête, repère émis).
- **Validation après** (TCG, image finale) : 2 000 lignes SMP4 5/5, SMP2,
  SMP1 dans l'ordre ; tous les bancs d'entrelacement : 0 inversion, 0
  doublon.
- **Limites** : intermittent avant correctif (2/4) : la validation est
  statistique ; le contrôle d'ordre est désormais exigé par l'analyseur du
  banc d'entrelacement.

### C1g — Queue de 5–6 ms au repos en SMP1 KVM, hors du sommeil

- **Preuve initiale** : A/B réveil précis KVM SMP1, `3adcea3c` (run
  39602362503) contre `571b4978` (run 39563049559), un run chacun. Bras `on` :
  p95 42–50 µs → 298–5 411 µs, p99 65–2 737 µs → 6 305–6 567 µs. Le noyau
  n'attribue au pire réveil que 1,0–1,5 ms (détection au tic) : le retard est
  **hors** de `dort_jusqua`.
- **Défaut de mesure trouvé** : la « granularité » de la sonde valait
  1 030–2 531 µs dans **les deux bras** (préemptions prises pour le pas de
  l'horloge, voir `cdba65db`) ; le plancher de jugement montait à 4,6 ms.
  Corrigé (pas recoupé par le TSC, tests hôte), il ne qualifie rien.
- **Mécanisme** : **inconnu**. Hypothèse écartée à la lecture : le fil
  `console` cède bien à son point coopératif (fil noyau compris). Candidats
  à mesurer : copie VGA texte (C1h) et repli (C1f), présents dans les deux
  bras.
- **À faire** : A/B console on/off KVM SMP1 sur le HEAD final.
- **Limites** : un seul run par SHA.

### C2 — Reprise d'une interactive SMP4 KVM : 1 226 µs

- **Preuve initiale** : attribution KVM SMP4 (`571b4978`, run 39563049559) :
  1 226 µs entre élection et retour ; livraison 3 µs, élection 25 µs.
- **Mécanisme** : **inconnu**.
- **Instrumentation** : `7b674b0d` (BOUCHAUD_REPRISE_DECOUPEE_V1) découpe la
  reprise en commutation, passation, retour (`/proc/self/reveil`) ; la sonde
  lit la ligne sur 1 000 octets (`c31fe38f`). Aucun correctif avant le
  segment dominant.
- **Limites** : attend les données KVM de 39602362503.

### C3 — Attente interactive jusqu'au quantum (4 ms, 13 ms vus)

- **Preuve initiale** : attribution KVM antérieure (`ed5602f4`).
- **Mécanisme** : partiel — demande différée jusqu'au point sûr ou au
  quantum ; émissions série masquées (C1).
- **Correctif de mesure** : `b1_retard` sur l'attente de la tâche (`7b674b0d`).
- **Constat** `3adcea3c` : A/B précis SMP4, tour 2 `on` `QUALIFIE_ECHEC` (pire
  571 µs contre 314 + 185) — déjà vu de façon intermittente sur `ed5602f4`,
  vert sur `571b4978` : un échantillon de chaque côté, pas de conclusion.
- **Limites** : KVM après C1 à lire.

## P1 — méthode

### C4 — A/B réveil précis non qualifiable

- **Preuve initiale** : 0/4 paires valides ; tirages dans le bruit (~100 µs).
- **Mécanisme démontré** : le bras `off` ne peut pas ouvrir la porte « la
  charge dégrade » ; comparaison sous le bruit (closeout 20/10 §6.1, §6.2).
- **Correctif** : `fc7e00b1` (BOUCHAUD_AB_CINQ_PROPRIETES_V1) — cinq verdicts
  imprimés à côté du verdict historique, qui reste **seul juge**.
- **Négative** : 12 tests dont chaque propriété qui tombe sur son défaut ;
  6 mutations détectées.
- **Première campagne** (`3adcea3c`) : SMP4 `P1=OK P2=ECHEC P3=OK P4=ECHEC
  P5=OK` ; SMP1 `P1=ECHEC P2=ECHEC P3=NON_QUALIFIANT P4=OK P5=OK`.
- **Limites** : deux campagnes de plus, puis décision humaine (porte de P3 =
  bruit au repos, §6.2 a).

## Stabilité

### C5 — SIGSEGV `compta-stress-probe` et `persist-course-probe`

- **Preuve initiale** : CI TCG, intermittent ; `persist-course` : trois fils
  en faute à `printf_core+0`, `cr2=0x45`.
- **Mécanisme** : **inconnu**. Écarté par revue : RSP utilisateur au
  `syscall`, TLS du `clone`, double cartographie d'une page à la demande.
- **Reproduction** : **non reproduit** — 45 passes os-primitives antérieures
  et 40 itérations ciblées du 21/10 (section d'os-primitives qui mène aux deux
  sondes, SMP4 TCG, deux instances simultanées + compilation : 0 mort, 0
  dépassement I2).
- **Limites** : un banc qui ne reproduit pas ne prouve rien ;
  `PROCESS_FAULT_DETAIL` attend la prochaine occurrence CI.

### C6 — Surcomptage CPU (invariant I2)

- **Preuve initiale** : `COMPTA_DEPASSE soi delta_ticks=8 borne=7` sous
  surcharge hôte (compta-stress, tâche **utilisateur**).
- **Mécanisme** : **inconnu** (recul de `utime` corrigé `37f481c4` ; C6a ne
  concerne que les fils noyau).
- **Reproduction** : non reproduit (campagne C5 ci-dessus : 0 dépassement).

### C6a — Chevauchement de tranche après idle

- **Preuve initiale** : os-primitives TCG du 21/10 : `chevauchements=1
  pire_chevauchement_us=5859`, `INVARIANT_COMPTA_ECHEC` (0 sur les deux
  images précédentes).
- **Mécanisme démontré à la lecture** : `rearme_compta_apres_idle` lisait
  l'instant et le cœur avant sa section, interruptions ouvertes au retour du
  `hlt` ; un fil **noyau** préempté dans la fenêtre était réarmé à un instant
  périmé, sur le bloc de son ancien cœur.
- **Correctif** : `3adcea3c` (BOUCHAUD_COMPTA_REARME_EN_SECTION_V1).
- **Validation après** : os-primitives TCG 3/3 (0 chevauchement), CI
  `3adcea3c` os-primitives TCG vert.
- **Limites** : **pas de reproduction négative déterministe** (une seule
  occurrence) ; à prouver par injection (élargir la fenêtre) et sous KVM.

### C7 à C12

| # | Preuve initiale | Mécanisme | Correctif | Négative | Statut |
|---|---|---|---|---|---|
| C7 Compositor vivant après fermeture | robustesse 8/9 | hypothèses non départagées | — | — | `OUVERT` |
| C8 Croissance mémoire WebContent | diagnostic rouge 2/2 sur `d02e8294` | POLLOUT incohérent corrigé (`f7ecba30`), lien non démontré | `f7ecba30` | sonde `TUBE_POLLOUT` avant 10 refus | `CORRIGE_TCG` ; vert sur `ed5602f4` et `3adcea3c` |
| C9 Panique `courant.rs:276` | endurance ancienne, isolée | inconnu | — | — | `OUVERT` |
| C10 Assertion LibJS `Value.cpp:484` | WebContent | inconnu | sonde `[LB:JSVALEUR]` | — | `OUVERT` |
| C11 `irq_depth()` toujours nul | revue du 21/10 | `irq_enter`/`irq_exit` jamais appelés ; les gardes reposent sur IF=0 | — | — | `CAUSE_CONFIRMEE`, risque latent |
| C12 Préfixe du journal : ~30 accès CMOS par ligne | revue du 21/10 | lecture RTC à chaque ligne, concurrente hors jeton | `24698aa1` | — | `CORRIGE_TCG` |

## Hors périmètre de ce registre (fonctionnalités, matériel)

SMP8 KVM (hôte ≥ 8 CPU libres), P10+.3c/d (gelé), GPU 3D, RTL8168/Trigkey,
P16/P17/P18/P19, audio HDA : suivis par `P_LOTS_REGISTER.md`, aucun
développement tant que les P0 ne sont pas clos.
