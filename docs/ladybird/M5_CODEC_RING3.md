# M5b en ring 3 : le reveil perdu de `poll`

**M5 est termine.** Les six verifications du temoin passent en ring 3, sous
QEMU, dans le travail `product-qemu-boot` du workflow `userland`.

Ce document garde la trace du defaut, parce que la facon dont il a ete trouve
vaut plus que le correctif lui-meme — cinq lignes.

## La preuve

Sortie serie de Bouchaud OS, le runner Linux n'etant que le compilateur et
l'hote de QEMU :

    == temoin LibIPC : endpoints generes ==
      ok     u32 : ping 42 -> pong 43
      ok     String traverse intact
      ok     Vector<u32> traverse intact
      ok     Optional<String> renseigne traverse intact
      ok     URL::URL traverse intacte
      ok     les quatre types dans un seul message
    RESULTAT : 0 verification(s) en echec (6 passees)

Le verdict de CI ne se contente pas du bandeau : il exige chacune des six
lignes **et** le `RESULTAT` (`.github/workflows/userland.yml`). Un temoin qui
s'arreterait au milieu ne pourrait pas passer pour vert.

## Ce que l'instrumentation avait etabli

Le temoin envoie **un type a la fois**, en chaine, chaque reponse declenchant
l'envoi suivant, et chaque etape trace son passage des deux cotes. C'est ce
decoupage qui a rendu le defaut lisible. En ring 3, avant correctif :

    == temoin LibIPC : endpoints generes ==
             [client] envoi ping
             [serveur] ping recu
      ok     u32 : ping 42 -> pong 43
             [client] envoi echo_string
      ECHEC  aucune reponse en 10 s
             [serveur] echo_string recu        <-- APRES le delai

La derniere ligne portait tout : le serveur recevait bien `echo_string`, mais
seulement une fois le delai ecoule — c'est-a-dire au moment ou le client
appelait `shutdown()`, qui vide la file d'envoi.

Cela ecartait trois pistes d'un coup :

- **pas le codec** : le message etait decode correctement quand il arrivait ;
- **pas la fragmentation** : il arrivait entier, plus tard ;
- **pas `URL::URL`** : l'echec survenait des le premier message apres le
  `ping`, qui ne porte qu'un `String`.

La matrice des types n'etait donc pas « `u32` OK, le reste en echec » mais
« `u32` OK, le reste **jamais atteint** ». Une nuance qui change la recherche
du tout au tout : il ne fallait pas chercher dans la serialisation.

## La cause

`TransportSocket` n'ecrit pas directement : il empile dans `m_send_queue`, puis
appelle `wake_io_thread()`, qui ecrit un octet dans un tube. Le fil
d'entree/sortie attend sur `poll`, sur deux descripteurs — la socket et le bout
lecteur de ce tube.

Le defaut etait dans `sys_poll` (`src/kernel/abi/file.rs`) :

```rust
task::yield_now();
crate::arch::x86_64::cpu::wait_for_interrupt();
```

`yield_now()` pouvait basculer vers un autre fil, celui-la meme qui ecrivait
l'octet de reveil. Mais au retour, la boucle tombait **directement** dans
`wait_for_interrupt()` — un `hlt` — sans jamais reevaluer les descripteurs. Le
reveil logiciel etait donc perdu jusqu'a la prochaine interruption materielle.

C'est bien pour cela que le message finissait par passer : il attendait le
minuteur, pas le tube.

## Le correctif

```rust
if task::schedule() {
    continue;
}
crate::arch::x86_64::cpu::wait_for_interrupt();
```

`task::schedule()` rend `true` quand une commutation a **reellement** eu lieu.
Dans ce cas un autre fil s'est execute et a pu rendre un descripteur pret : on
reprend la boucle et on relit les descripteurs avant de dormir. Sinon,
personne d'autre n'etait pret et le `hlt` est legitime.

`sys_select` avait exactement le meme defaut et a recu le meme correctif.

## Ce qu'il faut en retenir

Le correctif est **du cote de Bouchaud**, generique, et ne touche pas une ligne
de Ladybird. C'etait la bonne lecture des le depart : ce qui manquait n'etait
pas une brique de Ladybird mais une primitive de Bouchaud — le reveil d'un fil
par un tube. Toute application POSIX qui reveille un `poll` depuis un autre fil
en beneficie, pas seulement LibIPC.

Le sous-jacent n'avait d'ailleurs jamais ete mis en doute : `TestTransportSocket`
d'upstream — cinq cas, dont le passage de descripteurs et le raccrochage du
pair — passait deja en ring 3. Le transport tenait ; c'est l'ordonnanceur qui
dormait trop tot.
