# Bouchaud Lab — QEMU BRDP / telemetry E2E

Base attendue :

```text
branche : claude/ladybird-observability-performance
HEAD    : 6cea9568a3366cb2d87b1140cac48edd81851e87
```

## Pourquoi ce patch existe

Le premier vrai essai TAP/QEMU a prouve que le canal LAB fonctionnait :
telemetrie UDP recue par l'hote, `discover` fonctionnel, TCP 2222, nonce,
HMAC et commandes BRDP reelles.

Il a aussi revele un defaut de vivacite independant de BRDP : la carte e1000
fonctionne en polling, mais le fil BRDP ne vidait que sa **file logique**.
Quand cette file etait vide, personne ne demandait a l'ingress unique de lire
l'anneau materiel. Un SYN et des requetes ARP pouvaient donc attendre dans la
carte jusqu'a ce qu'un autre chemin — observe : la reprise DHCP — appelle
`net::draine_anneau()`.

La capture observait la sequence suivante :

```text
SYN :2222 / ARP arrivent
        -> aucune reponse
        -> le guest continue d'emettre de la telemetrie
        -> un DHCP Request se declenche plus tard
        -> ARP Reply + SYN/ACK sortent d'un coup
```

Le correctif ne cree **pas** un second lecteur RX. Il applique au diagnostic la
meme regle que `retire_trame_smoltcp()` :

1. regarder sa file sous `VERROU_RECEPTION` ;
2. si elle est vide, rendre le verrou ;
3. appeler l'ingress unique `draine_anneau()` ;
4. reprendre le verrou et relire sa file.

C'est important : `draine_anneau()` reste l'unique proprietaire physique de
l'anneau RX et `route_trame()` continue de distribuer les trames aux piles.

## Contenu de l'overlay

```text
APPLY-BRDP-QEMU-E2E.ps1
src/net/diag_distant/mod.rs                 (modifie par le script)
.github/workflows/integration.yml            (modifie par le script)
tools/ci/run_brdp_qemu_e2e.sh               (nouveau)
tools/remote/brdp_qemu_live.py               (nouveau)
tools/remote/brdp_pcap_check.py              (nouveau)
docs/BOUCHAUD_LAB_QEMU_E2E.md                (nouveau)
```

Le ZIP est un **root overlay** : extraire directement a la racine du depot.

## 1. Mettre le depot local exactement a jour

PowerShell :

```powershell
cd C:\Users\Arthur\RustroverProjects\bouchaud-os-main

git status --short --branch
git fetch origin
git switch claude/ladybird-observability-performance
git pull --ff-only

git rev-parse HEAD
git log -1 --oneline
```

Le HEAD doit etre :

```text
6cea9568a3366cb2d87b1140cac48edd81851e87
```

Aucun `reset`, `clean` ou `restore` n'est necessaire.

## 2. Extraire puis appliquer

Extraire le contenu du ZIP directement dans :

```text
C:\Users\Arthur\RustroverProjects\bouchaud-os-main
```

Puis :

```powershell
.\APPLY-BRDP-QEMU-E2E.ps1
```

Le script refuse :

- une mauvaise branche ;
- un mauvais HEAD ;
- des modifications deja presentes sur des fichiers suivis ;
- une ancre source inattendue.

Il ne fait aucun reset et ne supprime rien.

## 3. Validation locale sans QEMU

Depuis WSL/Linux dans le depot, ou avec l'environnement habituel du projet :

```bash
cargo build
bash tools/ci/run_host_tests.sh
python3 tools/verifie-jeton-brdp.py
python3 tools/mesure-chantiers.py --verifie
```

Puis les garde-fous habituels du depot.

Le nombre de suites Rust/Python peut rester celui du HEAD de base : les deux
nouveaux scripts Python sont des sondes **live**, pas des `test_*.py` hote.

## 4. Preuve QEMU/TAP complete

Prerequis Linux/WSL :

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends qemu-system-x86 iproute2
```

`/dev/net/tun` doit exister.

Lancer :

```bash
bash tools/ci/run_brdp_qemu_e2e.sh
```

Le banc :

- construit une image avec le jeton de banc ephemere ;
- verifie que cette image contient bien le serveur BRDP ;
- cree un TAP temporaire `169.254.254.1/16` ;
- lance le listener telemetry avant QEMU ;
- decouvre l'IP `169.254.x.y` du guest depuis UDP 2223 ;
- prouve le handshake TCP 2222 + HMAC ;
- execute les commandes reelles du client ;
- teste coalescence, fragmentation, avant-auth, mauvais HMAC, reconnexion,
  deconnexion brutale et client lent ;
- produit un dump reel ;
- analyse le PCAP sans Scapy ;
- exige des SYN/SYNACK rapides et au moins une connexion sans DHCP entre les
  deux ;
- refuse tout RST emis depuis le port 2222 du guest ;
- verifie que le jeton n'apparait dans aucun artefact ;
- detruit toujours le TAP avec un `trap`.

Verdict attendu :

```text
BOUCHAUD_BRDP_QEMU_E2E_OK
```

Les preuves sont dans :

```text
target/brdp-qemu-e2e/
```

Notamment :

```text
serial.log
telemetry-boot.log
discover.json
live.json
live.log
brdp.pcap
pcap.json
commandes/
dumps/
resultat.txt
```

## 5. Ce que le banc ne pretend pas

Même vert, ce chantier ne prouve pas encore la panne physique RTL8168.

Le critere Trigkey reste :

```text
rx_packets > 64
ET
rx_rendus_tour2 > 0
```

Le banc QEMU prouve le **canal d'enquete** et la vivacite de l'ingress en
polling. Le chantier suivant reste la Blackbox longue duree (>45 s, >90 s),
puis seulement la nouvelle image physique.

## 6. Commit / push seulement apres tout vert

Quand le build, les tests hote et le E2E TAP sont verts :

```powershell
git status --short
git diff --check
git diff -- src/net/diag_distant/mod.rs .github/workflows/integration.yml

git add src/net/diag_distant/mod.rs `
        .github/workflows/integration.yml `
        tools/ci/run_brdp_qemu_e2e.sh `
        tools/remote/brdp_qemu_live.py `
        tools/remote/brdp_pcap_check.py `
        docs/BOUCHAUD_LAB_QEMU_E2E.md

git commit -m "test(diag): prouver BRDP et la telemetrie sous QEMU"
git push -u origin claude/ladybird-observability-performance
```

Ne pas inclure `APPLY-BRDP-QEMU-E2E.ps1` dans le commit : c'est seulement le
transport du patch vers le poste Windows.

Ensuite attendre :

```text
CI Fast
Integration
Reliability V3
```

Le job Integration `qemu / BRDP + telemetry e2e` devient lui aussi bloquant.
