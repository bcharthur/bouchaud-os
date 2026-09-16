# `docs/historique/` — pieces de travail, pas documentation

Ce dossier garde les **pieces brutes** d'un chantier : manifestes de lot,
notes de session, transcriptions, scripts d'epoque. Elles ont servi une fois,
a une date, et sont conservees pour pouvoir refaire le chemin.

Ne pas confondre avec **`docs/history/`**, qui est autre chose : un dossier
de documents qui DECRIVAIENT le projet, redigés pour etre lus, et qui ne
disent plus vrai aujourd'hui. Ceux-la portent le *pourquoi* d'une decision et
restent references depuis la documentation courante et depuis le code ; voir
`docs/history/README.md`.

En resume :

| Dossier            | Contenu                        | Encore reference ?        |
|--------------------|--------------------------------|---------------------------|
| `docs/historique/` | pieces de travail, notes, lots | rarement, par un commit    |
| `docs/history/`    | anciennes documentations       | oui, depuis docs/ et src/ |

Aucun des deux ne decrit l'etat actuel du projet. Pour cela : `docs/ARCHITECTURE.md`,
`docs/ETAT_DES_LIEUX.md`, `docs/ROADMAP.md`.
