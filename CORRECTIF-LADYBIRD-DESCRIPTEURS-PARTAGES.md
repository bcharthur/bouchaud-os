# Correctif Ladybird — descripteurs partages inscriptibles

Base attendue : commit `e8cea0f`.

## Panne corrigee

Bouchaud creait bien les nœuds RAMFS anonymes utilises par les tampons de
Ladybird, mais les descripteurs correspondants conservaient le mode par defaut
en lecture seule. Le controle de securite refusait donc, a juste titre, leur
projection partagee en ecriture et journalisait :

```text
[SECURITY-DENY] op=mmap-file-access detail=0x8
BouchaudBrowserHost: Permission denied (errno=13)
```

Le correctif ajoute un constructeur Bouchaud explicite pour les fichiers
partages inscriptibles et l'emploie sur les trois producteurs concernes :

- les tampons anonymes crees par `memfd_create` ;
- la surface graphique injectee au lancement d'un client ;
- les descripteurs de surface produits par `Surface::descripteur`.

Le controle `mmap` reste strict : aucune permission globale n'est ajoutee et
la sandbox n'est pas elargie. Le mode d'acces est conserve par `fork`, `dup` et
le passage de descripteurs, car `FileDesc::clone` le recopie deja.

Le correctif est entierement implemente dans Bouchaud. Il n'ajoute aucun noyau,
service, sous-systeme ou environnement hote externe. Le repertoire de
compatibilite deja present ne contient que l'implementation Bouchaud de l'ABI
attendue par les programmes portes.

## Installation

Extraire le ZIP directement a la racine de `bouchaud-os` et accepter le
remplacement des fichiers.

Puis, dans PowerShell :

```powershell
.\VALIDER-CORRECTIF-LADYBIRD-DESCRIPTEURS.ps1 -Bootimage
.\run.ps1
```

Ouvrir Ladybird et verifier que `BROWSER_HOST_INITIALIZED` n'est plus suivi de
`mmap-file-access detail=0x8`, de `BROWSER_HOST_EXIT erreur` ou de l'erreur 13
fatale.

Le refus de lecture de `/proc/sys/vm/overcommit_memory` est une sonde facultative
du programme et n'empeche pas son execution. L'avertissement de cache sous
`/persist` est un sujet distinct et non fatal : ce lot ne lui accorde pas un
passe-droit dans la sandbox.
