> **HISTORIQUE.** Relevé ou jalon daté, conservé pour mémoire : il n'est pas réévalué à chaque passe et ne décrit pas forcément le HEAD courant. État vivant : section 1 du [README](../../README.md).

# P13.1 — Hôte LAB Windows / ICS / source BRDP

Ce lot ne modifie ni RTL8168, ni DHCP/DNS/TCP/TLS de Bouchaud OS.

## Défaut physique observé

Windows pouvait afficher `SharedAccess=Running` et une configuration ICS correcte
(`Wi-Fi=PUBLIC`, `Ethernet=PRIVATE`) alors qu'aucun endpoint UDP 67/68 n'existait.
`pktmon` voyait alors les DHCP DISCOVER du Trigkey puis les rejetait avec
`transport endpoint was not found` / `Port unreachable`.

La désactivation puis réactivation du partage a recyclé le processus ICS et fait
réapparaître UDP 67/68 sur `192.168.137.1`.

## Contrat retenu

- `192.168.137.1/24` reste l'adresse normale de l'interface privée ICS.
- `169.254.6.185/16` reste l'adresse LAB locale, avec `SkipAsSource=True`.
- Les clients BRDP choisissent explicitement cette source au lieu de dépendre de
  la sélection automatique Windows.
- `BOUCHAUD_LAB_SOURCE_IP` peut vivre dans le `.env` local ignoré par Git.
- `tools/remote/windows-lab-host.ps1` vérifie/répare l'état ICS sans tuer un
  `svchost.exe` à l'aveugle.
- L'appliqueur ne bloque que si `.env.example`, `bouchaud-lab.py` ou
  `bouchaud-control.py` sont déjà modifiés. Les changements sans rapport restent
  intacts pendant l'application.

## Validation attendue

```powershell
.\tools\remote\windows-lab-host.ps1

python .\tools\remote\bouchaud-lab.py --json dhcp `
    --host 169.254.178.21 `
    --timeout 10
```

Après un retry DHCP, on attend `offres>=1`, `requests>=1`, `acks>=1`, `bail=true`
et une adresse `192.168.137.x`.
