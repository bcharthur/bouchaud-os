# `docs/` — plan de la documentation

| Où | Nature | Fait foi pour l'état actuel ? |
|---|---|---|
| [`current/`](current/README.md) | documentation courante, et [`status.json`](current/status.json), la seule source des SHA et des verdicts | **oui** |
| `*.md` de ce dossier | dossiers thématiques datés (août à octobre 2026) : audits, rapports de chantier, protocoles, notes de conception | non : chacun vaut à sa date et pour son sujet |
| [`history/`](history/README.md) | anciennes documentations, rédigées pour être lues, qui ne disent plus vrai | non |
| [`historique/`](historique/README.md) | pièces de travail brutes : manifestes de lot, notes de session | non |

## Les dossiers thématiques de ce répertoire

Ils restent utiles pour comprendre **pourquoi** un mécanisme est ainsi fait. Le code et `current/` priment en cas de désaccord.

| Sujet | Documents |
|---|---|
| Vision et plans (des projets, pas des fonctionnalités) | `VISION.md`, `ROADMAP.md` (journal des versions), `BOUCHAUD_OS_0_1.md`, `ARCHITECTURE_DIRECTION.md` |
| États des lieux anciens | `ETAT_DES_LIEUX.md` (18 août 2026), `ARCHITECTURE.md` (27 août 2026) |
| Ordonnanceur et noyau | `SCHEDULER_NG.md`, `SCHEDULER_NG_BASELINE.md`, `CONTINUATION_SYNCHRONE.md`, `CYCLE_DE_VIE_TACHE.md`, `SMP_GUARD_ACTIVATION_V2.md`, `BKL_LATENCY_AUDIT.md`, `AUDIT_VERROUILLAGE_SYSCALLS.md`, `p13-reprise-20261025.md` |
| Navigateur | `BROWSER_*.md`, `WEB_ENGINE_MODULES.md`, `RENDERER_PRIVILEGE_AUDIT.md` |
| Graphique et bureau | `GUI_DAMAGE_AUDIT.md`, `GUI_USERLAND_PROTOCOL.md`, `MESURE_DEMARRAGE_A_FROID.md` |
| Réseau | `README_RESEAU.md`, `README_TLS.md`, `RTL8168_PREMIER_BOUCLAGE.md`, `REMOTE_CONTROL.md`, `BOUCHAUD_LAB_*.md` |
| CI et endurance | `CI_PALIERS.md`, `CI_PALIERS_MESURE.md`, `ENDURANCE.md`, `PORTABILITY_MATRIX.md` |
| Trigkey (relevés de septembre 2026) | `TRIGKEY_*.md`, `TRIGKEY-SMP-DOUBLE-FAULT-FIX.md` |
| Audit des chantiers (régénéré par `tools/mesure-chantiers.py --ecris`) | `AUDIT_12_CHANTIERS.md` |
