# Etat d'integration de Ladybird

<!--
  CE FICHIER EST GENERE. Ne pas le modifier a la main entre les balises
  MESURE:DEBUT et MESURE:FIN : `tools/verifie-integration-ladybird.py` les
  compare a une mesure fraiche et echoue si elles different.

      python3 tools/ladybird/mesure-integration.py --ecris

  Le tableau et les items vivent dans `tools/ladybird/mesure-integration.py`.
-->

## Comment lire ce tableau

Chaque ligne porte une preuve, et la preuve est rejouee a chaque mesure.

| Etat | Ce qu'il veut dire |
|---|---|
| `OK` | un banc ou un garde-fou vert, execute a l'instant |
| `CABLE` | le chemin existe dans la chaine de portage -- **pas** qu'il marche |
| `PHYSIQUE` | vu une fois sur la TRIGKEY, a la date indiquee, rejoue par personne |
| `NON MESURE` | aucune preuve. Pas « ca ne marche pas » : « on ne sait pas » |
| `ECHEC` | la preuve a ete rejouee et elle est rouge |

Les deux premiers chiffres ne se melangent pas, et c'est deliberé. Ce qui est
verifie en continu ne peut pas regresser sans qu'on le sache ; ce qui a ete vu
une fois sur la machine peut avoir cesse d'etre vrai depuis, et aucune CI ne le
dira. Les confondre donnerait le « 95 % » qui ne veut rien dire.

`NON MESURE` est la colonne la plus utile du tableau : c'est la liste de ce
qu'il reste a instrumenter.

## Mesure

<!-- MESURE:DEBUT -->

    prouve par execution  13/32 elements  (40 %)
    cable, non prouve     4 ligne(s)
    vu sur la machine     4 ligne(s), non rejouees
    non mesure            11 ligne(s)
    en echec              0 ligne(s)

| Element | Etat | Preuve | Ce qu'elle dit |
|---|---|---|---|
| Reseau physique (RTL8168) | **OK** | `garde:verifie-pilote-rtl8168` | garde verte |
| DNS | **OK** | `garde:verifie-lien-reseau` | garde verte |
| Verdict reseau | **OK** | `garde:verifie-verdict-reseau` | garde verte |
| Supervision des processus | **OK** | `hote:test_supervision` | test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Roles des binaires livres | **OK** | `hote:test_roles_livres` | test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Fautes de page par processus | **OK** | `hote:test_fautes` | test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Cycle de vie des onglets | **OK** | `garde:verifie-lifecycle-pages` | garde verte |
| Enregistrement des pages chez l'hote | **CABLE** | `marqueur:tools/ladybird/prepare-m11-page-registry.py:M11_TAB_STAGE 80 READY` | M11_TAB_STAGE 80 READY present dans tools/ladybird/prepare-m11-page-registry.py |
| Vue Services | **OK** | `garde:verifie-fenetre-services` | garde verte |
| Onglets du chrome | **OK** | `garde:verifie-onglets` | garde verte |
| Clavier du navigateur | **OK** | `garde:verifie-clavier-navigateur` | garde verte |
| Repeinture partielle | **OK** | `garde:verifie-repeinture-partielle` | garde verte |
| Polices | **OK** | `garde:verifie-polices-navigateur` | garde verte |
| Telechargements | **OK** | `garde:verifie-telechargements` | garde verte |
| Compositor supervise | **CABLE** | `marqueur:src/kernel/navigateur/supervision_corps.rs:Role::Composition` | Role::Composition present dans src/kernel/navigateur/supervision_corps.rs |
| ImageDecoder cable | **CABLE** | `marqueur:tools/ladybird/prepare-image-decoder.py:ImageDecoder` | ImageDecoder present dans tools/ladybird/prepare-image-decoder.py |
| WebWorker cable | **CABLE** | `marqueur:tools/ladybird/prepare-full-browser-host.py:WebWorker` | WebWorker present dans tools/ladybird/prepare-full-browser-host.py |
| HTTPS / TLS | **PHYSIQUE** | `physique:bb(8) 2026-09-18` | bb(8) 2026-09-18 |
| HTTP / RequestServer | **PHYSIQUE** | `physique:bb(8) 2026-09-18` | bb(8) 2026-09-18 |
| Document charge | **PHYSIQUE** | `physique:bb(8) 2026-09-18` | bb(8) 2026-09-18 |
| Plusieurs onglets | **PHYSIQUE** | `physique:2026-09-19` | 2026-09-19 |
| Images affichees | **NON MESURE** | `—` | telechargees mais non affichees ; chaine de decodage non instrumentee |
| JavaScript | **NON MESURE** | `—` | aucun banc ne l'exerce |
| WebWorker a l'oeuvre | **NON MESURE** | `—` | packagé, jamais observe en service |
| Cookies | **NON MESURE** | `—` | --disable-sql-database |
| Cache disque | **NON MESURE** | `—` | --disable-http-disk-cache |
| Stockage / profil | **NON MESURE** | `—` | pots upstream en memoire seulement |
| Isolation de site | **NON MESURE** | `—` | --site-isolation=disable |
| Audio | **NON MESURE** | `—` | aucun backend |
| GPU | **NON MESURE** | `—` | --force-cpu-painting |
| Temps de demarrage | **NON MESURE** | `—` | non profile |
| Latence interactive sous charge | **NON MESURE** | `—` | non mesuree |

<!-- MESURE:FIN -->
