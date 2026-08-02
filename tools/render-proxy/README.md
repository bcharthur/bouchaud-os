# Nautile — service de rendu déporté

Nautile (le navigateur de Bouchaud OS) ne peut pas exécuter les sites modernes
(applications JavaScript type Google/YouTube, CSS complet, SVG, polices web…).
Ce petit service les rend avec un **vrai Chromium headless** et renvoie une
**image PNG** de la page. Nautile affiche cette image telle quelle — donc les
pages s'affichent **exactement comme dans Chrome**.

C'est l'architecture des navigateurs « cloud » (Opera Mini, Puffin, Amazon Silk) :
le rendu lourd se fait côté serveur, le client léger affiche le résultat.

## Installation (une fois)

Prérequis : Node.js 18+.

```bash
cd tools/render-proxy
npm run setup     # installe playwright + sharp, puis télécharge Chromium
```

## Lancement

```bash
npm start         # écoute sur http://0.0.0.0:8080
```

Laisse ce terminal ouvert pendant que tu utilises Nautile.

## Test rapide (depuis l'hôte)

```
http://localhost:8080/healthz                     -> ok
http://localhost:8080/render?url=https://example.com   -> PNG
```

## Comment Nautile l'utilise

Sous QEMU (réseau utilisateur SLIRP), l'hôte est joignable depuis l'OS à
l'adresse **`10.0.2.2`**. Deux usages :

- **Nautile en mode compat** (préfixe `compat:` ou icône Nautile) — rendu
  statique d'une page en une image :
  ```
  http://10.0.2.2:8080/render?url=<url-encodée>
  ```
- **App WebView** (bureau) — navigation interactive, une page Chromium
  persistante pilotée à distance :
  ```
  /wv/open?url=&w=&h=     ouvre une URL (règle aussi le viewport)
  /wv/click?x=&y=&w=&h=   clic aux coordonnées viewport (1:1 avec la fenêtre)
  /wv/scroll?dy=&w=&h=    molette verticale
  /wv/type?text=&w=&h=    saisie clavier
  /wv/key?k=&w=&h=        touche spéciale (Enter, Backspace, Arrow*, …)
  /wv/back /wv/forward /wv/reload   navigation
  /wv/url                 URL courante (text/plain, pour la barre d'adresse)
  ```
  Chaque action renvoie une capture PNG du viewport (RGB non entrelacé).

Les adresses candidates du proxy sont dans `src/browser/loader.rs`
(`PROXY_HOSTS` : `10.0.2.2:8080` puis l'IP LAN de l'hôte). L'OS sonde
`/healthz` et retient le premier qui répond.

### Chromium fourni par l'environnement

Si un Chromium/Playwright est déjà installé, exportez `CHROMIUM_PATH` (chemin du
binaire `chrome`) avant `npm start` pour éviter un téléchargement. Le serveur
lit aussi `HTTPS_PROXY`/`NO_PROXY` pour fonctionner derrière un proxy sortant.

### Pare-feu Windows (important)

La connexion invité→hôte (SLIRP) arrive sur l'interface de l'hôte et peut être
**bloquée par le pare-feu Windows** sur `node.exe` (alors que `localhost:8080`
depuis Chrome passe par le loopback, non filtré). Si Nautile affiche
« connexion TCP echouee », autorise le port 8080 en entrée — PowerShell
**administrateur** :

```powershell
New-NetFirewallRule -DisplayName "Nautile render proxy" -Direction Inbound -Action Allow -Protocol TCP -LocalPort 8080
```

(ou autorise `node.exe` quand Windows le propose au premier `npm start`, et mets
le profil réseau en « Privé »).

## Notes / limites (v1)

- L'image est redimensionnée pour rester sous le plafond du décodeur PNG de
  l'OS (~1,2 Mpx) et produite en **RGB 8 bits non entrelacé**.
- Pages longues : la page entière est capturée puis réduite pour tenir dans le
  budget — le texte des très longues pages devient petit. (Réglable via `BUDGET`
  dans `server.js`.)
- Les liens internes ne sont pas encore cliquables sur une page rendue à
  distance : on navigue via la barre d'adresse. (Évolution prévue : renvoyer la
  carte des liens.)
- Les pages internes de l'OS (`about:bouchaud`, `about:wasm`, `file:/…`) restent
  rendues nativement par le moteur de l'OS, sans passer par ce service.
