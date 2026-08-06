"""Le tuto pywebview, tel quel, sur Bouchaud OS.

    exec /bo-navigateur /usr/share/bo-navigateur/exemple-webview.py
    exec /bo-navigateur /usr/share/bo-navigateur/exemple-webview.py https://…

Rien dans ce fichier n'est propre a Bouchaud OS : c'est l'API publique de
pywebview, celle de n'importe quel tutoriel. C'est tout l'interet — le moteur
d'affichage est choisi par la bibliotheque, et celui de l'OS en est un parmi
les autres (`webview/platforms/bouchaud.py`).
"""

import sys
import threading
import time

import webview

PAGE = """
<!doctype html>
<html>
  <head><title>pywebview sur Bouchaud OS</title></head>
  <body style="font-family: sans-serif; margin: 0; background-color: #f6f8fb">
    <div style="background-color:#12203c; color:#fff; padding:32px 40px">
      <h1 style="margin:0; font-size:32px">pywebview</h1>
      <p style="margin:8px 0 0; color:#a9c4ee">
        La meme API que sur un poste de travail, sur un noyau ecrit depuis zero.
      </p>
    </div>
    <div style="padding:26px 40px">
      <h2>Ce que fait ce programme</h2>
      <ol>
        <li><code>webview.create_window(titre, html=…)</code> cree la fenetre</li>
        <li><code>webview.start(func)</code> lance la boucle et un fil de travail</li>
        <li>le fil appelle <code>window.load_url(…)</code> au bout de quelques secondes</li>
      </ol>
      <h2>Sous le capot</h2>
      <ul>
        <li>moteur d'affichage : <b>bouchaud</b> — analyse HTML, cascade CSS,
            mise en page et peinture ecrites pour cet OS</li>
        <li>surface : Qt 5.15 sur <code>/dev/fb0</code>, en ring 3</li>
        <li>interprete : CPython 3.12 embarque dans le meme binaire</li>
      </ul>
      <p><a href="bo:apropos">Ce que le moteur sait faire, en detail</a></p>
    </div>
  </body>
</html>
"""


def travail(fenetre):
    """Fonction passee a `webview.start` : elle tourne dans un autre fil."""
    print('[exemple] moteur d\'affichage :', webview.renderer)
    print('[exemple] ecrans :', [str(e) for e in webview.screens])

    # `loaded` est arme par le moteur apres chaque chargement.
    fenetre.events.loaded.wait(10)
    print('[exemple] page initiale chargee, titre :', fenetre.title)

    time.sleep(4)
    cible = sys.argv[1] if len(sys.argv) > 1 else 'http://archive.ubuntu.com/ubuntu/'
    print('[exemple] navigation vers', cible)
    fenetre.load_url(cible)

    fenetre.events.loaded.wait(20)
    print('[exemple] adresse courante :', fenetre.get_current_url())
    print('[exemple] taille de la fenetre :', fenetre.width, 'x', fenetre.height)


if __name__ == '__main__':
    fenetre = webview.create_window(
        'Bouchaud OS — pywebview',
        html=PAGE,
        width=1280,
        height=720,
    )
    webview.start(travail, fenetre)
    print('[exemple] termine')
