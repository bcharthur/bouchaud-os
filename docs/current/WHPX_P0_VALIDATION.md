# Validation WHPX de #366

Le banc compare main `c856ece9aaced8167aa8c783276ca3a5da7222a4` et #366
`218a556c733d412b30fc11c9ad0384e824edecf3` sur Windows PowerShell 5.1.
Il exige Python 3.9+, Git, GitHub CLI authentifiée, Cargo avec bootimage
et QEMU WHPX à son emplacement habituel sous Program Files. Fermer les
VM QEMU avant la campagne et garder les mêmes réglages d'affichage hôte.

Depuis le checkout du banc, indiquer les deux checkouts sources existants.
Le banc lit leurs SHA et crée des worktrees temporaires à part. Il construit
les bootimages, démarre son propre serveur HTTP local sur un port libre,
puis enchaîne A/B/B/A/A/B. Chaque campagne exige un dossier de sortie neuf.

```powershell
$projects = Join-Path $env:USERPROFILE 'RustroverProjects'
$options = @{
    ReferenceCheckout = Join-Path $projects 'bouchaud-os'
    CandidateCheckout = Join-Path $projects 'bouchaud-os-p366'
}
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
.\tools\windows\Banc-BrowserHost-WHPX.ps1 @options -Mode CommonKernel `
    -OutputDirectory (Join-Path $projects "whpx-366-commun-$stamp")
.\tools\windows\Banc-BrowserHost-WHPX.ps1 @options -Mode FullStack `
    -OutputDirectory (Join-Path $projects "whpx-366-complet-$stamp")
```

- **CommonKernel** utilise le bootimage main dans les deux bras. Il compare
  les artefacts Ladybird à noyau identique.
- **FullStack** utilise le bootimage propre à chaque SHA. Il qualifie le
  candidat complet, y compris le correctif noyau, avec le même scénario.

Le lanceur vérifie la recette et les SHA du manifeste Ladybird. Les runs
par défaut sont `45009297744` pour A et `45044774669` pour B. Si un
artefact expire, fournir un run compatible avec `-ReferenceRunId` ou
`-CandidateRunId`. Une fois le premier artefact vérifié pour chaque bras,
le banc réutilise le cache et rejette toute différence de manifeste ou
de SHA BrowserHost/WebContent à l'intérieur du bras.

QEMU attend la validation de son disque en pré-démarrage. Chaque passage
repart d'une image neuve avec zone de persistance nulle. Le contenu logique
du disque hors charge Ladybird doit être identique. Le disque complet est
haché par passage, et son image de base reste inchangée grâce au snapshot.
Ne pas interagir avec la fenêtre pendant ces mesures. Le moniteur arrête la
VM après réception des marqueurs. Le banc contrôle le code de sortie, le
SHA du noyau, la signature WHPX, le viewport, les 240 callbacks et la suite
continue des publications. Une session incomplète interrompt la campagne.

Les répertoires de passage contiennent les journaux du lanceur, de QEMU et
du port série, ainsi que le manifeste d'artefact et les empreintes des
entrées. `rapport.json` donne P50/P95/P99 et les écarts des trois paires.
Les intervalles rAF sont des mesures côté page. Les publications et leur
temps de présentation sont des mesures BrowserHost. La cadence affichée
demande une capture hôte.

## Vérification visuelle

Faire un passage interactif dans chaque checkout sur la fixture servie
depuis #366. Utiliser les mêmes réglages et capturer la fenêtre QEMU à
fréquence fixe. Garder ce passage distinct de la campagne automatique.

1. Attendre « Page prête », puis « Terminé ». Vérifier que les lignes
   défilent dans l'ordre et que la barre d'outils reste stable.
2. Ouvrir un deuxième onglet, charger la fixture sans démarrage automatique,
   revenir au premier onglet, puis fermer le deuxième.
3. Naviguer vers la page d'accueil locale, revenir à la fixture, puis
   redimensionner la fenêtre Ladybird en largeur et en hauteur dans les deux
   sens. Vérifier les zones nouvellement exposées et la réponse aux clics.
4. Sur la capture, relever toute zone blanche inattendue, image périmée,
   corruption ou pause visible avec l'heure et l'action. Conserver les
   journaux série et QEMU correspondant aux deux captures.
