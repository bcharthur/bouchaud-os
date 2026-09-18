BOUCHAUD OS — PATCH M11 PAGE LIFECYCLE / PAGE REGISTRY
======================================================

Base cible
----------
Branche : claude/rtl8168-ladybird-internet
Commit  : a36b3e477adedeaf807992b6a75ea186a5c64271
Ladybird: cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6

But
---
Corriger le cas physique bb(8) : page 2/3 existe dans WebContent mais n'est
pas enregistree dans WebContentClient::m_views. Resultat avant patch :
"Did not find a page with ID 2/3", backing stores rejetes et Wikipedia blanc.

Principe
--------
Avec BouchaudBrowserHost, la creation d'onglet repasse par l'IPC upstream
DidRequestNewWebView. Le HeadlessWebView enfant est donc cree cote host et
WebContentClient::register_view(page_id, view) est execute AVANT que WebContent
cree son PageClient. Le chemin legacy sans BrowserHost reste disponible.

Le patch traite :
- onglet utilisateur (+ / Ctrl+T) ;
- popup / target=_blank avec BrowserHost ;
- garde statique du pipeline de preparation.

Installation
------------
1. Extraire CE ZIP directement a la racine de :
   C:\Users\Arthur\RustroverProjects\bouchaud-os-claude
   Il n'y a aucun dossier englobant dans le ZIP.

2. Verifier d'abord :
   git status --short
   git branch --show-current
   git rev-parse HEAD

   HEAD attendu : a36b3e477adedeaf807992b6a75ea186a5c64271

3. Creer de preference une branche locale de test :
   git switch -c test/m11-page-lifecycle-a36b3e4

4. Appliquer le petit hook dans browser-upstream.sh :
   powershell -ExecutionPolicy Bypass -File .\APPLY-PAGE-LIFECYCLE-A36B3E4.ps1

5. Inspecter le diff dans RustRover / IDE :
   git status --short
   git diff -- tools/ladybird/browser-upstream.sh tools/ladybird/prepare-m11-page-registry.py tools/verifie-lifecycle-pages.py

6. Gardes :
   python tools/verifie-lifecycle-pages.py
   git diff --check

7. Construire l'image habituelle :
   powershell -ExecutionPolicy Bypass -File .\tools\reference\IMAGE-TRIGKEY.ps1

Criteres physiques
------------------
Scenario : onglet 1 example.com, onglet 2 wikipedia.org, puis un 3e onglet.

Attendu :
- M11_TAB_HOST_REGISTERED page=2 puis page=3 ;
- aucune serie "Did not find a page with ID 2/3" ;
- Wikipedia visible, pas seulement hit-testable ;
- frames/backing stores des pages 2/3 acceptes ;
- changement d'onglet et fermeture fonctionnels.

Important
---------
Ce patch NE TOUCHE PAS au RTL8168, DHCP, DNS, TCP ou TLS.
Il ne fait aucun reset/clean/restore Git.
Il ne pousse rien sur GitHub.
