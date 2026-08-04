// Application Qt reelle pour Bouchaud OS : une fenetre dessinee sur /dev/fb0.
//
// C'est la validation que les sondes ne pouvaient pas donner. `qpa-probe.c`
// verifie que chaque appel de la sequence de demarrage de Qt aboutit ; ce
// programme-ci est la sequence, avec le vrai Qt au bout.
//
// Construction : voir tools/userland/build-qt.sh, qui produit un binaire
// statique-PIE embarquant Qt et le plugin de plateforme linuxfb.
//
// Le plugin est lie en dur (Q_IMPORT_PLUGIN) : un executable statique ne peut
// pas charger de `.so`, donc il n'y a pas de plugin a decouvrir a l'execution.

#include <QtGui/QGuiApplication>
#include <QtWidgets/QApplication>
#include <QtWidgets/QWidget>
#include <QtGui/QPainter>
#include <QtGui/QLinearGradient>
#include <QtGui/QScreen>
#include <QtCore/QTimer>
#include <QtCore/QDateTime>
#include <QtCore/QtPlugin>
#include <cstdio>

Q_IMPORT_PLUGIN(QLinuxFbIntegrationPlugin)

namespace {

// Combien de temps la fenetre reste affichee avant de rendre la main. Non nul
// pour qu'on puisse prendre une capture, borne pour que le scenario
// automatique se termine tout seul.
int dureeMs()
{
    const QByteArray demande = qgetenv("QT_DEMO_DUREE_MS");
    bool ok = false;
    const int valeur = demande.toInt(&ok);
    return ok && valeur > 0 ? valeur : 3000;
}

class Fenetre : public QWidget
{
public:
    Fenetre()
    {
        setWindowTitle(QStringLiteral("Bouchaud OS — Qt"));
    }

protected:
    void paintEvent(QPaintEvent *) override
    {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);

        QLinearGradient fond(0, 0, 0, height());
        fond.setColorAt(0.0, QColor(18, 26, 48));
        fond.setColorAt(1.0, QColor(8, 12, 24));
        p.fillRect(rect(), fond);

        const int marge = qMax(24, width() / 24);
        QRect carte = rect().adjusted(marge, marge, -marge, -marge);
        p.setPen(Qt::NoPen);
        p.setBrush(QColor(255, 255, 255, 18));
        p.drawRoundedRect(carte, 18, 18);

        QFont titre = p.font();
        titre.setPointSize(qMax(20, height() / 18));
        titre.setBold(true);
        p.setFont(titre);
        p.setPen(QColor(120, 200, 255));
        p.drawText(carte.adjusted(40, 40, -40, 0), Qt::AlignTop | Qt::AlignLeft,
                   QStringLiteral("Qt %1 sur Bouchaud OS").arg(QT_VERSION_STR));

        QFont corps = p.font();
        corps.setPointSize(qMax(12, height() / 36));
        corps.setBold(false);
        p.setFont(corps);
        p.setPen(QColor(220, 226, 240));

        const QString texte = QStringLiteral(
            "Plateforme QPA : %1\n"
            "Ecran : %2 x %3\n"
            "Rendu : QPainter, tampon /dev/fb0\n"
            "Date : %4\n\n"
            "Cette fenetre est peinte par le vrai Qt, en ring 3,\n"
            "sur un noyau ecrit depuis zero.")
            .arg(QGuiApplication::platformName())
            .arg(width())
            .arg(height())
            .arg(QDateTime::currentDateTime().toString(QStringLiteral("yyyy-MM-dd HH:mm:ss")));
        p.drawText(carte.adjusted(40, 40 + titre.pointSize() * 3, -40, -40),
                   Qt::AlignTop | Qt::AlignLeft | Qt::TextWordWrap, texte);

        // Une rangee de pastilles : de quoi voir d'un coup d'œil si les formes
        // pleines, l'anticrenelage et le melange alpha se comportent.
        const int rayon = qMax(10, height() / 40);
        int x = carte.left() + 40 + rayon;
        const int y = carte.bottom() - 40 - rayon;
        const QColor couleurs[] = {
            QColor(233, 84, 84), QColor(240, 173, 78),
            QColor(92, 184, 92), QColor(91, 192, 222), QColor(150, 120, 230),
        };
        for (const QColor &c : couleurs) {
            p.setBrush(c);
            p.setPen(Qt::NoPen);
            p.drawEllipse(QPoint(x, y), rayon, rayon);
            x += rayon * 3;
        }
    }
};

} // namespace

int main(int argc, char **argv)
{
    QApplication app(argc, argv);

    std::printf("[qt] plateforme : %s\n",
                QGuiApplication::platformName().toUtf8().constData());
    if (QScreen *ecran = QGuiApplication::primaryScreen()) {
        std::printf("[qt] ecran : %dx%d, %d bits\n",
                    ecran->geometry().width(), ecran->geometry().height(),
                    ecran->depth());
    }

    Fenetre fenetre;
    fenetre.showFullScreen();

    // Deux passes de peinture au moins : la premiere prouve que ca s'affiche,
    // la seconde qu'un redessin declenche par la boucle d'evenements arrive
    // bien jusqu'au tampon — c'est-a-dire que la boucle tourne vraiment.
    QTimer rafraichir;
    QObject::connect(&rafraichir, &QTimer::timeout, [&fenetre] {
        std::printf("[qt] redessin declenche par la boucle d'evenements\n");
        fenetre.update();
    });
    rafraichir.start(dureeMs() / 3);

    QTimer::singleShot(dureeMs(), &app, [] {
        std::printf("[qt] fin de la demonstration\n");
        QCoreApplication::quit();
    });

    const int code = app.exec();
    std::printf("[qt] boucle d'evenements terminee (code %d)\n", code);
    return code;
}
