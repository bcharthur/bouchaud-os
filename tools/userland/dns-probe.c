// Sonde DNS ring 3 — reproduit, sans Ladybird, le defaut de la couche UDP.
//
// ## Pourquoi elle existe
//
// Le blocage de la navigation par nom se voyait seulement au bout d'un build
// Ladybird complet, soit quatre-vingt-dix minutes, et sur un journal ou trois
// processus parlent en meme temps. Cette sonde reduit la boucle a un ELF de
// quelques kilooctets : elle ouvre des sockets UDP, envoie de vraies requetes
// DNS, et dit ce qui revient.
//
// ## Ce qu'elle mesure
//
//   1. un socket seul  : une requete, une reponse. Le cas qui marchait deja.
//   2. deux sockets    : deux requetes en vol. Le cas de LibDNS, qui interroge
//                        A et AAAA, et le cas ou le defaut apparait.
//   3. le temps        : une reponse qui n'arrive pas doit couter une attente,
//                        pas un cœur a 100 %.
//
// ## Pourquoi sans libc
//
// Le userland Bouchaud se construit avec une chaine croisee d'une heure. Une
// sonde qui en depend ne serait jamais lancee pendant un diagnostic. Celle-ci
// n'utilise que l'instruction `syscall` et se compile en une seconde avec le
// clang de l'hote.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;

#define SYS_write          1
#define SYS_poll           7
#define SYS_socket        41
#define SYS_sendto        44
#define SYS_recvfrom      45
#define SYS_bind          49
#define SYS_exit          60
#define SYS_ioctl         16
#define SYS_connect       42
#define SYS_fcntl         72
#define SYS_clock_gettime 228

#define AF_INET       2
#define SOCK_DGRAM    2
#define O_NONBLOCK 04000
#define F_GETFL        3
#define F_SETFL        4
#define POLLIN         1
#define CLOCK_MONOTONIC 1
#define CLOCK_PROCESS_CPUTIME_ID 2
#define MSG_DONTWAIT 0x40
#define FIONREAD 0x541B

static i64 sys1(i64 n, i64 a) {
    i64 r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a) : "rcx", "r11", "memory");
    return r;
}
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

static void ecris_u64(u64 v) {
    char tampon[24];
    int i = 23;
    tampon[i--] = 0;
    if (v == 0) tampon[i--] = '0';
    while (v > 0) { tampon[i--] = (char)('0' + (v % 10)); v /= 10; }
    ecris(&tampon[i + 1]);
}

static void ecris_i64(i64 v) {
    if (v < 0) { ecris("-"); ecris_u64((u64)(-v)); return; }
    ecris_u64((u64)v);
}

/// Les identifiants DNS se lisent en hexadecimal ; les ecrire en decimal
/// derriere un prefixe « 0x » ferait croire a une transaction qui n'est pas
/// celle qu'on a emise.
static void ecris_hex16(u16 v) {
    char const* chiffres = "0123456789abcdef";
    char tampon[5];
    for (int i = 0; i < 4; ++i) tampon[3 - i] = chiffres[(v >> (i * 4)) & 0xf];
    tampon[4] = 0;
    ecris(tampon);
}

static u64 horloge_ms(i64 horloge) {
    struct { i64 sec; i64 nsec; } t = { 0, 0 };
    if (sys3(SYS_clock_gettime, horloge, (i64)&t, 0) < 0) return 0;
    return (u64)t.sec * 1000u + (u64)(t.nsec / 1000000);
}

/// Millisecondes monotones : le temps qui passe au mur.
static u64 maintenant_ms(void) { return horloge_ms(CLOCK_MONOTONIC); }

/// Millisecondes de **processeur** consommees par ce processus.
///
/// C'est la seule mesure qui distingue une attente qui dort d'une attente qui
/// brule un cœur : les deux durent le meme temps au mur. Une duree ecoulee ne
/// prouve donc rien sur le cout d'une attente, et c'est pour cela que le noyau
/// expose desormais `CLOCK_PROCESS_CPUTIME_ID`.
static u64 processeur_ms(void) { return horloge_ms(CLOCK_PROCESS_CPUTIME_ID); }

struct sockaddr_in {
    u16 famille;
    u16 port_be;
    u32 adresse_be;
    u8 zero[8];
};

static u16 vers_be16(u16 v) { return (u16)((v << 8) | (v >> 8)); }

static void remplis_adresse(struct sockaddr_in* a, u8 o1, u8 o2, u8 o3, u8 o4, u16 port) {
    a->famille = AF_INET;
    a->port_be = vers_be16(port);
    a->adresse_be = (u32)o1 | ((u32)o2 << 8) | ((u32)o3 << 16) | ((u32)o4 << 24);
    for (int i = 0; i < 8; ++i) a->zero[i] = 0;
}

/// Construit une requete DNS de type A. Rien d'exotique : en-tete de douze
/// octets, le nom en etiquettes, puis QTYPE/QCLASS.
static int fabrique_requete(u8* out, u16 identifiant, char const* nom) {
    int n = 0;
    out[n++] = (u8)(identifiant >> 8);
    out[n++] = (u8)identifiant;
    out[n++] = 0x01; out[n++] = 0x00;  // recursion demandee
    out[n++] = 0x00; out[n++] = 0x01;  // une question
    out[n++] = 0x00; out[n++] = 0x00;
    out[n++] = 0x00; out[n++] = 0x00;
    out[n++] = 0x00; out[n++] = 0x00;

    int debut = n++;
    int compte = 0;
    for (char const* p = nom;; ++p) {
        if (*p == '.' || *p == 0) {
            out[debut] = (u8)compte;
            debut = n;
            if (*p == 0) break;
            ++n;
            compte = 0;
        } else {
            out[n++] = (u8)*p;
            ++compte;
        }
    }
    out[debut] = 0;
    n = debut + 1;

    out[n++] = 0x00; out[n++] = 0x01;  // QTYPE  = A
    out[n++] = 0x00; out[n++] = 0x01;  // QCLASS = IN
    return n;
}

static int ouvre_udp(void) {
    return (int)sys3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
}

/// Lie le socket a un port ephemere explicite : deux sockets doivent avoir deux
/// ports differents, sinon le demultiplexage ne veut rien dire.
static int lie(int fd, u16 port) {
    struct sockaddr_in local;
    remplis_adresse(&local, 0, 0, 0, 0, port);
    return (int)sys3(SYS_bind, fd, (i64)&local, sizeof(local));
}

static int envoie_requete(int fd, u16 identifiant, char const* nom,
                          u8 s1, u8 s2, u8 s3, u8 s4) {
    u8 requete[512];
    int taille = fabrique_requete(requete, identifiant, nom);
    struct sockaddr_in serveur;
    remplis_adresse(&serveur, s1, s2, s3, s4, 53);
    return (int)sys6(SYS_sendto, fd, (i64)requete, taille, 0,
                     (i64)&serveur, sizeof(serveur));
}

/// Attend une reponse en non bloquant, et rend le temps passe. Le mode non
/// bloquant est celui de LibDNS : c'est lui qui revele une attente active.
static i64 attends_reponse(int fd, u64 delai_ms, u64* duree_ms, u16* identifiant) {
    u8 tampon[2048];
    u64 depart = maintenant_ms();
    for (;;) {
        i64 n = sys6(SYS_recvfrom, fd, (i64)tampon, sizeof(tampon), 0, 0, 0);
        if (n > 0) {
            *duree_ms = maintenant_ms() - depart;
            *identifiant = (u16)(((u16)tampon[0] << 8) | tampon[1]);
            return n;
        }
        if (maintenant_ms() - depart >= delai_ms) {
            *duree_ms = maintenant_ms() - depart;
            *identifiant = 0;
            return n < 0 ? n : -1;
        }
    }
}

static void titre(char const* s) { ecris("\n[dns-probe] "); ecris(s); ecris("\n"); }

/// Laisse passer `duree_ms` en interrogeant l'horloge. Une sonde n'a pas besoin
/// de `nanosleep` : ce qui compte ici est que du temps s'ecoule pendant que la
/// carte recoit, pas que le processeur se repose.
static void patiente(u64 duree_ms) {
    u64 depart = maintenant_ms();
    while (maintenant_ms() - depart < duree_ms) { }
}

/// Un seul passage non bloquant. C'est cet appel qui vide l'anneau de reception
/// pour le compte du socket interroge — et c'est la que se decidait le sort des
/// datagrammes adresses a un autre.
static i64 sonde_non_bloquante(int fd) {
    u8 tampon[2048];
    return sys6(SYS_recvfrom, fd, (i64)tampon, sizeof(tampon), MSG_DONTWAIT, 0, 0);
}

/// Point d'entree reel. `_start` ne peut pas porter ce corps : le noyau livre
/// une pile alignee sur 16 octets, comme Linux, alors que le compilateur suppose
/// l'alignement d'APRES un `call` — soit huit octets de decalage. Un `movaps`
/// emis pour une variable locale fautait donc en ring 3. On aligne et on appelle.
void principal(void) {
    ecris("[dns-probe] DEBUT\n");

    // Le resolveur du NAT de QEMU. Surchargeable nulle part exprès : cette
    // sonde teste la pile, pas la configuration.
    u8 const s1 = 10, s2 = 0, s3 = 2, s4 = 3;

    // ---- Cas 1 : un seul socket ---------------------------------------
    titre("CAS 1 : un socket, une requete");
    {
        int fd = ouvre_udp();
        if (fd < 0) { ecris("[dns-probe] ECHEC socket errno="); ecris_i64(fd); ecris("\n"); sys1(SYS_exit, 1); }
        lie(fd, 40001);
        i64 envoye = envoie_requete(fd, 0x1111, "example.com", s1, s2, s3, s4);
        ecris("[dns-probe] envoye="); ecris_i64(envoye); ecris("\n");

        u64 duree = 0; u16 ident = 0;
        i64 recu = attends_reponse(fd, 3000, &duree, &ident);
        ecris("[dns-probe] CAS1 recu="); ecris_i64(recu);
        ecris(" id=0x"); ecris_hex16(ident);
        ecris(" duree_ms="); ecris_u64(duree); ecris("\n");
        if (recu > 0 && ident == 0x1111) ecris("[dns-probe] CAS1_OK\n");
        else ecris("[dns-probe] CAS1_ECHEC\n");
    }

    // ---- Cas 2 : un datagramme adresse a un autre socket ---------------
    //
    // Le scenario est rendu **deterministe**, et c'est le point important : une
    // premiere version envoyait deux requetes puis lisait les deux reponses, ce
    // qui laissait la panne dependre de l'ordre d'arrivee. Elle passait une fois
    // sur deux sur un noyau pourtant casse — donc elle ne prouvait rien.
    //
    // Ici la reponse de B est **certainement** dans l'anneau quand A le draine :
    //
    //   1. B emet sa requete ;
    //   2. on laisse passer 300 ms, largement plus que les ~15 ms de reponse ;
    //   3. A fait une lecture non bloquante — c'est elle qui vide l'anneau ;
    //   4. B lit a son tour.
    //
    // Une couche qui route rend la reponse a B. Une couche qui jette ce qui ne
    // va pas au socket qu'elle regarde l'a perdue pour toujours.
    titre("CAS 2 : la reponse de B doit survivre a une lecture de A");
    {
        int a = ouvre_udp();
        int b = ouvre_udp();
        if (a < 0 || b < 0) { ecris("[dns-probe] ECHEC socket x2\n"); sys1(SYS_exit, 1); }
        lie(a, 40002);
        lie(b, 40003);

        envoie_requete(b, 0x3333, "example.org", s1, s2, s3, s4);
        patiente(300);

        i64 vol = sonde_non_bloquante(a);
        ecris("[dns-probe] CAS2 lecture de A (ne doit rien recevoir) = ");
        ecris_i64(vol); ecris("\n");

        u64 duree_b = 0; u16 ident_b = 0;
        i64 recu_b = attends_reponse(b, 3000, &duree_b, &ident_b);
        ecris("[dns-probe] CAS2 socketB recu="); ecris_i64(recu_b);
        ecris(" id=0x"); ecris_hex16(ident_b);
        ecris(" duree_ms="); ecris_u64(duree_b); ecris("\n");

        if (vol > 0)
            ecris("[dns-probe] CAS2_ECHEC socket A a recu un datagramme qui ne lui est pas adresse\n");
        else if (recu_b > 0 && ident_b == 0x3333)
            ecris("[dns-probe] CAS2_OK\n");
        else
            ecris("[dns-probe] CAS2_ECHEC demultiplexage : la reponse de B a ete jetee\n");
    }

    // ---- Cas 3 : personne ne repond, cout **processeur** de l'attente --
    //
    // Ce cas mesurait la duree ecoulee, ce qui ne prouvait rien : la boucle a
    // plein processeur d'avant la correction rendait la main au bout de ~4,9 s,
    // donc elle aurait passe le meme seuil qu'une attente qui dort. Une montre
    // ne dit pas si le processeur travaille.
    //
    // On compare donc deux horloges. Une attente qui dort consomme quelques
    // millisecondes de processeur pour cinq secondes au mur ; une attente qui
    // boucle en consomme cinq mille.
    titre("CAS 3 : personne ne repond, cout processeur de l'attente");
    {
        int fd = ouvre_udp();
        lie(fd, 40004);
        envoie_requete(fd, 0x4444, "example.net", 10, 0, 2, 99);

        u64 mur_avant = maintenant_ms();
        u64 cpu_avant = processeur_ms();
        u64 duree = 0; u16 ident = 0;
        i64 recu = attends_reponse(fd, 1500, &duree, &ident);
        u64 mur = maintenant_ms() - mur_avant;
        u64 cpu = processeur_ms() - cpu_avant;

        ecris("[dns-probe] CAS3 recu="); ecris_i64(recu);
        ecris(" mur_ms="); ecris_u64(mur);
        ecris(" cpu_ms="); ecris_u64(cpu); ecris("\n");

        // Le seuil est large : un quart du temps au mur. Une attente qui dort
        // reste tres en dessous, une attente qui boucle est au plafond. Entre
        // les deux il n'y a rien a discuter.
        if (mur >= 1000 && cpu * 4 < mur)
            ecris("[dns-probe] CAS3_ATTENTE_SANS_CPU\n");
        else
            ecris("[dns-probe] CAS3_ECHEC l attente consomme le processeur\n");
    }

    // ---- Cas 4 : le chemin de LibDNS, par poll() -----------------------
    //
    // Les trois cas precedents lisent en bloquant. **Ce n'est pas ce que fait
    // LibDNS** : il pose un `Core::Notifier` sur son socket et attend que la
    // boucle d'evenements le declare lisible — c'est-a-dire qu'il depend de
    // `poll()`, pas de `recvfrom`. Un `poll` qui ne signale jamais la
    // lisibilite laisserait le resolveur attendre une reponse pourtant deja
    // rangee dans le socket, et le journal de Ladybird ressemblerait
    // exactement a ce qu'il montre : plus de boucle a plein processeur, mais
    // toujours aucune navigation commitee.
    //
    // Ce cas ferme ou ouvre cette hypothese, du cote du noyau.
    titre("CAS 4 : reveil par poll(), le chemin du resolveur");
    {
        int fd = ouvre_udp();
        lie(fd, 40005);
        envoie_requete(fd, 0x5555, "example.com", s1, s2, s3, s4);

        struct { int fd; short events; short revents; } pfd;
        pfd.fd = fd;
        pfd.events = POLLIN;
        pfd.revents = 0;

        u64 depart = maintenant_ms();
        i64 pret = sys3(SYS_poll, (i64)&pfd, 1, 3000);
        u64 attente = maintenant_ms() - depart;

        ecris("[dns-probe] CAS4 poll rend="); ecris_i64(pret);
        ecris(" revents="); ecris_i64(pfd.revents);
        ecris(" attente_ms="); ecris_u64(attente); ecris("\n");

        i64 recu = -1;
        u16 ident = 0;
        if (pret > 0 && (pfd.revents & POLLIN)) {
            u8 tampon[2048];
            recu = sys6(SYS_recvfrom, fd, (i64)tampon, sizeof(tampon), MSG_DONTWAIT, 0, 0);
            if (recu > 0) ident = (u16)(((u16)tampon[0] << 8) | tampon[1]);
        }
        ecris("[dns-probe] CAS4 recu="); ecris_i64(recu);
        ecris(" id=0x"); ecris_hex16(ident); ecris("\n");

        if (pret > 0 && recu > 0 && ident == 0x5555)
            ecris("[dns-probe] CAS4_OK\n");
        else
            ecris("[dns-probe] CAS4_ECHEC poll ne signale pas la reponse\n");
    }

    // ---- Cas 5 : la sequence exacte de LibCore -------------------------
    //
    // Les quatre cas precedents mesurent des primitives isolees. LibDNS, lui,
    // ne les appelle pas telles quelles : il passe par `Core::BufferedUDPSocket`
    // et par `Core::UDPSocket`, dont le chemin de lecture enchaine trois choses
    // dans un ordre precis.
    //
    // D'abord `UDPSocket::connect` pose un pair par `connect()` — la prise n'est
    // donc plus seulement liee, elle est **connectee**, et c'est un autre chemin
    // de routage dans le noyau.
    //
    // Ensuite `process_incoming_messages` interroge deux fois la lisibilite : la
    // boucle d'evenements reveille le notifier, puis la fonction elle-meme
    // redemande `can_read_without_blocking()` avant de lire, et **sort si cette
    // seconde interrogation dit non**. Un noyau qui consommerait la lisibilite
    // au premier `poll` laisserait donc LibDNS repartir sans jamais lire, avec
    // le datagramme toujours en attente — exactement le silence observe.
    //
    // Enfin `UDPSocket::read_some` appelle `ioctl(FIONREAD)` avant chaque
    // lecture, et refuse de lire si la valeur rendue depasse le tampon.
    //
    // Ce cas rejoue cette sequence telle quelle, avec les memes appels dans le
    // meme ordre.
    titre("CAS 5 : la sequence de LibCore, connect + double poll + FIONREAD");
    {
        int fd = (int)sys3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
        if (fd < 0) { ecris("[dns-probe] CAS5_ECHEC socket\n"); goto fin_cas5; }

        struct sockaddr_in serveur;
        remplis_adresse(&serveur, s1, s2, s3, s4, 53);
        i64 lie_ok = sys3(SYS_connect, fd, (i64)&serveur, sizeof(serveur));
        ecris("[dns-probe] CAS5 connect="); ecris_i64(lie_ok); ecris("\n");
        if (lie_ok < 0) { ecris("[dns-probe] CAS5_ECHEC connect\n"); goto fin_cas5; }

        // Sur une prise connectee, LibCore emet sans adresse de destination.
        u8 requete[512];
        int taille = fabrique_requete(requete, 0x7777, "example.com");
        i64 envoye = sys6(SYS_sendto, fd, (i64)requete, taille, 0, 0, 0);
        ecris("[dns-probe] CAS5 envoye="); ecris_i64(envoye); ecris("\n");
        if (envoye < 0) { ecris("[dns-probe] CAS5_ECHEC sendto\n"); goto fin_cas5; }

        struct { int fd; short events; short revents; } pfd;
        pfd.fd = fd; pfd.events = POLLIN; pfd.revents = 0;

        // Premiere interrogation : celle de la boucle d'evenements, avec un
        // vrai delai. C'est elle qui reveille le notifier.
        i64 premier = sys3(SYS_poll, (i64)&pfd, 1, 3000);

        // Seconde interrogation : celle de `process_incoming_messages`, avec un
        // delai **nul**. C'est le point de sortie silencieux.
        pfd.revents = 0;
        i64 second = sys3(SYS_poll, (i64)&pfd, 1, 0);

        ecris("[dns-probe] CAS5 poll1="); ecris_i64(premier);
        ecris(" poll2="); ecris_i64(second);
        ecris(" revents="); ecris_i64(pfd.revents); ecris("\n");

        // `FIONREAD` sur une prise a datagrammes : la taille du **prochain**
        // datagramme, jamais zero quand il y en a un.
        int en_attente = -1;
        i64 code_ioctl = sys3(SYS_ioctl, fd, FIONREAD, (i64)&en_attente);
        ecris("[dns-probe] CAS5 ioctl="); ecris_i64(code_ioctl);
        ecris(" FIONREAD="); ecris_i64(en_attente); ecris("\n");

        u8 tampon[2048];
        i64 recu = sys6(SYS_recvfrom, fd, (i64)tampon, sizeof(tampon), MSG_DONTWAIT, 0, 0);
        u16 ident = 0;
        if (recu > 0) ident = (u16)(((u16)tampon[0] << 8) | tampon[1]);
        ecris("[dns-probe] CAS5 recu="); ecris_i64(recu);
        ecris(" id=0x"); ecris_hex16(ident); ecris("\n");

        // Le verdict porte sur les trois maillons a la fois : la seconde
        // interrogation doit encore signaler la lisibilite, `FIONREAD` doit
        // annoncer la taille reelle, et la lecture doit rendre ce nombre
        // d'octets.
        if (second > 0 && (pfd.revents & POLLIN) && en_attente == (int)recu
            && recu > 0 && ident == 0x7777)
            ecris("[dns-probe] CAS5_OK\n");
        else if (second <= 0)
            ecris("[dns-probe] CAS5_ECHEC la seconde interrogation perd la lisibilite\n");
        else if (en_attente != (int)recu)
            ecris("[dns-probe] CAS5_ECHEC FIONREAD ne decrit pas le datagramme\n");
        else
            ecris("[dns-probe] CAS5_ECHEC la lecture ne rend pas la reponse\n");
    }
fin_cas5:

    ecris("[dns-probe] FIN\n");
    sys1(SYS_exit, 0);
}

__attribute__((naked)) void _start(void) {
    __asm__ volatile(
        "xorq %rbp, %rbp\n\t"
        "andq $-16, %rsp\n\t"
        "movabsq $principal, %rax\n\t"
        "callq *%rax\n\t"
        "ud2\n\t");
}
