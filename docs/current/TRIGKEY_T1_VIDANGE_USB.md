# Trigkey T1 — vidange réelle du stockage USB (BOUCHAUD_USB_VIDANGE_REELLE_V1)

Branche `claude/trigkey-t1-vidange-usb-20261104`, issue de `main@69e179a4`. Premier point du plan T1 (`docs/current/TRIGKEY_PLATFORM_PLAN.md`, branche `claude/trigkey-physical-20261103`).

Le code de mise à jour A/B (`kernel/update`) **n'est pas modifié** : il refuse toujours un commit sur un volume sans vidange réelle. C'est le pilote USB qui, désormais, peut prouver qu'il en a une.

## Le verrou

Le volume USB (`usb-bot`) déclarait `vidange_reelle: false`, et une vidange rendait `Fait(0)` sans rien émettre. C'était honnête, puisqu'une clé avec un cache d'écriture peut accepter une écriture sans l'avoir posée. Mais cela interdisait tout commit A/B sur une clé : en QEMU (xHCI et usb-storage), le slot B démarrait et son commit de santé était refusé (`B- B-`, `volume_sans_vidange_reelle`).

## Ce qui change (`src/drivers/usb/xhci_active.rs`)

- **Sonde au montage.** Après `READ CAPACITY`, le pilote émet `SYNCHRONIZE CACHE (10)` (opcode `0x35`, sans données, LBA 0, longueur 0, c'est-à-dire tout le support). `vidange_reelle` ne devient vrai **que** si le support la termine avec un CSW de statut 0. En cas de refus, le sens en attente est lu pour l'effacer, et le volume reste utilisable sans promesse de durabilité. La ligne `BOUCHAUD_USB_STOCKAGE_VIDANGE sonde=… vidange_reelle=…` le dit.
- **Vidange réelle.** `Genre::Vidange` émet la commande et attend son CSW.
- **Perte de confiance.** Un support qui refuse plus tard repasse à `vidange_reelle=0`, la vidange échoue et le commit A/B est refusé (`BOUCHAUD_USB_STOCKAGE_VIDANGE_REFUSEE`).
- **Injection** : `usbfault vidange` (shell) et la feature de banc `banc-ab-sans-vidange` (jamais dans `default`), qui fait refuser la sonde, pour le témoin négatif.

## Preuves (QEMU TCG, OVMF, `tools/ci/run_ab_rollback.sh`, nouveau `BO_AB_SUPPORT=usb`)

| Support | Scénario | Attendu | Obtenu |
|---|---|---|---|
| **USB** | `sante` | `B+ B=` | `B+ B=` (`SERVICED_HEALTH_COMMIT result=committed`) |
| **USB** | `mise-a-jour` (A installe B sur la clé, B démarre et se committe) | `A* B+ B=` | `A* B+ B=` |
| **USB** | `panique` | `B B B A A` | `B B B A A` |
| **USB** | `recovery-rollback` | `R B=` | `R B=` |
| **USB** | `echecs-repetes` | `A- A- A- R` | `A- A- A- R` |
| **USB** | `sans-vidange` (**témoin négatif**, nouveau) | `B- B- B- A= A=` | `B- B- B- A= A=` : `sonde=vidange-refus-injecte vidange_reelle=0`, `SERVICED_HEALTH_COMMIT rc=-5 result=refused` |
| NVMe | `sante` (non-régression) | `B+ B=` | `B+ B=` |

Autres vérifications :
- `tools/ci/run_usb_stockage.sh` vert, avec `BOUCHAUD_USB_STOCKAGE_VIDANGE sonde=ok vidange_reelle=1` ;
- garde-fous 209/209, `CI_POLICY_OK` ;
- 62 avertissements, comme sur `main`.

## Ce que cela ne prouve PAS

- **PHYSICAL reste NOT_STARTED.** Ces preuves sont QEMU : le usb-storage de QEMU accepte `SYNCHRONIZE CACHE`. Une vraie clé peut la refuser (le pilote le gère : pas de commit), ou l'accepter sans vider réellement son cache, ce qu'aucun logiciel ne peut détecter. La campagne Trigkey (T3, T4) doit le mesurer sur la clé choisie, avec une coupure secteur pendant l'écriture.
- Le reste du plan T1 n'est pas encore fait : image A/B physique traçable (`build_trigkey_ab_image.sh`), paquet de mise à jour hors image, garde `verifie-image-trigkey-ab.py`.
