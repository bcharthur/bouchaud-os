# M12 — Modes d'usage Atelier (premiere tranche reelle)

Dans le centre de controle, les profils Standard, Travail, Focus et Jeu
affectent les preferences *effectives* d'animations et du diagnostic FPS.

| Mode | Animation | FPS |
|---|---|---|
| Standard | normale | masque |
| Travail | reduite, sans translation | masque |
| Focus | aucune | masque |
| Jeu | normale | visible |

Les reglages peuvent aussi etre exportes avant la session avec
`BO_ATELIER_USAGE=standard|travail|focus|jeu`. Les preferences explicites
`BO_ATELIER_ANIMATIONS` et `BO_ATELIER_DIAGNOSTIC` ont toujours priorite,
quel que soit l'ordre d'export. Une modification manuelle indique alors
`Personnalise` plutot que de pretendre que le profil initial reste en vigueur.

Le journal de changement porte `scope=VISUEL`. **Aucune** politique CPU,
GPU, services, reseau, energie ou notification n'est encore appliquee ;
aucune restauration de session n'est annoncee. Le mode est local a la
session et n'est pas encore persiste. Les automatisations restent explicites
et soumises au choix de l'utilisateur.

Preuve hote : `tools/gui/test_atelier.rs` inclut les vrais modules du noyau,
teste les profils et les cibles clic/survol ainsi que la geometrie 640x480.
La preuve d'execution interactive QEMU reste a ajouter avant une qualification
produit. Le but est une progression honnête du chantier #12.
