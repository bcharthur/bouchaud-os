# Bouchaud OS V0.13.x — Bureau + terminal graphique

Ce patch stabilise le bureau VGA 13h et ajoute un terminal graphique interactif.

## Fonctionnalités

- Retour propre au mode texte après `desktop`.
- Recharge de la police texte VGA à la sortie du mode 13h.
- Échap réactif : la file clavier est drainée à chaque frame.
- Barre des tâches : `Terminal` et `Quitter`.
- Fenêtre `Système` déplaçable à la souris.
- Terminal graphique plein écran :
  - prompt utilisateur + chemin courant ;
  - curseur `_` ;
  - scrollback automatique ;
  - réutilisation du shell complet via `shell::run_capture` ;
  - support des commandes, pipes, redirections et variables existantes ;
  - `exit` ou Échap pour revenir au bureau.

## Limitations

Les commandes plein écran texte sont bloquées dans le terminal graphique :

- `desktop`
- `gui`
- `edit`
- `nano`
- `panic-test`
- `breakpoint`

Utilise-les depuis le shell texte.

## Tests

Depuis le shell texte :

```text
desktop
```

Dans le bureau :

1. Cliquer `Terminal`.
2. Tester :

```text
whoami
ls
cat /etc/passwd | grep root
sysinfo
date
exit
```

3. Cliquer `Quitter` ou appuyer sur Échap.

La sortie série doit montrer :

```text
[gui] bureau demarre
[gui] terminal demarre
[gui] terminal ferme
[gui] bureau ferme
```
