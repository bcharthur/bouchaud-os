# Bouchaud OS V0.9 — Comptes utilisateurs dynamiques + login

Refonte de la gestion des utilisateurs pour se rapprocher de Linux.

## Nouveautes

- **Base d'utilisateurs dynamique** (`users/mod.rs`) : table a taille fixe
  (sans `alloc`). Comptes par defaut : **`root`** (uid 0, mdp `root`) et
  **`guest`** (uid 1000, mdp `guest`). Plus d'utilisateur code en dur ailleurs.
- **Ecran de connexion au boot** : `login:` + `Mot de passe:` (saisie masquee).
  Tant que les identifiants sont faux, on reste sur l'ecran. `logout`/`exit`
  ferment la session et **reviennent a l'ecran de connexion**.
- **Gestion des comptes** :
  - `useradd <nom>` (root) : cree un compte, demande + confirme le mot de passe,
    cree son home `/home/<nom>` (mode 700).
  - `userdel <nom>` (root) : supprime un compte (root protege).
  - `passwd [user]` : change son propre mot de passe, ou celui d'un autre si root.
  - `users` : liste les comptes (format `nom:x:uid:gid:home`).
  - `su [user]` : change d'utilisateur dans la session (avec mot de passe).
- **`chmod` symbolique facon Linux** : en plus de l'octal (`chmod 755 f`), on
  accepte `chmod +x f`, `chmod u+w f`, `chmod go-r f`, `chmod a=rx f`, etc.
- **`chown <user|uid> <path>`** (root) : utilise la base d'utilisateurs.

## Demo a tester

```text
# Au boot : ecran de connexion
login: guest
Mot de passe: guest

guest@bouchaud-os:/home/guest$ whoami
guest
guest@bouchaud-os:/home/guest$ useradd arthur      # refuse : reserve a root
useradd: reserve a root
guest@bouchaud-os:/home/guest$ su root             # mdp: root
root@bouchaud-os:/$ useradd arthur
Nouveau mot de passe: ****
Confirmer: ****
useradd: arthur cree (uid=1001)
root@bouchaud-os:/$ users
root:x:0:0:/
guest:x:1000:1000:/home/guest
arthur:x:1001:1001:/home/arthur
root@bouchaud-os:/$ logout

# retour a l'ecran de connexion
login: arthur
Mot de passe: ****
arthur@bouchaud-os:/home/arthur$ touch script.sh
arthur@bouchaud-os:/home/arthur$ chmod +x script.sh
arthur@bouchaud-os:/home/arthur$ ls -l
-rwxr-xr-x 1001:1001    0 script.sh

# isolation : guest ne voit pas le home d'arthur
arthur@...$ logout
login: guest / guest
guest@...$ cd /home/arthur     -> cd: permission denied
guest@...$ cat /home/arthur/script.sh -> cat: permission denied
```

## Build & lancement

```powershell
git pull origin claude/great-wright-jvdnh5
cargo +nightly clean
.\run.ps1
```
