/*
 * FAUX-NAVIGATEUR : plusieurs processus portant un nom de service Ladybird.
 *
 * # Pourquoi il existe
 *
 * L'arbre par PID de la fenetre Services ne se verifie que s'il existe
 * PLUSIEURS processus vivants portant le meme nom de service. Sous QEMU
 * l'artefact Ladybird n'est pas la, et l'attendre coute deux heures et demie
 * de construction pour eprouver quelques dizaines de lignes de publication.
 *
 * Ce programme fournit la seule chose qui compte ici : des processus
 * distincts, vivants en meme temps, qui portent le nom attendu et consomment
 * reellement du temps et de la memoire.
 *
 * # Deux details sans lesquels il ne prouverait rien
 *
 * LE PERE RESTE VIVANT. Quand le programme de premier plan se termine, le
 * noyau arrete toute sa session -- c'est la semantique POSIX d'un shell, elle
 * est voulue et documentee dans `exit_current`. Un pere qui se retirerait
 * emporterait donc ses enfants, et le releve ne trouverait que des zombies.
 * Ce comportement a ete observe avant d'etre compris : trois enfants crees,
 * trois enfants zombies, et rien dans l'arbre.
 *
 * LES ENFANTS TOUCHENT LEUR MEMOIRE, ils ne se contentent pas de la
 * reserver. Une reservation ne coute aucune page residente, et l'arbre
 * afficherait un RSS nul la ou il doit montrer un processus qui occupe la
 * machine. Chacun en prend une quantite DIFFERENTE : c'est ce qui permet de
 * verifier que les lignes ne sont pas toutes alimentees par le meme
 * processus.
 *
 *   cc -O1 -static-pie -fPIE -nostdlib -nostartfiles -o WebContent faux-navigateur.c
 */
static long appel6(long n,long a,long b,long c,long d,long e,long f){long r;register long r10 __asm__("r10")=d;register long r8 __asm__("r8")=e;register long r9 __asm__("r9")=f;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(r10),"r"(r8),"r"(r9):"rcx","r11","memory");return r;}
static long appel(long n,long a,long b,long c){return appel6(n,a,b,c,0,0,0);}
static void dis(const char*s){long n=0;while(s[n])n++;appel(1,1,(long)s,n);}
struct ts { long sec; long nsec; };
#define PAGE 4096
void _start(void){
  dis("VIT debut\n");
  for (int i=0;i<3;i++){
    long r = appel6(56, 17, 0, 0, 0, 0, 0);
    if (r != 0) continue;
    long pages = 256L*(i+1);
    long a = appel6(9,0,pages*PAGE,0x3,0x22,-1,0);
    if (a>0||a<-4096){ char*z=(char*)a; for(long p=0;p<pages;p++) z[p*PAGE]=(char)p; }
    dis("VIT enfant au travail\n");
    /* Du VRAI travail, pour que le releve voie du CPU : sinon la colonne
       serait a zero et ne distinguerait pas les instances entre elles. */
    volatile long s=0; for(long t=0;t<60000000L*(i+1);t++) s+=t;
    /* Puis rester VIVANT : le releve a lieu toutes les cinq secondes, et un
       processus deja mort ne montre que le chemin de disparition. */
    struct ts d={30,0}; appel(35,(long)&d,0,0);
    appel(60,0,0,0); for(;;){}
  }
  dis("VIT pere attend\n");
  struct ts t={40,0};
  appel(35,(long)&t,0,0);
  dis("VIT pere sort\n");
  appel(60,0,0,0); for(;;){}
}
