// Charge d'epreuve du chemin FichierPrive FROID.
//
// BOUCHAUD_C51_OU_SONT_LES_HUIT_SECONDES
//
// Le contraste a expliquer vient de Ladybird :
//
//     WebWorker #1  2440 fautes fichier / 8 008 342 us
//     WebWorker #3  2440 fautes fichier /    54 398 us
//
// Le reproduire demandait jusqu'ici un run Ladybird complet -- deux heures de
// construction. Ce programme le reproduit en quelques minutes, parce que la
// seule chose qui compte est sa TAILLE : au-dela de `INLINE_BOOT_FILE_SIZE`
// (quatre mebioctets), `tar.rs` cesse de copier le contenu dans le noeud et
// l'enregistre comme etendue ATA. Ses pages arrivent alors par le meme chemin
// que celles des ELF de Ladybird.
//
// Le tableau est en `.rodata` via `.incbin` : un initialiseur C de cinq
// mebioctets mettrait des minutes a compiler, et un `.bss` ne serait pas
// adosse au fichier -- il faudrait alors des fautes Zero, qui ne sont pas
// l'objet.
__asm__(
    ".section .rodata\n"
    ".globl gros_bloc\n"
    "gros_bloc:\n"
    ".incbin \"gros.bin\"\n"
    ".section .text\n"
);
extern const unsigned char gros_bloc[];

// BOUCHAUD_C70_ATTEINDRE_LE_CHEMIN_D_EVICTION
//
// La taille etait figee a 5 Mio. C'etait assez pour reproduire le contraste
// froid/chaud des fautes fichier, et PAS assez pour atteindre l'eviction du
// cache de pages : `MAX_RECLAIMABLE_PAGES` vaut 16 384 pages, soit 64 Mio.
//
// Consequence mesuree : le banc rendait `CACHE_BALAYAGE appels=0` et j'en ai
// conclu que le balayage de secours etait hors de cause. Le run 35955074619 a
// montre `appels=22773` et 277 SECONDES passees dedans. Le banc ne refutait
// rien -- il n'atteignait simplement jamais ce chemin.
//
// `-DTAILLE_MIO=N` permet desormais de le depasser volontairement.
#ifndef TAILLE_MIO
#define TAILLE_MIO 5
#endif
#define TAILLE ((unsigned long)TAILLE_MIO * 1024 * 1024)
#define PAGE 4096

static long ecris(const char *s, unsigned long n) {
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(1), "D"(1), "S"(s), "d"(n)
                     : "rcx", "r11", "memory");
    return r;
}

static void sortie(int code) {
    __asm__ volatile("syscall" :: "a"(60), "D"((long)code) : "rcx", "r11", "memory");
    __builtin_unreachable();
}

static void nombre(char *out, unsigned long v) {
    char tampon[24];
    int n = 0;
    if (v == 0) tampon[n++] = '0';
    while (v) { tampon[n++] = (char)('0' + v % 10); v /= 10; }
    int i = 0;
    while (n) out[i++] = tampon[--n];
    out[i] = 0;
}

void _start(void) {
    // TOUTES les pages sont touchees, une par page : c'est ce qui produit les
    // fautes FichierPrive, et leur nombre est connu d'avance (1280).
    volatile unsigned long somme = 0;
    for (unsigned long o = 0; o < TAILLE; o += PAGE) {
        somme += gros_bloc[o];
    }

    char ligne[64] = "GROS_ELF_OK pages=";
    char n[24];
    nombre(n, TAILLE / PAGE);
    int i = 18;
    for (int j = 0; n[j]; j++) ligne[i++] = n[j];
    ligne[i++] = '\n';
    ecris(ligne, (unsigned long)i);
    sortie(somme == 0 && TAILLE != 0 ? 0 : 0);
}
