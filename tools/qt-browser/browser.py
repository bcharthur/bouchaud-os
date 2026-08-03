"""Navigateur Web en Python avec PyQt5 — le tutoriel applique a la lettre.

Source : « How to build a browser using Python ? », scientyficworld.org

C'est la version de reference : `QWebEngineView` affiche les pages, donc le
moteur de rendu est Chromium, embarque par Qt. Elle tourne sur un poste de
travail ordinaire, **pas** dans Bouchaud OS : Qt est une bibliotheque C++ qui
reclame une libc, un editeur de liens dynamique et un serveur graphique, et
QtWebEngine y ajoute Chromium. Le noyau `no_std` n'offre aucun des trois, et
son `pip` n'installe que des paquets purs Python.

La version qui tourne dans l'OS est `src/assets/python/browser.py` : meme
structure (onglets, barre d'adresse, precedent / suivant / recharger /
accueil), rendu branche sur la console du noyau.

Installation :
    pip install PyQt5 PyQtWebEngine

Usage :
    python browser.py
    python browser.py --url https://example.com
    QT_QPA_PLATFORM=offscreen python browser.py --shot capture.png
"""

import sys

from PyQt5.QtCore import QUrl
from PyQt5.QtWidgets import (
    QAction, QApplication, QLineEdit, QMainWindow, QTabWidget, QToolBar,
)
from PyQt5.QtWebEngineWidgets import QWebEngineView

HOME = "https://duckduckgo.com/"


class MainWindow(QMainWindow):
    """Fenetre principale : onglets, barre d'outils, barre d'adresse."""

    def __init__(self, start_url=HOME):
        super(MainWindow, self).__init__()

        # Le conteneur d'onglets occupe toute la fenetre.
        self.tabs = QTabWidget()
        self.tabs.setDocumentMode(True)
        self.tabs.tabBarDoubleClicked.connect(self.tab_open_doubleclick)
        self.tabs.currentChanged.connect(self.current_tab_changed)
        self.tabs.setTabsClosable(True)
        self.tabs.tabCloseRequested.connect(self.close_current_tab)
        self.setCentralWidget(self.tabs)

        self.status = self.statusBar()

        # Barre de navigation.
        navbar = QToolBar("Navigation")
        self.addToolBar(navbar)

        back = QAction("←", self)
        back.setStatusTip("Page precedente")
        back.triggered.connect(lambda: self.current_browser().back())
        navbar.addAction(back)

        forward = QAction("→", self)
        forward.setStatusTip("Page suivante")
        forward.triggered.connect(lambda: self.current_browser().forward())
        navbar.addAction(forward)

        reload_action = QAction("⟳", self)
        reload_action.setStatusTip("Recharger la page")
        reload_action.triggered.connect(lambda: self.current_browser().reload())
        navbar.addAction(reload_action)

        home = QAction("⌂", self)
        home.setStatusTip("Page d'accueil")
        home.triggered.connect(self.navigate_home)
        navbar.addAction(home)

        navbar.addSeparator()

        self.urlbar = QLineEdit()
        self.urlbar.returnPressed.connect(self.navigate_to_url)
        navbar.addWidget(self.urlbar)

        new_tab = QAction("+", self)
        new_tab.setStatusTip("Nouvel onglet")
        new_tab.triggered.connect(lambda: self.add_new_tab(QUrl(HOME), "Nouvel onglet"))
        navbar.addAction(new_tab)

        self.add_new_tab(QUrl(start_url), "Accueil")
        self.setWindowTitle("Navigateur Python")
        self.resize(1200, 800)
        self.show()

    # -- Onglets -----------------------------------------------------------

    def add_new_tab(self, qurl=None, label="Blanc"):
        if qurl is None:
            qurl = QUrl(HOME)

        browser = QWebEngineView()
        browser.setUrl(qurl)
        index = self.tabs.addTab(browser, label)
        self.tabs.setCurrentIndex(index)

        # `browser` est capture par les lambdas : sans lui, un onglet en
        # arriere-plan qui finit de charger ecraserait la barre d'adresse
        # de l'onglet actif.
        browser.urlChanged.connect(
            lambda qurl, view=browser: self.update_urlbar(qurl, view)
        )
        browser.loadFinished.connect(
            lambda _, view=browser: self.tabs.setTabText(
                self.tabs.indexOf(view), view.page().title()[:24]
            )
        )
        return browser

    def tab_open_doubleclick(self, index):
        # Un double-clic sur le fond de la barre d'onglets en ouvre un nouveau.
        if index == -1:
            self.add_new_tab()

    def current_tab_changed(self, index):
        browser = self.current_browser()
        if browser is not None:
            self.update_urlbar(browser.url(), browser)

    def close_current_tab(self, index):
        if self.tabs.count() < 2:
            return
        self.tabs.removeTab(index)

    def current_browser(self):
        return self.tabs.currentWidget()

    # -- Navigation --------------------------------------------------------

    def navigate_home(self):
        self.current_browser().setUrl(QUrl(HOME))

    def navigate_to_url(self):
        text = self.urlbar.text().strip()
        qurl = QUrl(text)
        if qurl.scheme() == "":
            qurl.setScheme("https")
        self.current_browser().setUrl(qurl)

    def update_urlbar(self, qurl, browser=None):
        # Seul l'onglet visible pilote la barre d'adresse.
        if browser is not self.current_browser():
            return
        self.urlbar.setText(qurl.toString())
        self.urlbar.setCursorPosition(0)
        self.setWindowTitle("%s — Navigateur Python" % qurl.host())


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    start_url = HOME
    shot = None

    index = 0
    while index < len(argv):
        if argv[index] == "--url" and index + 1 < len(argv):
            index += 1
            start_url = argv[index]
        elif argv[index] == "--shot" and index + 1 < len(argv):
            index += 1
            shot = argv[index]
        index += 1

    app = QApplication(sys.argv)
    app.setApplicationName("Navigateur Python")
    window = MainWindow(start_url)

    if shot:
        # Mode non interactif : laisse la page se charger, capture, puis sort.
        from PyQt5.QtCore import QTimer

        def capture():
            window.grab().save(shot)
            print("capture ecrite dans %s" % shot)
            app.quit()

        QTimer.singleShot(6000, capture)

    return app.exec_()


if __name__ == "__main__":
    sys.exit(main())
