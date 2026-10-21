# Registre de convergence — défauts bloquants (ouvert le 21 octobre 2026)

Registre **fermé** : une entrée par défaut, avec sa preuve, sa cause, son
correctif, son test de non-régression et son statut. Aucun nouveau chantier
n'y entre tant que les défauts bloquants ne sont pas résolus ; aucune entrée
n'est close sur un seul run vert.

Statuts : `OUVERT` (cause inconnue) · `CAUSE_CONFIRMEE` · `CORRIGE_TCG`
(correctif + bancs TCG verts, KVM à mesurer) · `KVM_VERIFIE` · `QUALIFIE`
(critère officiel inchangé, plusieurs campagnes indépendantes) · `CLOS`.
Un `NON_QUALIFIANT` reste non qualifiant ; `P13_STRICT=0` n'est pas une
qualification stricte.

## P0 — bloquent P13

| # | Défaut | Preuve | Cause | Correctif | Non-régression | Statut |
|---|---|---|---|---|---|---|
| C1 | Console série : lignes émises interruptions masquées, jeton tenu, vidange des lignes différées de tous les cœurs | KVM SMP4 `serie_pire_us` 15–20 ms par bras (run 39503091750) ; TCG os-primitives : émission masquée pire 28,6 ms, 5 497 émissions > 2 ms, attente du jeton 67 ms | **Confirmée** : coût du port (sortie de VM par octet) payé interruptions masquées par l'émetteur | `BOUCHAUD_CONSOLE_ASYNCHRONE_V1` : anneau de 64 Kio, fil `console`, morceaux de 16 octets préemption coupée et interruptions ouvertes, enregistrement de tête reprenable, producteur qui paie quand l'anneau est plein, panique/extinction/arrêt qui vident. TCG après : 6,5 ms (démarrage seulement), 36 > 2 ms, attente 0, 0 émission synchrone une fois le fil prêt | `verifie-console-asynchrone.py` (35 négatifs), `verifie-jeton-serie.py` (22), `tools/serie/test_differees.rs` (12, dont 4 écrivains + lecteur concurrent), `run_serie_entrelacement.sh` (SMP1/2/4, détenteur suspendu), `run_console_panique.sh` (SMP1/2/4, synchrone, KVM), bras témoin `console-async off` dans l'attribution KVM | `CORRIGE_TCG` — mesure KVM à venir |
| C1a | (trouvé en corrigeant C1) anneau plein dans un contexte qui ne peut pas céder : lignes perdues | `run_console_panique.sh` : 190 lignes sur 600 perdues | repli dans l'anneau du cœur (4 Kio) | le producteur aide à vider, retente sa place après chaque essai | même banc, 600/600 | `CORRIGE_TCG` |
| C1b | (idem) ordre d'un écrivain rompu | `+ panic-test` avant des lignes plus anciennes ; 543 avant 197 | alternance anneau / lignes différées ; aide qui perdait le jeton face au fil | lignes différées seulement anneau vide (ou 1 morceau sur 32) et jamais au milieu d'un enregistrement entamé ; retente de place | banc panique ×5 SMP4 | `CORRIGE_TCG` |
| C1c | (idem) SMP1 : attente à vide de 5 s, repli de sécurité, lignes restées en attente à l'extinction | journal coupé net au milieu d'un préfixe ; `repli_securite=1`, `retard_pire_us=5010333` | contexte de démarrage (autorun) qui ne peut pas céder ; jeton tenu par le fil préempté entre deux morceaux | un contexte qui ne peut pas céder émet lui-même ; enregistrement reprenable (jeton jamais tenu hors processeur) | `run_serie_entrelacement.sh` SMP1 vert | `CORRIGE_TCG` |
| C1d | (idem) panique : second lecteur de l'anneau, puis un autre cœur écrivant pendant la vidange | SMP4 : 22 doublons, 285 lignes abîmées ; puis `NET_BOOT` mêlé par lots | chemin synchrone d'un autre cœur actif en panique | seul le cœur qui panique lit l'anneau ; il prend et garde le jeton ; les autres cœurs diffèrent | banc panique ×5 SMP4 + SMP2 + SMP1 | `CORRIGE_TCG` |
| C1e | (idem) panique SMP4 : une ligne d'un autre cœur écrite au milieu d'un enregistrement entamé | banc panique ×5 SMP4 : 1 run sur 5, `DHCP_RX_ANNEAU` au milieu de `CONSOLE_RAFALE 238` | un détenteur légitime du jeton (chemin synchrone) n'achevait pas l'enregistrement entamé si la panique venait de commencer, puis écrivait sa ligne | le détenteur achève toujours l'enregistrement entamé (il tient le jeton, le cœur qui panique l'attend jusqu'à 200 ms) | garde `verifie-console-asynchrone.py` (négatif 10 retourné), banc panique ×8 SMP4 + SMP2 | `CORRIGE_TCG` |
| C2 | Reprise d'une interactive SMP4 KVM : 1 226 µs entre élection et retour (livraison 3 µs, élection 25 µs) | attribution KVM SMP4 (571b4978, run 39563049559, passe 1 `QUALIFIE_ECHEC`) | **Inconnue** | `BOUCHAUD_REPRISE_DECOUPEE_V1` : la reprise découpée en commutation (élu → contexte repris), passation (`complete_switch_handoff`), retour (→ `dort_jusqua`), publiée par `/proc/self/reveil` (`reprise_commutation_us`, `reprise_passation_us`, `reprise_retour_us`) — instrumentation seulement, aucun correctif avant localisation | `tools/smp/test_attribution_reveil.rs` (34, dont découpe et instants incohérents) | `OUVERT` — instrumenté, KVM à mesurer |
| C3 | Préemption : une interactive prête attend jusqu'au quantum (4 ms), 13 ms vus | attribution KVM antérieure | **Partielle** : demande différée jusqu'au point sûr/quantum ; émissions série masquées mesurées | classement `b1_retard` corrigé (`BOUCHAUD_B1_RETARD_SANS_REPOS_V1` : retard mesuré sur l'attente de la tâche, plus sur l'intervalle entre battements qui comptait le repos du cœur) ; effet de C1 à mesurer | — | `OUVERT` — compteurs corrigés, KVM à mesurer |

## P1 — méthode

| # | Défaut | Preuve | Cause | Correctif | Non-régression | Statut |
|---|---|---|---|---|---|---|
| C4 | A/B réveil précis non qualifiable | 0/4 paires valides ; tirages dans le bruit (~100 µs) | **Confirmée** (closeout §6.1, §6.2) : le bras `off` ne peut pas ouvrir la porte « la charge dégrade » ; comparaison sous le bruit | `BOUCHAUD_AB_CINQ_PROPRIETES_V1` : cinq verdicts séparés (P1 précision au repos, P2 sous charge, P3 gain de la priorité, P4 pire cas, P5 équité), seuils existants inchangés, imprimés à côté du verdict historique qui reste **seul juge** (code de sortie et `AB_REVEIL_PRECIS` inchangés) ; ligne `AB_PROPRIETES_V1` | `tools/ci/reliability/test_ab_reveil_precis.py` (38, dont 12 pour les propriétés : chacune tombe sur son défaut, un `NON_QUALIFIANT` n'est jamais `QUALIFIE_OK`, l'historique n'en dépend pas ; 6 mutations détectées) | `CAUSE_CONFIRMEE` — imprimé en parallèle ; trois campagnes KVM puis **décision humaine** (porte de P3 = bruit au repos, §6.2 a) avant tout remplacement |

## Stabilité

| # | Défaut | Preuve | Cause | Correctif | Non-régression | Statut |
|---|---|---|---|---|---|---|
| C5 | SIGSEGV `compta-stress-probe` (racine 139) et `persist-course-probe` (3 fils, `printf_core`, `cr2=0x45`) | CI TCG, intermittent ; 0 reproduction locale (28 + 5 + 12 passes) | **Inconnue**. Écarté le 21/10 par revue : RSP utilisateur au `syscall` (empilé avant `sti`), TLS du `clone` (posée avant publication), double cartographie d'une page à la demande (`translate` revérifié sous `mm`) | — | `PROCESS_FAULT_DETAIL` (fs_base, tid, octets d'instruction) | `OUVERT` — campagne de reproduction sous contention à mener |
| C6 | Surcomptage CPU (invariant I2) | dépassement sous surcharge hôte | **Inconnue** (recul de `utime` corrigé `37f481c4`) | — | invariant publié par `[PROC-STAT]` | `OUVERT` |
| C6a | (trouvé par la batterie finale du 21/10) invariant 2 violé : une tranche armée 5,9 ms avant le dernier repli du même cœur | os-primitives TCG sur l'image finale : `chevauchements=1 pire_chevauchement_us=5859`, `INVARIANT_COMPTA_ECHEC` (0 sur les deux images précédentes) | **Confirmée à la lecture** : `rearme_compta_apres_idle` lisait l'instant et le cœur **avant** sa section, interruptions ouvertes au retour du `hlt` ; un fil **noyau** préempté dans la fenêtre (`accorde_preemption_noyau`) était réarmé, à sa reprise, à l'instant périmé et sur le bloc de son ancien cœur (temps hors processeur compté, bloc d'un autre cœur écrasé hors séquence). Les tâches utilisateur n'y sont pas exposées (préemption différée en mode noyau) : **n'explique pas** le I2 de compta-stress (C6) | `BOUCHAUD_COMPTA_REARME_EN_SECTION_V1` : instant et cœur lus dans la section ; une tranche déjà réarmée par l'élection n'est pas touchée (`rearmes_deja_armes`) ; site et tid du pire chevauchement publiés (`chevauchement_site`, `chevauchement_tid`) | `verifie-compta-seqlock.py` règle 6 (2 négatifs) ; invariant 2 de `verdict_os_primitives.sh` | `CORRIGE_TCG` — à confirmer sur plusieurs campagnes |
| C7 | Compositor vivant après fermeture du navigateur | robustesse 8/9 | hypothèses non départagées | — | — | `OUVERT` |
| C8 | Croissance mémoire WebContent intermittente | diagnostic mémoire rouge 2/2 sur d02e8294, vert depuis ed5602f4 | POLLOUT incohérent corrigé (`f7ecba30`) ; lien non démontré | — | diagnostic mémoire Ladybird | `CORRIGE_TCG` (stabilisé, cause non confirmée) |
| C9 | Panique `courant.rs:276` | endurance ancienne, isolée | **Inconnue** | — | — | `OUVERT` |
| C10 | Assertion LibJS `Value.cpp:484` | WebContent | **Inconnue** | sonde `[LB:JSVALEUR]` | — | `OUVERT` |
| C11 | `irq_enter`/`irq_exit` jamais appelés : `irq_depth()` vaut toujours 0 | revue du 21/10 | les gardes « pas en IRQ » ne reposent que sur IF=0 (suffisant tant qu'aucun gestionnaire ne réouvre IF : `verifie-preemption-noyau.py`) | — | — | `CAUSE_CONFIRMEE` — risque latent, pas de défaut actif |
| C12 | Préfixe du journal : ~30 accès CMOS sans verrou par ligne, jusqu'à 2 ms d'attente du bit de mise à jour | revue du 21/10 (sérialisés sous le jeton avant C1, concurrents après) | lecture RTC à chaque ligne | `BOUCHAUD_PREFIXE_SANS_CMOS_V1` : horloge murale ancrée, RTC tant que l'ancre n'est pas posée | bancs C1 | `CORRIGE_TCG` |

## Hors périmètre de ce registre (fonctionnalités, matériel)

SMP8 KVM (hôte ≥ 8 CPU libres), P10+.3c/d (gelé), GPU 3D, RTL8168/Trigkey,
P16/P17/P18/P19, audio HDA : suivis par `P_LOTS_REGISTER.md`, aucun
développement tant que les P0 ne sont pas clos.
