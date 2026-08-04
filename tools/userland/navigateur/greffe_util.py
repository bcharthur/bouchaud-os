"""Apprend a pywebview le schema d'URL des pages internes du moteur.

`webview/util.py` classe comme « ressource locale » toute URL qui n'est ni
`http://`, ni `https://`, ni `file://`. Pour une telle URL, `Window.load_url`
demarre le serveur HTTP interne de pywebview afin de la servir — lequel a besoin
de `listen`/`accept`, que le noyau n'implemente pas encore.

Or `bo:` designe des pages que le moteur rend lui-meme, sans reseau : il n'y a
rien a servir. On le declare donc distant, ce qui court-circuite le serveur.

Applique par `greffe-pywebview.sh`.
"""

import sys

AVANT = "            or (url.startswith('http://'))"
APRES = ("            or (url.startswith('bo:'))  "
         "# pages internes du moteur Bouchaud OS\n"
         "            or (url.startswith('http://'))")


def main():
    chemin = sys.argv[1]
    source = open(chemin).read()
    if "url.startswith('bo:')" in source:
        print('  util.py   : deja greffe')
        return 0
    if AVANT not in source:
        print('  util.py   : is_local_url inattendu — greffe abandonnee',
              file=sys.stderr)
        return 1
    open(chemin, 'w').write(source.replace(AVANT, APRES, 1))
    print('  util.py   : schema « bo: » reconnu comme distant')
    return 0


if __name__ == '__main__':
    sys.exit(main())
