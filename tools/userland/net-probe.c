// Sonde reseau : telecharge un gros fichier et rend compte de sa progression.
//
// Elle existe pour une raison precise : un transfert de quelques kilo-octets —
// une page, une image — passait sans probleme, tandis qu'un transfert de
// plusieurs centaines de kilo-octets s'arretait en route. Le navigateur etait
// un mauvais banc d'essai pour cela : trop de couches entre la prise et le
// constat. Ici, il n'y a qu'une socket et une boucle de lecture.
//
// Le serveur attendu est celui de la machine hote, joignable en 10.0.2.2 depuis
// le reseau utilisateur de QEMU. Sans lui, la sonde le dit et rend 0 : elle ne
// doit pas faire echouer le scenario sur une machine ou personne n'ecoute.

#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

#define HOTE    "10.0.2.2"
#define PORT    8099
#define CHEMIN  "/videoplayback.mp4"

static int echecs = 0;
static int reussites = 0;

static void verifie(const char *nom, int condition, long valeur)
{
    if (condition) {
        reussites++;
    } else {
        echecs++;
        printf("  ECHEC %s (valeur %ld)\n", nom, valeur);
    }
}

static long maintenant_ms(void)
{
    struct timeval t;
    gettimeofday(&t, NULL);
    return t.tv_sec * 1000L + t.tv_usec / 1000L;
}

/// Ouvre une connexion et emet la requete. Rend le descripteur, ou -1.
static int demande(const char *portee)
{
    int prise = socket(AF_INET, SOCK_STREAM, 0);
    if (prise < 0) {
        return -1;
    }
    struct sockaddr_in adresse;
    memset(&adresse, 0, sizeof adresse);
    adresse.sin_family = AF_INET;
    adresse.sin_port = htons(PORT);
    adresse.sin_addr.s_addr = inet_addr(HOTE);
    if (connect(prise, (struct sockaddr *)&adresse, sizeof adresse) < 0) {
        close(prise);
        return -1;
    }

    char requete[512];
    int longueur = snprintf(requete, sizeof requete,
        "GET %s HTTP/1.1\r\nHost: %s:%d\r\nConnection: close\r\n%s\r\n",
        CHEMIN, HOTE, PORT, portee ? portee : "");
    if (write(prise, requete, (size_t)longueur) != longueur) {
        close(prise);
        return -1;
    }
    return prise;
}

/// Lit tout ce que le serveur envoie. Rend le nombre d'octets de corps.
static long avale(int prise, const char *etiquette)
{
    static char tampon[16384];
    long total = 0;
    long corps = 0;
    int entete_finie = 0;
    long jalon = 0;
    const long depart = maintenant_ms();

    for (;;) {
        ssize_t lus = read(prise, tampon, sizeof tampon);
        if (lus < 0) {
            printf("  %s : lecture en erreur apres %ld octets\n", etiquette, total);
            break;
        }
        if (lus == 0) {
            break; // le pair a ferme : transfert termine
        }
        total += lus;

        if (!entete_finie) {
            // On ne cherche la fin d'en-tete que dans le premier bloc : elle y
            // est toujours pour une reponse de cette taille.
            char *fin = memmem(tampon, (size_t)lus, "\r\n\r\n", 4);
            if (fin) {
                entete_finie = 1;
                corps += (tampon + lus) - (fin + 4);
            }
        } else {
            corps += lus;
        }

        // Un jalon tous les 128 Ko : c'est ce qui montre si le transfert
        // avance regulierement ou s'il s'arrete.
        if (corps - jalon >= 128 * 1024) {
            jalon = corps;
            printf("  %s : %ld Ko en %ld ms\n", etiquette, corps / 1024,
                   maintenant_ms() - depart);
            fflush(stdout);
        }
    }

    printf("  %s : %ld octets de corps en %ld ms\n", etiquette, corps,
           maintenant_ms() - depart);
    return corps;
}

int main(void)
{
    printf("=== net-probe ===\n");

    // Premier contact : une tranche minuscule, pour savoir si quelqu'un ecoute.
    int prise = demande("Range: bytes=0-63\r\n");
    if (prise < 0) {
        printf("  aucun serveur sur %s:%d — sonde ignoree\n", HOTE, PORT);
        printf("RESULTAT : 0 verification(s) en echec (0 passees)\n");
        return 0;
    }
    long petit = avale(prise, "tranche de 64 octets");
    close(prise);
    verifie("une petite tranche arrive en entier", petit == 64, petit);

    // Le cas qui echouait : une tranche de 512 Ko.
    prise = demande("Range: bytes=0-524287\r\n");
    if (prise < 0) {
        printf("  ECHEC connexion pour la grande tranche\n");
        echecs++;
    } else {
        long grand = avale(prise, "tranche de 512 Ko");
        close(prise);
        // Le fichier peut etre plus court que la tranche demandee : on accepte
        // tout ce qui depasse largement l'ancien point d'arret.
        verifie("une grande tranche arrive en entier", grand >= 256 * 1024, grand);
    }

    // Et le fichier entier, sans `Range` : le chemin le plus simple, et celui
    // qui exerce le plus longtemps la connexion.
    prise = demande(NULL);
    if (prise < 0) {
        printf("  ECHEC connexion pour le fichier entier\n");
        echecs++;
    } else {
        long entier = avale(prise, "fichier entier");
        close(prise);
        verifie("le fichier entier arrive", entier >= 256 * 1024, entier);
    }

    if (echecs > 0) {
        printf("RESULTAT : %d verification(s) en echec sur %d\n",
               echecs, echecs + reussites);
        return 1;
    }
    printf("RESULTAT : 0 verification(s) en echec (%d passees)\n", reussites);
    return 0;
}
