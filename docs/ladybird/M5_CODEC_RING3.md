# M5b en ring 3 : un message court passe, un message long non

## L'observation

Le temoin `tools/ladybird/libipc-codec-probe.cpp` echange deux messages a
travers des endpoints **generes** par `Meta/Generators/generate_ipc_definitions.py`,
entre deux vrais processus relies par un `socketpair`.

Sur l'hote, les cinq verifications passent.

En ring 3 sous QEMU :

    == temoin LibIPC : endpoints generes ==
      ok     pong : la valeur a fait l'aller-retour
             ping 42 -> pong 43
      ECHEC  aucune reponse en 10 s

    RESULTAT : 1 verification(s) en echec (1 passees)

Le premier message aboutit. Le second reste sans reponse.

## Ce que cela elimine deja

Le `ping`/`pong` qui passe **prouve** que la chaine complete fonctionne :

- le fichier `.ipc` est lu par le generateur d'upstream ;
- le `Proxy` genere encode et ecrit ;
- `IPC::Transport` traverse le `socketpair` de Bouchaud ;
- le fil d'entree/sortie de `TransportSocket` lit et reveille la boucle ;
- le `Stub` genere decode et appelle la methode virtuelle ;
- la reponse fait le chemin inverse.

Ce n'est donc ni le generateur, ni les endpoints, ni le transport, ni `fork`,
ni le `socketpair`, ni le fil d'entree/sortie qui manquent. **M5a est atteint en
ring 3.**

## La difference entre les deux messages

| | `ping` | `echo` |
|---|---|---|
| champs | `u32` | `String`, `Vector<u32>`, `Optional<String>`, `URL::URL` |
| taille | quelques octets | quelques centaines |
| codecs | arithmetique | chaines, conteneurs, URL |

Deux hypotheses, dans cet ordre de vraisemblance :

1. **Un codec ou un chemin de decodage qui s'arrete.** `URL::URL` passe par la
   caisse Rust `liburl_rust` ; `Optional<String>` et `Vector<u32>` par les
   gabarits de `Decoder.h`. Un echec cote serveur ferait taire la reponse sans
   rien imprimer, et c'est bien ce qu'on observe : pas de fin de fichier, pas de
   `die()`, juste un silence — donc un processus **vivant mais qui ne repond
   pas**, non un processus mort.

2. **La lecture fragmentee.** `TransportSocket` lit par
   `receive_message(..., MSG_DONTWAIT, ...)` dans un fil dedie et attend sur
   `Core::System::poll`. Un message livre en plusieurs morceaux exige que la
   lecture partielle soit conservee et que `poll` se rearme.

   Cette piste, que j'avais d'abord placee en tete, est **affaiblie par la
   mesure** : le tampon de lecture fait 4096 octets (`TransportSocket.cpp:423`)
   et le message `echo` en fait quelques centaines. Il n'a aucune raison d'etre
   fragmente. Elle ne redevient plausible que si le `socketpair` de Bouchaud
   livre par morceaux plus petits que ce que la taille du message justifie —
   ce qui serait en soi le defaut a corriger.

## Comment trancher

Par ordre de cout croissant :

1. **Instrumenter le serveur** : imprimer a l'entree de `echo()`. Si la ligne
   sort, le probleme est au retour ; si elle ne sort pas, il est a l'aller.
   C'est la mesure la plus discriminante, et la moins chere.
2. Retirer les champs un a un — d'abord `URL::URL`, puis `Optional`, puis
   `Vector` — jusqu'a ce que l'aller-retour revienne. Le champ qui debloque
   nomme le codec en cause (hypothese 1).
3. Rallonger un simple `String` par paliers. Si la rupture suit la taille et
   non le type, c'est l'hypothese 2.
4. `strace` autour du temoin (le harnais sait le faire) pour lire les
   `recvmsg`/`poll` reels et leurs tailles de retour.

## Pourquoi le temoin n'est pas dans le scenario QEMU

Parce qu'une verification rouge en permanence n'apprend rien de plus que ce
document, et masque les regressions reelles. Le temoin reste construit et joue
sur l'hote, ou il est vert ; `tools/test.sh` le reprendra des que le defaut sera
compris.

## Ce que cela ne remet pas en cause

`TestTransportSocket` d'upstream — cinq cas, dont le passage de descripteurs,
le raccrochage du pair et le message arrive juste avant la fin de fichier —
passe en ring 3. Le transport tient. Ce qui manque est un cas particulier de
lecture, pas la couche.
