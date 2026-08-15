# M5b en ring 3 : le message reste dans la file d'envoi

**M5 n'est pas termine.** Ce document dit ou en est le diagnostic.

## L'observation, apres instrumentation

Le temoin `tools/ladybird/libipc-codec-probe.cpp` envoie desormais **un type a
la fois**, en chaine, chaque reponse declenchant l'envoi suivant. Chaque etape
trace son passage, des deux cotes.

Sur l'hote, les six verifications passent — `u32`, `String`, `Vector<u32>`,
`Optional<String>`, `URL::URL`, puis les quatre reunis.

En ring 3 :

    == temoin LibIPC : endpoints generes ==
             [client] envoi ping
             [serveur] ping recu
      ok     u32 : ping 42 -> pong 43
             [client] envoi echo_string
      ECHEC  aucune reponse en 10 s
             [serveur] echo_string recu        <-- APRES le delai

## Ce que la trace etablit

La derniere ligne est la plus importante : **le serveur recoit bien
`echo_string`, mais seulement une fois le delai ecoule** — c'est-a-dire au
moment ou le client appelle `shutdown()`.

Cela elimine plusieurs pistes d'un coup :

- **ce n'est pas le codec.** Le message est decode correctement quand il
  arrive ; `String` n'est pas en cause, et les types suivants n'ont meme pas ete
  atteints ;
- **ce n'est pas la fragmentation.** Le message arrive entier, plus tard ;
- **ce n'est pas `URL::URL`.** L'echec survient des le premier message apres le
  `ping`, qui ne porte qu'un `String`.

La matrice des types est donc, en ring 3 : `u32` OK, le reste **non atteint** —
et non « en echec ».

## La cause probable, et pourquoi

Le comportement observe est exactement celui que decrit le test d'upstream
`buffered_message_is_drained_when_io_thread_stops_without_reading_it` : des
octets restes en attente sont drainés a l'arret.

`TransportSocket` n'ecrit pas directement. Il empile dans `m_send_queue`, puis
appelle `wake_io_thread()`, qui **ecrit un octet dans un tube** cree par
`pipe2(O_CLOEXEC | O_NONBLOCK)` (`TransportSocket.cpp:165`). Le fil
d'entree/sortie attend sur `poll` **deux** descripteurs : la socket et le bout
lecteur de ce tube (`TransportSocket.cpp:200`).

Le premier message du temoin (`ping`) part de `main()`, avant
`Core::EventLoop::exec()`. Les suivants partent depuis un gestionnaire, donc par
la file et son reveil.

L'hypothese est donc : **le reveil par le tube ne parvient pas au fil
d'entree/sortie sous Bouchaud.** Le message reste dans la file jusqu'a ce que
`shutdown()` la vide.

## Ce qu'il faut mesurer ensuite

1. Un test **independant de Ladybird** : deux fils d'un meme processus, un
   `pipe2(O_NONBLOCK)`, l'un bloque dans `poll(-1)`, l'autre ecrit un octet.
   Le premier doit se reveiller. C'est trois dizaines de lignes, et cela
   tranche entre « defaut Bouchaud » et « defaut d'integration ».
2. Si le reveil manque : regarder `sys_poll` (`src/kernel/abi/file.rs:1800`).
   Sa boucle fait `task::yield_now()` puis
   `crate::arch::x86_64::cpu::wait_for_interrupt()` — un `hlt`. Verifier qu'un
   `write` fait par **un autre fil du meme processus** rend bien la main a ce
   `poll`, et que `hlt` ne suspend pas le processeur alors qu'une autre tache
   est prete.
3. Verifier que `pipe2` honore `O_NONBLOCK` et `O_CLOEXEC`.

## Pourquoi le temoin n'est pas dans le scenario QEMU

Parce que le laisser rouge en permanence masquerait les regressions reelles,
et n'apprendrait rien de plus que ce document.

Ce n'est **pas** une facon de declarer M5 termine : il ne l'est pas. Le temoin
est construit pour la cible, joue sur l'hote ou il est vert, et `tools/test.sh`
le reprendra des que le reveil sera repare.

## Ce que cela ne remet pas en cause

`TestTransportSocket` d'upstream — cinq cas, dont le passage de descripteurs et
le raccrochage du pair — passe en ring 3. Le transport tient. Ce qui manque est
le reveil d'un fil par un tube, une primitive de Bouchaud, pas une brique de
Ladybird.
