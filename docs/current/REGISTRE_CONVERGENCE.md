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
| C1j | Repli de sécurité sur un fil `console` vivant, en file sur un cœur suspendu (KVM SMP4, verrou 3 s) | `CORRIGE_TCG` (négative déterministe) | CI KVM : témoin + correctif + vraie panne |
| C1k | Inversion d'ordre d'un écrivain entre lignes différées de même repère (KVM, jeton 12 s, repli) | `CORRIGE_HOTE` (négative déterministe sur la règle) | inversion non reproduite sous TCG ; CI KVM |
| C2 | Reprise SMP4 KVM : 1 226 µs | `OUVERT` | segments KVM (run 39602362503) |
| C3 | Attente interactive jusqu'au quantum | `CORRIGE_TCG` (partiel) | IPI ordinaire immédiat face au calcul utilisateur ; attente résiduelle 4 349 µs, KVM absent |
| C4 | A/B non qualifiable | `CORRIGE_TCG` | décisions du 24/10 intégrées ; trois campagnes strictes KVM au même SHA |
| C5 | SIGSEGV compta-stress / persist-course | `OUVERT` | reproduction déterministe |
| C6 | Surcomptage CPU (I2) | `OUVERT` | reproduction |
| C6a | Chevauchement de tranche après idle | `CORRIGE_TCG` (négative déterministe) | KVM |
| C7 | Compositor résiduel | `OUVERT` | preuve de mort de tous les processus |
| C8 | Croissance mémoire WebContent | `CORRIGE_TCG` (cause non confirmée) | plusieurs campagnes |
| C9 | Panique `courant.rs:276` | `OUVERT` | contention de verrous, `held_at` |
| C10 | Assertion LibJS `Value.cpp:484` | `OUVERT` | invariant violé ou corruption |
| C11 | `irq_depth()` toujours nul | `CORRIGE_TCG` | entrées/sorties et changements de pile intégrés avant `080d67ea` ; qualification KVM |
| C12 | Préfixe du journal : accès CMOS | `CORRIGE_TCG` | horodatage sous KVM |
| C13 | Tâche sensible placée sur un cœur hors ordonnanceur (mm-ng6 SMP4 : `usb-hid` 126 ms) | `CORRIGE_HOTE` (politique pure) | mesure : budget mm-ng6 répété |

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

### C1j — Repli de sécurité sur un fil `console` vivant (cœur suspendu)

- **Preuve initiale** : os-primitives `6e8bf33e` (run 39667691144, job
  119457010016), banc du verrou de l'anneau tenu 3 s, KVM SMP4 : 600/600
  intactes, mais `active=0 repli_securite=1 synchrones=1582 verrou_abandons=5`.
  `fil_somme_jeton_pire_us=3001297` : le « sommeil d'une milliseconde » du
  fil sur jeton occupé a duré 3 s ; `repli_muet_ms=2000`.
- **Mécanisme démontré** : réveillé (ou cédant à son point coopératif), le
  fil, fil noyau, est publié sur SON cœur ; c'était celui que le banc
  masquait. Un fil noyau n'est jamais volé (`runnable_steal`) : trois cœurs
  vivants à côté, personne ne pouvait le prendre ; le chien de garde a pris ce
  fil vivant pour un fil mort.
- **Reproduction négative déterministe** : `BANC_VERROU_CIBLE=fil` (fil de
  banc épinglé au cœur du fil, `spawn_noyau_epingle`), `BANC_SECOURS=off`.
  TCG SMP4 et SMP2 : `repli_securite=1`, `synchrones=1557/1569`,
  `repli_muet_ms=2501`, repli expliqué `fil_phase=2 fil_etat=0 (prêt)
  fil_file=1`, occupant du cœur 1 : `verrou-suspendu`. Même signature que le
  CI KVM.
- **Correctif** (BOUCHAUD_SECOURS_COEUR_MUET_V1 + BOUCHAUD_CONSOLE_FIL_SECOURS_V1) :
  trois situations distinguées par le chien de garde —
  * contention normale : le fil progresse, rien ;
  * cœur suspendu : fil PRÊT en file sur un cœur dont l'occupant est installé
    depuis plus de 20 ms (`SECOURS_SEUIL_NS`) : relogé sur un cœur vivant
    (`secours_tache_coeur_muet` : entrée retirée de la file du cœur muet,
    posée ailleurs ; jamais un fil noyau non migrable) ;
  * vrai blocage : silence > 2 s → soupçon, fil réveillé ; toujours muet
    0,5 s plus tard → repli définitif (borne 2,5 s). Le soupçon distingue
    aussi une machine entière suspendue (SMP1).
  Politique pure testée sur l'hôte (`tools/smp/test_secours_coeur_muet.rs`,
  propriété 50 000 tirages) ; garde `verifie-console-asynchrone.py` règle 13,
  7 négatifs (67 au total).
- **Validation après (TCG)** : cœur du fil suspendu 3 s, SMP4 et SMP2 :
  `secours=1`, relogé après 20–22 ms, aucun repli, 600/600 ; 12 s SMP4 :
  relogé après 21 ms, aucun repli ; SMP1, machine entière suspendue 3 s :
  `soupcons=1`, levé, aucun repli.
- **Vraie panne** (`BANC_FIL_GELE_MS=6000`, le fil cesse de progresser 6 s ;
  producteurs lents, 80 ms par ligne, l'anneau ne se remplit jamais) : repli
  expliqué (`fil_phase=5`), borné — silence 2 522–2 609 ms, confirmation
  507–573 ms —, 180/180 lignes entières par le chemin synchrone, SMP4, SMP2,
  SMP1, deux fois chacun. Fil gelé 6 s sous le flot ordinaire (anneau plein) :
  les producteurs vident eux-mêmes, la console progresse, **aucun** repli
  (600/600) — ce n'est pas une panne. Premier essai du banc avec le flot
  ordinaire puis avec 100 lignes × 20 processus : l'anneau se remplissait par
  les journaux de fautes du brasseur (68 Kio pour 20 processus), aucun repli —
  le banc ne testait pas la panne ; corrigé (pas de 80 ms, 4 processus).
- **Limites** : la règle « cœur muet » couvre aussi une contention extrême
  (occupant > 20 ms alors qu'une tâche attend > 20 ms) : vu une fois sous
  TCG SMP4 pendant le gel (fil relogé, sans effet sur le verdict). Seule la
  console appelle le secours : l'équilibrage ordinaire et sa règle de ne pas
  voler un fil noyau sont inchangés. KVM à mesurer en CI.

### C1k — Inversion d'ordre entre lignes différées de même repère

- **Preuve initiale** : os-primitives `63b519a8` (run 39740388363), KVM SMP4,
  détenteur du jeton suspendu 12 s : `inversions=1` — « écrivain 1, ligne
  0052 après 0053 » —, `lignes_differees=12`, console repliée (C1j :
  `fil_somme_jeton_pire_us=12004179`).
- **Mécanisme démontré** : une ligne différée porte comme repère la position
  d'écriture de l'anneau ; console repliée, cette position ne bouge plus et
  toutes les différées ont le **même** repère. L'égalité se départageait par
  l'ordre des cœurs (`repere < r` strict, parcours des cœurs dans l'ordre) :
  un écrivain qui change de cœur entre deux lignes différées les voit sortir
  inversées.
- **Correctif** (BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V2) : chaque différée porte
  aussi un numéro d'ordre global (16 octets d'en-tête) ; la plus ancienne mûre
  sort d'abord — plus petit repère, puis plus petit numéro. Règle pure
  `differees::plus_ancienne_mure`.
- **Négative** : `tools/serie/test_differees.rs` — le cas exact (52 sur le
  cœur 2, 53 sur le cœur 1, même repère) et une propriété (2 000 tirages,
  écrivains qui migrent, anneau figé ou non) ; tous deux **rouges** sur la
  règle d'avant (repère seul, ordre des cœurs). Garde
  `verifie-console-asynchrone.py` règle 6 : 2 négatifs de plus (69).
- **Validation après (TCG)** : batterie console entière verte (paniques
  2 000 SMP4 ×5/SMP2/SMP1, verrou 3/12 s, jeton 12 s, saturation 5 µs on/off,
  défaut SMP1/2/4).
- **Limites** : sous TCG, console off + jeton suspendu 6 s (9–12 différées)
  n'a pas reproduit l'inversion (il faut une migration entre deux différées
  d'un même écrivain) ; la preuve négative est au niveau de la règle.

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
- **Mesure TCG du 24/10** : pire réveil interactif brut `on` 3 355 µs
  (`080d67ea`) puis 3 667 µs après `25c0c2ba` : C1g reste **OUVERT**.
- **Limites** : un seul run par SHA.

### C2 — Reprise d'une interactive SMP4 KVM : 1 226 µs

- **Preuve initiale** : attribution KVM SMP4 (`571b4978`, run 39563049559) :
  1 226 µs entre élection et retour ; livraison 3 µs, élection 25 µs.
- **Mécanisme** : **inconnu**.
- **Instrumentation** : `7b674b0d` (BOUCHAUD_REPRISE_DECOUPEE_V1) découpe la
  reprise en commutation, passation, retour (`/proc/self/reveil`) ; la sonde
  lit la ligne sur 1 000 octets (`c31fe38f`). Aucun correctif avant le
  segment dominant.
- **Mesure TCG du 24/10** : reprise interactive ventilée maximale
  3 823 → 291 µs lors de la mission précédente, puis 978 µs avec
  `25c0c2ba`. Deux passes qualifiées mais progrès non monotone : **OUVERT**.
- **Limites** : aucune nouvelle preuve KVM ; les retards inexpliqués ne
  sont pas attribués à l'hôte.

### C3 — Attente interactive jusqu'au quantum (4 ms, 13 ms vus)

- **Preuve initiale** : attribution KVM antérieure (`ed5602f4`).
- **Mécanisme** : partiel — demande différée jusqu'au point sûr ou au
  quantum ; émissions série masquées (C1).
- **Correctif de mesure** : `b1_retard` sur l'attente de la tâche (`7b674b0d`).
- **Constat** `3adcea3c` : A/B précis SMP4, tour 2 `on` `QUALIFIE_ECHEC` (pire
  571 µs contre 314 + 185) — déjà vu de façon intermittente sur `ed5602f4`,
  vert sur `571b4978` : un échantillon de chaque côté, pas de conclusion.
- **Correctif livré** `25c0c2ba` : une interactive sous budget publiée
  derrière du calcul utilisateur normal envoie un IPI ordinaire ; la
  politique antérieure ne posait que `need_resched`. Les pairs interactifs,
  le budget, les continuations BSP et la préemption noyau gardent leurs
  exclusions. Un IPI par publication, aucune relance au refus.
- **Validation** : 39 tests de politique et gardes négatifs ; attribution
  TCG SMP4, deux passes : `election_inexpliquee` maximale 7 871 → 4 349 µs ;
  attente utilisateur diagnostique 8 158 → 4 490 µs. Les 7 871 µs ne sont
  pas entièrement expliquées par le mécanisme corrigé.
- **Limites** : attente résiduelle, reprise variable, pas de clôture KVM.

## P1 — méthode

### C4 — A/B réveil précis non qualifiable

- **Preuve initiale** : 0/4 paires valides ; tirages dans le bruit (~100 µs).
- **Mécanisme démontré** : le bras `off` ne peut pas ouvrir la porte « la
  charge dégrade » ; comparaison sous le bruit (closeout 20/10 §6.1, §6.2).
- **Correctif livré** : décisions du 24 octobre : `on` doit qualifier ;
  `off` est un contrôle négatif ; P3 n'est comparable qu'au-dessus du bruit
  mesuré au repos. Le verdict historique reste publié, pas effacé.
- **Validation TCG** : `080d67ea`, 8/8 bras, P1–P5 verts, P2 488/556,5 µs.
  Après `25c0c2ba` : 8/8 bras, P1–P5 verts, P2 574/594,5 µs (−3,45 %).
  Seuil P2 1,10 inchangé ; ce second résultat n'établit pas un gain absolu
  du p95 par rapport au premier.
- **Limites** : le job `primitives.ordonnanceur-ab` du run 40129187265 est
  rouge à l'étape A/B KVM SMP1/SMP4 (sortie 1). Journal inaccessible depuis
  le cloud : API Forbidden, interface publique « Sign in to view logs ».
  Diagnostic communiqué ensuite par l'opérateur : SMP1 tours 3/4 `on`,
  aucun armement précis ; P1 p99 3 631,5 µs contre 1 630 µs. SMP4 P2
  65,5/43 µs (+52,3 %) ; tour 3 P3/P4 rouges, pire interactif propre
  1 093 µs contre 470 + 158 µs. Ce sont des rouges KVM réels, non effacés
  par les succès TCG. Les compteurs détaillés avant/après restent absents ici.

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
- **Scénario utilisateur** `3f6289dc` : 200 cycles calcul/syscall/sommeil,
  concurrent utilisateur ; la borne retire 400 ms de sommeil demandé du
  temps mur. SMP4 TCG : 14 ticks consommés, borne 26 ; puis stress de 8 s,
  7 532 lectures, 0 recul, 0 dépassement. Aucun correctif comptable déduit
  artificiellement de ce succès : C6 reste **OUVERT, non reproduit**.

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
- **Négative déterministe** (BOUCHAUD_COMPTA_INJECTION_V1,
  `tools/ci/run_compta_injection.sh`) : `compta-injection 2000 ancien|correctif`
  élargit la fenêtre `hlt` → réarmement (2 ms interruptions ouvertes, puis
  `schedule()`), pendant `compta-stress-probe 8`. `ancien` rejoue l'ordre
  d'avant `3adcea3c`. Mesures (TCG, même image, deux démarrages par ligne) :

  | Banc | `ancien` : chevauchements (pire) | `correctif` | Réarmements évités (`correctif`) |
  |---|---|---|---|
  | SMP2 (22/10, 1ʳᵉ série) | 13 (48 329 µs) | 0 | 11 |
  | SMP2 (répétition) | 16 | 0 | 13 |
  | SMP4 | 32 | 0 | 40 |
  | SMP2 (image finale) | 9 (196 383 µs), `COMPTA_STRESS_ECHEC` (`COMPTA_DEPASSE global` ×2) | 0, `COMPTA_STRESS_OK` | 10 |
  | SMP4 (image finale) | 23 (31 134 µs) | 0, `COMPTA_STRESS_OK` | 30 |
  | SMP1 | `NON_QUALIFIANT` : `fenetres=0` (aucun réarmement avec tâche courante) | — | 0 |

  L'ordre d'avant viole aussi l'invariant I2 **global** sous injection (SMP2) ;
  il ne départage pas C6 (`COMPTA_DEPASSE soi`, tâche utilisateur), qui reste
  ouvert. Injection inerte par défaut (fenêtre 0, ordre corrigé), posée par le
  seul réglage `regle_injection_compta` : `verifie-compta-seqlock.py` règle 7,
  9 négatifs. Banc ajouté à os-primitives (TCG SMP2 et SMP4 ; code 2 =
  `NON_QUALIFIANT`, rouge).
- **Limites** : pas encore mesuré sous KVM ; SMP1 non qualifiable par ce banc.

### C7 à C12

| # | Preuve initiale | Mécanisme | Correctif | Négative | Statut |
|---|---|---|---|---|---|
| C7 Compositor vivant après fermeture | robustesse 8/9 | hypothèses non départagées | — | — | `OUVERT` |
| C8 Croissance mémoire WebContent | diagnostic rouge 2/2 sur `d02e8294` | POLLOUT incohérent corrigé (`f7ecba30`), lien non démontré | `f7ecba30` | sonde `TUBE_POLLOUT` avant 10 refus | `CORRIGE_TCG` ; vert sur `ed5602f4` et `3adcea3c` |
| C9 Panique `courant.rs:276` | endurance ancienne, isolée | inconnu | — | — | `OUVERT` |
| C10 Assertion LibJS `Value.cpp:484` | WebContent | inconnu | sonde `[LB:JSVALEUR]` | — | `OUVERT` |
| C11 `irq_depth()` toujours nul | revue du 21/10 | suivi absent des handlers et des changements de pile | intégré avant `469d8c76`, conservé sur `080d67ea` | gardes IRQ et banc contexte SMP1/4 | `CORRIGE_TCG`, KVM à qualifier |
| C12 Préfixe du journal : ~30 accès CMOS par ligne | revue du 21/10 | lecture RTC à chaque ligne, concurrente hors jeton | `24698aa1` | — | `CORRIGE_TCG` |
| C13 Tâche sensible placée sur un cœur hors ordonnanceur | Integration PR 39740393249 (63b519a8, mm-ng6 SMP4) : `ready_latency_interactive_max_ms` 126,5 > 100 (p99 33,6, écart isolé ; le push du même SHA vert) ; `[SCHED-NG-PIRE] classe=interactive tid=104 cpu=0 pret 2471 ms élu 2598 ms`, `[SCHED-NG-ATTENTE-VIVE] cpu=0 cur_tid=0 idle=0` | le cœur zéro exécutait le shell de l'autorun hors de toute tâche ; le placement des tâches sensibles le classait « tâche de fond, peu coûteux » (rang 1) et y posait `usb-hid` (sensible), qui n'y est élu qu'au retour du shell | BOUCHAUD_PLACEMENT_HORS_ORDONNANCEUR_V1 : un cœur ni endormi ni muni d'une tâche est le pire rang (4), retenu seulement à défaut de tout autre | test hôte `un_coeur_hors_ordonnanceur_n_est_retenu_qu_a_defaut` (rouge sans le correctif) | `CORRIGE_HOTE` ; limites : seulement pour les tâches sensibles sous budget ; un dépassement de budget ou une tâche non sensible garde le placement historique |

## Hors périmètre de ce registre (fonctionnalités, matériel)

SMP8 KVM (hôte ≥ 8 CPU libres), P10+.3c/d (gelé), GPU 3D, RTL8168/Trigkey,
P16/P17/P18/P19, audio HDA : suivis par `P_LOTS_REGISTER.md`, aucun
développement tant que les P0 ne sont pas clos.

## Qualification du candidat du 24 octobre 2026

Les cinq workflows CI Fast, Integration, Reliability V3, Native IPC Runtime
et Console sont verts sur `080d67ea`. Le run canonique
[40129187265](https://github.com/bcharthur/bouchaud-os/actions/runs/40129187265)
porte un rouge confirmé `primitives.ordonnanceur-ab` ; la compilation
Ladybird et `primitives.probes` étaient encore en cours au contrôle.
`primitives.gpu-tampon` est vert. Aucun scénario actuel réussi ne ferme C7
ou C10 ; un journal inaccessible n'est pas un diagnostic de crash.

Le lanceur `tools/ci/run_p13_campaigns.sh FULL_SHA [SORTIE_NEUVE]` crée un
checkout isolé du commit, construit l'image, vérifie son hash et conserve
quinze verdicts stricts dans trois campagnes séparées. Baseline : quatre
CPU alloués ; SMP8 : huit. Accès KVM testé par ioctl et création de VM,
quota et charge contrôlés avant chaque campagne. Aucun TCG de substitution.
Commande depuis un checkout propre, sur Linux équipé de rustup, musl-tools,
build-essential et qemu-system-x86 :

```sh
BOUCHAUD_DEDICATED_HOST=YES bash tools/ci/run_p13_campaigns.sh "$(git rev-parse HEAD)"
```

Cloud courant : affinité 5 CPU, quota 4 CPU, `/dev/kvm` absent ; précontrôle
refusé. Inventaire des runners distant refusé par l'API. Les tests hôte de
l'orchestration utilisent des fixtures explicitement fictives, jamais des
preuves KVM. P13 reste **PARTIAL**.

### Correctifs après le diagnostic KVM communiqué

Le réarmement « occupé » est désormais exclu pendant l'idle ; le chemin
avant HLT garde la responsabilité des échéances de sommeil. Le timer idle
utilise une borne par CPU, au lieu de faire armer la même borne globale par
tous les AP. Le balayage reste global, avec fetch_min pour conserver les
armements concurrents et les alarmes du BSP. Un IPI ordinaire n'est plus
réémis vers le CPU dont l'IRQ de préemption sert déjà cette publication.
Aucun compteur d'armement ni critère d'analyse n'a été modifié pour faire
passer une mesure.

Un relevé ponctuel `REVEIL_TIMER_LOCAL` montre les vrais registres LVT et
TSC-deadline du CPU de la commande, distincts du mode logiciel déclaré.
Les annotations du job canonique publient les verdicts et les compteurs
existants, y compris en cas d'échec. La disparition des armements KVM et
P2/P3/P4 ne seront déclarés corrigés qu'après ces mesures sur le nouveau SHA.

Attribution TCG après ces changements : deux passes qualifiées en trois
tentatives ; maximum `election_inexpliquee` 2 964 µs, reprise ventilée
1 755 µs (toutes tentatives conservées). L'attente baisse, la reprise reste
irrégulière : C1g/C2 ne sont pas clos et aucune suspension hôte n'est prouvée.

A/B TCG SMP1 après `8777274c` : 476/555 µs (on/off, −14,23 %),
8/8 bras valides, P1–P5 verts. Le pire réveil brut interactif `on` vaut 8392 µs.
Ces valeurs ne remplacent pas les échecs KVM du SHA précédent.

### Retour KVM sur `7e5943f6` et coût de l'identité CPU

Run canonique `40132770745`, job `120987948156` : le défaut d'armement n'est
plus reproduit, 8/8 bras valides sur SMP1 et SMP4. Les relevés matériels LVT
valent `0x400f1` (TSC-deadline), avec un MSR deadline programmé. Cela ne clôt
pas la latence : SMP1 P1 p99 `on=5514,5 / off=4233 µs` est rouge ; P2 reste
rouge sur SMP1 (`65,5 / 45,5 µs`) et SMP4 (`165,5 / 123 µs`). P3/P4/P5
passent selon les décisions adoptées ; les contrôles non comparables ne
deviennent pas des preuves de priorité. Aucun seuil n'a changé.

Le chemin `smp::cpu_index()` contournait la primitive commune et relisait
les MSR GS, y compris dans les opérations répétées du timer précis. Il
utilise désormais le chemin TSC_AUX vérifié de `usermode::cpu_index()` ; les
replis GS/APIC d'amorçage sont conservés. Six mutations négatives protègent
cette délégation et la validation de l'identité. Son effet sur P1/P2 doit
encore être mesuré sous KVM. Les A/B précis sont exécutés avant les A/B
expérimentaux dans le même job ; aucune épreuve n'est supprimée.

Les annotations KVM longues ont été tronquées dans la vue publique après
le début du tour 3. Leur publication est maintenant découpée sans perte,
avec les quantiles et l'attribution par bras ; l'analyseur reste inchangé.

Compléments TCG sur le code de `9e662f7c` : SMP1 8/8 bras valides,
P2 `498,5 / 482 µs`, pire interactif brut `on=9097 µs` ; SMP4
**NON_CONCLUANT**, 6/8 bras valides (deux contrôles non exploitables), malgré
quatre bras `on` qualifiés. L'attente d'élection inexpliquée interactive
`on` remonte à 4584 µs dans cet A/B : les 2964 µs de l'attribution précédente
ne constituent pas une borne stable.

Le scénario C6 utilisateur SMP1/4 passe sous KVM dans le run `40132770745`,
ainsi que les trois démarrages de primitives et les injections C6a. C5 reste
non reproduit (nouveau banc TCG multifil 24/24). Sur `9e662f7c`, CI Fast,
Integration, Reliability V3 et Console Sync Proof sont verts. La console
donne 4/4, 600/600 lignes, `secours=1 relogees=1`, CPU 3 → 1
(run `40133365331`), mais le secours atteint 1495 ms. Un replay du run
`40132770698` avait échoué avec le même code noyau ; son journal inaccessible
ne permet pas d'en attribuer la cause. La qualification stricte et la
convergence Ladybird restent ouvertes.

### Candidat `44080fab` : résultats KVM et corrections suivantes

Run canonique `40134718112`, job `120993791199` : 8/8 bras valides en
SMP1 et SMP4, sans défaut d'armement. Le chemin d'identité CPU rapide réduit
la médiane au repos, mais ne clôt pas les régressions :

| Mesure KVM (on / off, µs) | SMP1 | SMP4 |
| --- | --- | --- |
| P1 médiane au repos | 29,5 / 451 | 30,5 / 404 |
| P1 p95 au repos | 51,5 / 975,5 | 63 / 954 |
| P1 p99 au repos | 4415 / 3181,5 — **ÉCHEC** | 230,5 / 1029 — OK |
| P2 p95 interactif | 70 / 21,5 — **ÉCHEC** | 106,5 / 73,5 — **ÉCHEC** |

P3/P4/P5 passent selon les décisions inchangées ; P3 ne dispose que de
quatre bras comparables sur huit. Les quatre contrôles dans le bruit ne
sont pas présentés comme des comparaisons qualifiées. C1g demeure hors du
sommeil : en SMP1, les pires attributions au repos restent vers 1,1 ms
alors que les quantiles bruts atteignent plusieurs millisecondes.

Sur ce SHA, CI Fast, Reliability V3, Integration, Native IPC et Console
Sync Proof passent. La console CI donne 4/4, 600/600 lignes par cas et
`secours=1 relogees=1`, CPU 1 → 0, secours maximal 84 ms
(run `40134718158`). C6 utilisateur KVM SMP1/4 et C6a passent aussi.
La compilation Ladybird n'était pas terminée ; aucun nouveau résultat
Compositor ne peut être déduit de ces succès.

Les mesures TCG ne ferment pas la latence : l'A/B SMP1 du code
`44080fab` est NON_CONCLUANT (6/8 bras valides) ; l'attribution SMP4 donne
une passe QUALIFIE_ECHEC, une NON_QUALIFIANT, avec une attente d'élection
inexpliquée jusqu'à 16281 µs. Aucun retard n'est attribué à l'hôte sans
preuve. L'A/B SMP4 après regroupement des lectures du registre reste
NON_CONCLUANT (5/8 bras valides).

Corrections supplémentaires, dont l'effet KVM reste à vérifier :

- Le recalcul des échéances utilise une vue du registre, au lieu de
  reprendre un garde pour chaque champ. Le temporaire de l'ancienne
  expression `for … tasks().len()` protégeait déjà le scan entier : la
  nouvelle vue conserve cette portée et tombe avant le verrou des alarmes.
- La conversion nanosecondes → TSC arrondit au premier cycle où l'horloge
  atteint l'échéance. Le test avec des fréquences non entières en GHz
  échoue sur l'ancienne conversion et passe avec le correctif (19 tests).
- Un tir anticipé sans candidat ne fabrique plus de demande différée en
  contexte noyau. Les demandes ordinaires/ciblées, les échéances encore
  dues après un scan refusé et le retrait des tâches condamnées en ring 3
  restent servis. Les seuils et le domaine de préemption sont inchangés.
- Un nouveau rouge console local est reproduit : sans secours, tous les
  producteurs pouvaient attendre six secondes dans `range_console` sans
  consulter le chien de garde (`soupcons=1 repli=0`). La boucle coopérative
  le consulte désormais IRQ ouvertes, hors verrou, et sort vers le repli
  s'il est confirmé. Les quatre replays suivants passent, 600/600 chacun :
  contrôle négatif `repli=1`, confirmation 500 ms ; secours actif
  `secours=1 relogees=1`, CPU 3 → 1, 20 ms. Ce diagnostic local n'attribue
  pas rétrospectivement le rouge CI dont le journal reste inaccessible.

Les annotations publiques sont plafonnées à dix notices par étape : le
run `40134718112` n'a publié que trois des sept blocs SMP4. Chaque topologie
a maintenant sa propre étape de publication, et l'A/B précis son propre
job, sans suppression des A/B expérimentaux. Les analyseurs et leurs
seuils restent inchangés. Le verdict P13 reste **PARTIAL**.

### `7ad8c543` : deux A/B KVM encore rouges

Les runs `40136548809` (canonique) et `40136589584` (déclenchement manuel
indépendant) portent le même SHA. P2 demeure rouge : p95 interactif on/off
SMP1 `109/60,5` puis `69,5/44 µs`, SMP4 `178,5/140,5` puis
`157,5/111,5 µs`. P1 SMP1 échoue dans le canonique (`p99=5014,5/3511,5`),
passe dans le manuel (`5299,5/4316`) ; cette variation ne clôt pas C1g.

Le canonique reproduit aussi **P4 SMP4, tour 2 on** : pire interactif
1131 µs contre normal 449 µs et repos 204 µs, 300/300 échantillons propres.
L'attribution mesure 1020 µs de reprise, dont 1019 µs dans le segment
commutation. Le verdict global reste rouge : le bilan calculé sur les
sept autres bras ne peut effacer ce huitième bras en échec. Le manuel
passe P4 ; ce succès indépendant n'efface pas non plus la régression.

La console CI `40136548810` passe 4/4, 600/600 lignes par cas, secours
CPU 2 → 0 en 20 ms et contrôle sans secours replié après confirmation de
500 ms. C6 utilisateur KVM SMP1/4 et C6a passent dans les deux runs.
L'attribution TCG obtient deux passes qualifiées en trois tentatives ; en
conservant la tentative non qualifiante, l'attente d'élection inexpliquée
atteint 7108 µs et la reprise 1260 µs. C2/C3 restent ouverts. Le nouveau
multifil TCG passe 24/24 sans reproduire C5.

Le correctif suivant supprime les auto-IPI pendant les publications du
balayage déjà suivi d'une élection locale. Le précédent ne couvrait que
`DemandeAvecIpi` dans le balayage IRQ : `ReveilImmediat`, `PreemptionCiblee`
et le balayage de `schedule()` pouvaient encore notifier le CPU qui était
déjà en train de servir la demande. Le contexte est borné par un garde,
restauré après publication ; les demandes restent posées et les IPI vers
un autre CPU restent envoyés. `auto_ipi_evites` rend l'exécution observable.
Les 40 tests de politique et 36 mutations du garde du réveil passent ;
l'effet sur P2/P4 exige une nouvelle mesure KVM, sans modifier l'analyseur.

Validation locale du correctif d'auto-IPI : BIOS/UEFI compilent, contexte
SMP1/SMP4 passe et observe respectivement 7/3 notifications évitées. C6
utilisateur SMP4 passe 200 cycles, 10 522 lectures, zéro régression ou
dépassement. Les quatre replays TCG passent, 600/600 lignes chacun ;
le secours actif reloge effectivement une tâche (`secours=1 relogees=1`,
20 ms), le contrôle négatif confirme son repli après 500 ms.
Ces résultats ne constituent pas une qualification KVM de P2/P4.

### `a2b5e594` : auto-IPI supprimes, latence encore rouge ; attente NVMe reproduite

KVM `40137907568`, job `121003485608` : tous les armements `on` existent,
mais P1 SMP1 reste rouge (`p99 on/off=4842/3286 us`), ainsi que P2
(`62,5/36,5 us` SMP1, `149/102 us` SMP4). P4 SMP4 tour 2 on echoue encore :
788 us contre normal 329 + repos 202 us, 300/300 propres. Cette fois,
l'attribution du pire reveil place 770 us dans l'attente d'election, 2 us
dans la reprise. La baisse absolue de certaines mesures ne respecte pas
encore les criteres relatifs ; aucun defaut de latence n'est clos.

La console CI `40137907585` reste 4/4, 600/600, secours CPU 3 -> 0 en 20 ms.
L'attribution TCG SMP4 passe 2/2 sans tentative ecartee, attente d'election
inexpliquee maximale 3587 us et reprise 287 us. Mais l'A/B TCG SMP4 conserve
un P4 rouge (pire 16209 us, election inexpliquee 7897 us) ; l'A/B TCG SMP1
qualifie ses huit bras mais echoue P2 (496/409 us). Ces mesures differentes
restent toutes conservees, sans substitution des seules passes vertes.

Integration `40137907516` echoue sur la profondeur NVMe, apres les montages
i440fx/q35 reussis. Le rouge est reproduit au deuxieme de trois demarrages
locaux : `fini=0 reussies=24 echecs=0 libres=4`, un lecteur Ready sur CPU0
avec `cur_tid=0 idle=0`, expiration de 30 s. Le RIP interrompu designe le
predicat de `sonde_parallele` dans `attente_bornee`. La continuation BSP
attend activement le lecteur qu'elle empeche d'elire ; aucun emplacement
NVMe n'est perdu dans cette reproduction.

La sonde utilise maintenant WaitQueue, dont le parking passe par le contexte
centralise `avec_tache`. Le dernier lecteur publie sa fin puis reveille
l'attente ; le ticket precede la relecture, et une expiration ne permet
plus de remettre a zero les comptes de lecteurs encore vivants. Le pilote
conserve ses attentes materielles bornees, et le banc tous ses criteres.
Apres correction : BIOS/UEFI compilent ; trois demarrages SMP4, neuf passages
32/32 lectures, profondeur 4 atteinte a chaque demarrage, zero attente
faute d'emplacement ; SMP1 termine aussi trois passages 32/32 (sans utiliser
ce dernier essai comme preuve de profondeur concurrente). Le garde NVMe
refuse quatre mutations qui retirent parking, ticket, exclusion ou reveil.
Ce correctif local n'attribue pas sans journal detaille le rouge CI exact.
Les changements bloc declenchent desormais aussi la chaine canonique
Ladybird ; Integration publie les comptes NVMe sans modifier son verdict.

### Reprise du 25 octobre : cout des scans et correction du diagnostic IPI

Le SHA distant `45247ce15fb3dd967bec86f12b67980dc98be24e` contient bien le
correctif NVMe. Integration `40138825247`, console `40138824946`, CI Fast et
Reliability sont verts. KVM `40138824904` reste rouge : P2 SMP1 62/35,5 us,
SMP4 154,5/102,5 us ; P4 SMP4 tour 3 on 1150 > 367 + 152 us. L'attribution
situe ce dernier retard surtout avant Ready (1053 us), puis 87 us avant
l'election et 1 us en reprise. Cela ne prouve pas encore une cause racine.

**Rectification des paragraphes precedents** : reschedule_cpu excluait deja
les destinations locales avant l'ecriture APIC. `auto_ipi_evites` ne compte
pas des interruptions materielles economisees, mais des appels logiciels
elides. L'hypothese d'auto-IPI materiels imbriques expliquant P2 est retiree.
Le nom historique du compteur reste pour comparer les journaux. Les tirs
precis et anticipes se recoupent et ne doivent pas etre additionnes.

Le candidat 291d6b4d (retire apres mesure ci-dessous) supprime les prises imbriquees du registre dans deux
scans, sans allonger leur portee et sans changer le reveil. Un profileur
opt-in mesure les operations reelles dans des boots distincts de la
qualification. Le temps de suspension d'une continuation IRQ est exclu du
segment mesure. L'identite du dormeur est publiee hors serie chronometree ;
`reveille_par` reste l'auteur du reveil. La branche d'audit demande maintenant
explicitement p13_strict=true au workflow reutilisable.

Validation avant push : 40 tests de politique, 19 tests du timer, 59
verifications C++ du verdict, 38 tests Python A/B et 43 mutations du garde.
BIOS/UEFI et contextes SMP1/SMP4 passent, C6 utilisateur passe 9852 lectures
sans recul/depassement, console 4/4 avec relogement reel. L'A/B TCG SMP1
qualifie 8/8 bras et passe P1-P5 : P2 on/off 378/434 us (precedent essai
496/409 us). Le diagnostic TCG se termine avec les compteurs attendus ;
ses mesures ne qualifient pas P13. La preuve KVM du candidat reste requise.

Comparaisons exhaustives de reference, hypotheses et commandes de reprise :
[document de reprise du 25 octobre](../p13-reprise-20261025.md).

Le TCG SMP4 du meme candidat reste NON_CONVAINCANT, 6/8 bras valides : P4
tour 1 on 6046 > 3694 + 2173 us et tour 1 off non qualifiant. L'attribution
conserve un pire sommeil brut 18157 us, dont 17962 avant election, 126 de
reprise ; pid32/tid128, CPU0, reveille hors IRQ par pid42/tid138. Cette pointe
brute est plus haute que la precedente et n'est pas escamotee. Les quantiles
sur les quatre couples complets sont meilleurs (p95 on/off 534,5/518,5 contre
580/509,5), mais seuls trois couples restent valides : aucune qualification
SMP4 ni stabilisation C3 n'est annoncee. Le KVM du candidat doit trancher.


### Retrait du candidat 291d6b4d apres mesure KVM stricte

Run 40142054551, job 121015675494 : A/B et qualificateur strict rouges.
P2 SMP1 58/16,5 us, SMP4 102,5/63,5 us : baisse absolue mais toujours hors
contrat. P1 SMP1 echoue ; P3 SMP4 echoue et P4 SMP4 compte trois bras on
en echec, contre un sur 45247ce1. Seulement 5/8 bras valides en SMP4.
Console CI 40142054544 : 3/4, temoin secours off avec 1218 lignes differees
perdues ; le secours on reloge bien (secours=1, relogees=1), sans perte.
Le candidat est retire par commit inverse, en conservant ses preuves et
le correctif NVMe de reference. Aucune causalite individuelle des changements
n'est pretendue. Restent le commentaire IPI corrige, p13_strict=true sur
la branche et la concurrence canonique par SHA qui conserve Ladybird.

Le profilage KVM separe observe 47100 ecritures deadline sous on contre
4295 sous off en SMP1, environ 5 us par ecriture. P4 tour3 SMP4 du candidat
attribue 1060 us a la commutation. Ce sont des pistes mesurees, pas encore
une cause racine prouvee de P2/P4. Comparaisons de tous les bras, limitations,
preuves et prochaines commandes : docs/p13-reprise-20261025.md. P13 PARTIAL.

Le retour compile BIOS/UEFI, passe contextes SMP1/4, C6 utilisateur (8200
lectures sans recul/depassement) et les quatre replays TCG (600/600 chacun,
dernier secours=1, relogees=1). L'isolation par SHA sur la branche d'audit
est etendue au workflow reutilisable pour ne pas annuler les preuves en
cours lors du retrait. Aucune nouvelle qualification KVM n'est presume.
