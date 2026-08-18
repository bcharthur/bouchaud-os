# Autorites racine pour le navigateur natif

Ce dossier ne contient **aucun certificat versionne**. Il contient le script
qui en fabrique un, et le resultat (`cacert.pem`) est ignore par git.

## Pourquoi

`RequestServer` pose le chemin recu en `--certificate` dans `CURLOPT_CAINFO`.
Sans ce fichier, curl retombe sur le chemin decide a la compilation, qui
n'existe pas dans Bouchaud OS : **toute** connexion HTTPS echoue a la
verification, y compris contre un serveur parfaitement valide. Le symptome
ressemble a une panne reseau et n'en est pas une.

## Pourquoi il n'est pas versionne

Un magasin d'autorites decrit a qui **une machine donnee** fait confiance, a une
**date donnee**. Le figer dans le depot, c'est :

- diffuser une decision de confiance a la place de l'utilisateur ;
- garantir qu'il sera perime — une revocation ou un retrait de racine ne se
  propagerait plus ;
- et donner l'illusion du controle plutot que le controle.

Le script prend donc les autorites que la machine hote a **deja** acceptees.

## Utilisation

`run.ps1` l'appelle tout seul quand l'URL demandee est en `https://` et que le
bundle manque. A la main :

```powershell
.\tools\ladybird\certs\fabrique-bundle.ps1          # fabrique s'il manque
.\tools\ladybird\certs\fabrique-bundle.ps1 -Force   # reconstruit
```

Sources, dans l'ordre :

1. le magasin racine de Windows (`LocalMachine` puis `CurrentUser`), certificats
   expires exclus ;
2. a defaut, les racines DER de `src/net/security/tls/ca/`, celles de la pile
   TLS du noyau — huit autorites, suffisantes pour un essai, pas pour le Web.

## Ce que cela ne relache pas

Rien. Pas de `-k`, pas de `CURLOPT_SSL_VERIFYPEER=0`, pas d'exception de nom.
Un certificat invalide fait echouer la requete, et `docs/ladybird/M12_HTTPS.md`
dit pourquoi il n'y a pas encore d'ecran d'avertissement pour le porter.
