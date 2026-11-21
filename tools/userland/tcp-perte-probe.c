// Sonde TCP ring 3 -- un segment de donnees qui ne part pas doit etre renvoye.
//
// ## L'incident qu'elle reproduit
//
// Le 18/11, sous WHPX, le consentement YouTube (« Saving your choice ») a
// tourne quatre minutes. Le journal de RequestServer montre trois POST vers
// www.youtube.com partis a 10:58:48, 10:58:54 et 10:59:37 -- sur une connexion
// HTTP/2 deja ouverte et muette depuis 10:58:31 -- puis plus rien jusqu'a
// 11:02:35, soit l'echeance d'inactivite de Google (240 s) comptee depuis la
// DERNIERE activite qu'il avait vue. Le serveur n'avait donc jamais recu ces
// requetes. La sauvegarde du consentement attendait la premiere.
//
// La cause est dans le noyau : `TcpConn::send` ignorait le verdict de
// `send_ip_immediat`. Quand l'entree ARP de la passerelle a expire (TTL de
// 60 s, jamais rafraichi par l'usage), la trame n'est PAS emise, une requete
// ARP part a sa place, et le numero de sequence avance quand meme. Rien ne
// retransmettait sur ce chemin : le pair attend pour toujours l'octet manquant.
//
// ## Ce qu'elle fait
//
//   E1  une requete juste apres l'ouverture       -> repond (temoin)
//   --  65 s de silence : l'ARP de la passerelle expire
//   E2  une requete de trois segments              -> DOIT repondre vite
//   E3  une requete juste derriere                 -> le flux reste en ordre
//
// Le serveur, cote hote, verifie chaque octet : une retransmission qui
// dupliquerait ou decalerait le flux est detectee, pas seulement une perte.
//
// Sans libc, comme `dns-probe.c` : clang de l'hote, instruction `syscall`.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;

#define SYS_poll           7
#define SYS_write          1
#define SYS_nanosleep     35
#define SYS_socket        41
#define SYS_connect       42
#define SYS_sendto        44
#define SYS_recvfrom      45
#define SYS_exit          60
#define SYS_clock_gettime 228
#define SYS_fcntl         72
#define F_GETFL 3
#define F_SETFL 4
#define O_NONBLOCK 04000
#define POLLOUT 4
#define EAGAIN 11

#define AF_INET     2
#define SOCK_STREAM 1
#define POLLIN      1
#define CLOCK_MONOTONIC 1

#ifndef PORT_SERVEUR
#define PORT_SERVEUR 18457
#endif

/// Silence impose entre E1 et E2. Doit depasser le TTL ARP du noyau (60 s).
#ifndef SILENCE_S
#define SILENCE_S 65
#endif

/// Une reponse plus lente que cela est un echec. Le RTO initial est d'une
/// seconde ; trois expirations successives tiennent en sept. Dix laissent de la
/// marge a TCG sans confondre une reprise avec l'echeance de 240 s d'un vrai
/// serveur.
#define ECHEANCE_REPONSE_MS 10000

static i64 sys3(i64 n, i64 a, i64 b, i64 c) {
    i64 r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c)
                     : "rcx", "r11", "memory");
    return r;
}
static i64 sys6(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    i64 r;
    register i64 r10 __asm__("r10") = d;
    register i64 r8 __asm__("r8") = e;
    register i64 r9 __asm__("r9") = f;
    __asm__ volatile("syscall" : "=a"(r)
                     : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return r;
}

static u64 longueur(char const* s) { u64 n = 0; while (s[n]) ++n; return n; }
static void ecris(char const* s) { sys3(SYS_write, 1, (i64)s, (i64)longueur(s)); }
static char* decimal(char* fin, u64 v) {
    *--fin = 0;
    do {
        *--fin = (char)('0' + (v % 10));
        v /= 10;
    } while (v > 0);
    return fin;
}
static void ecris_u64(u64 v) {
    char tampon[21];
    ecris(decimal(tampon + sizeof(tampon), v));
}
static void ecris_i64(i64 v) {
    if (v < 0) { ecris("-"); ecris_u64((u64)(-v)); return; }
    ecris_u64((u64)v);
}

static u64 maintenant_ms(void) {
    struct { i64 sec; i64 nsec; } t = { 0, 0 };
    if (sys3(SYS_clock_gettime, CLOCK_MONOTONIC, (i64)&t, 0) < 0) return 0;
    return (u64)t.sec * 1000u + (u64)(t.nsec / 1000000);
}

struct sockaddr_in { u16 famille; u16 port_be; u32 adresse_be; u8 zero[8]; };

/// L'octet `i` du corps de la requete `n`. Le serveur recalcule la meme suite :
/// un octet duplique ou decale par une retransmission fausse se voit.
static u8 motif(u32 n, u32 i) { return (u8)('a' + ((i * 7u + n * 13u) % 26u)); }

/// 256 Kio de corps : plus que les 64 segments (~90 Kio) que la file
/// d'emission du noyau garde en vol -- l'emetteur DOIT attendre des ACK.
#define GROS (256u * 1024u)
static u8 tampon[GROS + 64];
/// `EAGAIN` vus pendant l'envoi non bloquant (E5).
static u64 eagain_vus;

/// Envoie la requete `n` (en-tete `REQ<n> <taille>\n`, puis `taille` octets) et
/// attend `OK<n>`. Rend la latence en ms, ou -1 si rien n'est revenu a temps,
/// ou -2 si le serveur a vu un flux corrompu.
static i64 echange(i64 fd, u32 n, u32 taille, int non_bloquant) {
    u32 pos = 0;
    tampon[pos++] = 'R'; tampon[pos++] = 'E'; tampon[pos++] = 'Q';
    tampon[pos++] = (u8)('0' + n);
    tampon[pos++] = ' ';
    char chiffres[12];
    int k = 0;
    u32 t = taille;
    do { chiffres[k++] = (char)('0' + t % 10); t /= 10; } while (t);
    while (k) tampon[pos++] = (u8)chiffres[--k];
    tampon[pos++] = '\n';
    for (u32 i = 0; i < taille; ++i) tampon[pos++] = motif(n, i);

    u64 debut = maintenant_ms();
    u32 envoye = 0;
    while (envoye < pos) {
        i64 r = sys6(SYS_sendto, fd, (i64)(tampon + envoye), (i64)(pos - envoye), 0, 0, 0);
        if (non_bloquant && r == -EAGAIN) {
            // La file est pleine : POLLOUT doit revenir quand le pair acquitte.
            ++eagain_vus;
            struct { int fd; short events; short revents; } p = { (int)fd, POLLOUT, 0 };
            if (sys3(SYS_poll, (i64)&p, 1, 2000) <= 0 && maintenant_ms() - debut > ECHEANCE_REPONSE_MS) {
                ecris("tcp-perte: POLLOUT jamais revenu\n");
                return -1;
            }
            continue;
        }
        if (r <= 0) {
            ecris("tcp-perte: sendto="); ecris_i64(r); ecris("\n");
            return -1;
        }
        envoye += (u32)r;
    }

    u8 reponse[64];
    u32 recu = 0;
    while (maintenant_ms() - debut < ECHEANCE_REPONSE_MS) {
        struct { int fd; short events; short revents; } p = { (int)fd, POLLIN, 0 };
        i64 pret = sys3(SYS_poll, (i64)&p, 1, 500);
        if (pret <= 0) continue;
        i64 r = sys6(SYS_recvfrom, fd, (i64)(reponse + recu), (i64)(sizeof(reponse) - 1 - recu), 0, 0, 0);
        if (r <= 0) {
            ecris("tcp-perte: recv="); ecris_i64(r); ecris("\n");
            return -1;
        }
        recu += (u32)r;
        if (recu > 0 && reponse[recu - 1] == '\n') break;
    }
    if (recu == 0 || reponse[recu - 1] != '\n') return -1;
    if (recu >= 3 && reponse[0] == 'O' && reponse[1] == 'K' && reponse[2] == (u8)('0' + n)) {
        return (i64)(maintenant_ms() - debut);
    }
    reponse[recu] = 0;
    ecris("tcp-perte: reponse inattendue "); ecris((char const*)reponse);
    return -2;
}

static char* ajoute_texte(char* dst, char const* texte) {
    while (*texte) *dst++ = *texte++;
    return dst;
}

static void verdict(char const* nom, i64 resultat) {
    // Un seul write pour que les traces noyau ne coupent pas le marqueur.
    char ligne[80];
    char chiffre[21];
    char* fin = ajoute_texte(ligne, "TCP_PERTE_");
    fin = ajoute_texte(fin, nom);
    if (resultat >= 0) {
        fin = ajoute_texte(fin, "_OK latence_ms=");
        fin = ajoute_texte(fin, decimal(chiffre + sizeof(chiffre), (u64)resultat));
    } else if (resultat == -2) {
        fin = ajoute_texte(fin, "_CORROMPU");
    } else {
        fin = ajoute_texte(fin, "_ECHEC attente_ms>=");
        fin = ajoute_texte(fin, decimal(chiffre + sizeof(chiffre), ECHEANCE_REPONSE_MS));
    }
    *fin++ = '\n';
    sys3(SYS_write, 1, (i64)ligne, (i64)(fin - ligne));
}

/// Point d'entree reel : voir `dns-probe.c`. Le noyau livre une pile alignee
/// sur 16 octets, le compilateur suppose l'alignement d'apres un `call`.
void principal(void) {
    i64 fd = sys3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    if (fd < 0) { ecris("TCP_PERTE_SOCKET_ECHEC\n"); sys3(SYS_exit, 1, 0, 0); }

    // 10.0.2.2 : la passerelle de SLIRP, qui transmet a 127.0.0.1 de l'hote.
    struct sockaddr_in a;
    a.famille = AF_INET;
    a.port_be = (u16)((PORT_SERVEUR << 8) | (PORT_SERVEUR >> 8));
    a.adresse_be = 10u | (0u << 8) | (2u << 16) | (2u << 24);
    for (int i = 0; i < 8; ++i) a.zero[i] = 0;
    i64 c = sys3(SYS_connect, fd, (i64)&a, sizeof(a));
    if (c < 0) { ecris("TCP_PERTE_CONNECT_ECHEC errno="); ecris_i64(c); ecris("\n"); sys3(SYS_exit, 1, 0, 0); }

    verdict("E1", echange(fd, 1, 200, 0));

    ecris("tcp-perte: silence de "); ecris_u64(SILENCE_S); ecris(" s (expiration ARP)\n");
    struct { i64 sec; i64 nsec; } duree = { SILENCE_S, 0 };
    sys3(SYS_nanosleep, (i64)&duree, 0, 0);

    // Trois segments : le premier part pendant que l'ARP est a refaire.
    verdict("E2", echange(fd, 2, 4000, 0));
    verdict("E3", echange(fd, 3, 200, 0));

    // E4 : envoi BLOQUANT plus gros que la file d'emission.
    verdict("E4", echange(fd, 4, GROS, 0));

    // E5 : le meme en NON BLOQUANT. Ecritures partielles et EAGAIN sont
    // permis ; un flux tronque ou duplique ne l'est pas (le serveur verifie).
    i64 drapeaux = sys3(SYS_fcntl, fd, F_GETFL, 0);
    sys3(SYS_fcntl, fd, F_SETFL, drapeaux | O_NONBLOCK);
    eagain_vus = 0;
    i64 e5 = echange(fd, 5, GROS, 1);
    verdict("E5", e5);
    ecris("tcp-perte: E5 eagain="); ecris_u64(eagain_vus); ecris("\n");
    sys3(SYS_fcntl, fd, F_SETFL, drapeaux);

    // E6 : le serveur repond puis FERME. La lecture suivante doit rendre 0
    // (fin de flux) rapidement -- pas une attente, pas une erreur.
    verdict("E6", echange(fd, 6, 100, 0));
    {
        u64 debut = maintenant_ms();
        i64 r = -1;
        u8 octet;
        while (maintenant_ms() - debut < ECHEANCE_REPONSE_MS) {
            struct { int fd; short events; short revents; } p = { (int)fd, POLLIN, 0 };
            if (sys3(SYS_poll, (i64)&p, 1, 500) <= 0) continue;
            r = sys6(SYS_recvfrom, fd, (i64)&octet, 1, 0, 0, 0);
            break;
        }
        if (r == 0) { ecris("TCP_PERTE_FIN_PAIR_OK latence_ms="); ecris_u64(maintenant_ms() - debut); ecris("\n"); }
        else { ecris("TCP_PERTE_FIN_PAIR_ECHEC recv="); ecris_i64(r); ecris("\n"); }
    }
    ecris("TCP_PERTE_FIN\n");
    sys3(SYS_exit, 0, 0, 0);
}

__attribute__((naked)) void _start(void) {
    __asm__ volatile(
        "xorq %rbp, %rbp\n\t"
        "andq $-16, %rsp\n\t"
        "movabsq $principal, %rax\n\t"
        "callq *%rax\n\t"
        "ud2\n\t");
}
