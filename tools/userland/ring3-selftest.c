// Test freestanding : ring3 + syscalls + clone/futex + TLS (fs base)
typedef unsigned long u64; typedef long i64;
static inline i64 sc(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f){
  i64 r; register i64 r10 __asm__("r10")=d; register i64 r8 __asm__("r8")=e; register i64 r9 __asm__("r9")=f;
  __asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(r10),"r"(r8),"r"(r9):"rcx","r11","memory");
  return r;
}
static void wr(const char*s){ int n=0; while(s[n]) n++; sc(1,1,(i64)s,n,0,0,0); }
static void num(i64 v){ char b[24]; int i=23; b[i--]=0; if(!v) b[i--]='0'; int neg = v<0; if(neg) v=-v;
  while(v){ b[i--]='0'+(v%10); v/=10; } if(neg) b[i--]='-'; wr(&b[i+1]); }

static volatile int futex_word = 0;
static volatile int child_ran = 0;
static char child_stack[65536] __attribute__((aligned(16)));

static int child_fn(void){
  child_ran = 1;
  wr("[child] thread demarre, tid=");
  num(sc(186,0,0,0,0,0,0)); wr("\n");
  futex_word = 1;
  sc(202,(i64)&futex_word,1 /*FUTEX_WAKE*/,1,0,0,0);
  sc(60,0,0,0,0,0,0); // exit
  return 0;
}

void _start(void){
  wr("[main] uname: ");
  char uts[390]; sc(63,(i64)uts,0,0,0,0,0); wr(uts); wr(" "); wr(uts+65*2); wr("\n");

  wr("[main] getpid="); num(sc(39,0,0,0,0,0,0));
  wr(" gettid="); num(sc(186,0,0,0,0,0,0)); wr("\n");

  // brk
  i64 b0 = sc(12,0,0,0,0,0,0);
  i64 b1 = sc(12,b0+0x10000,0,0,0,0,0);
  wr("[main] brk avant="); num(b0); wr(" apres="); num(b1);
  wr(b1 >= b0+0x10000 ? " OK\n" : " ECHEC\n");
  *(volatile char*)(b0+0x8000) = 42;
  wr("[main] ecriture dans le tas brk OK\n");

  // mmap + mprotect
  i64 m = sc(9,0,0x4000,3,0x22,-1,0);
  *(volatile int*)m = 0x1234;
  wr("[main] mmap="); num(m); wr(" valeur="); num(*(volatile int*)m); wr("\n");
  wr("[main] mprotect="); num(sc(10,m,0x4000,1,0,0,0)); wr("\n");

  // TLS via arch_prctl(ARCH_SET_FS)
  static u64 tls_area[16];
  tls_area[0] = (u64)tls_area;
  sc(158,0x1002,(i64)tls_area,0,0,0,0);
  u64 self; __asm__ volatile("mov %%fs:0, %0":"=r"(self));
  wr(self == (u64)tls_area ? "[main] TLS (fs base) OK\n" : "[main] TLS ECHEC\n");

  // clock_gettime monotone
  u64 ts[2]; sc(228,1,(i64)ts,0,0,0,0);
  wr("[main] clock_gettime monotone s="); num(ts[0]); wr("\n");

  // clone -> thread
  futex_word = 0;
  i64 tid = sc(56, 0x100|0x10000|0x200|0x400|0x800,
               (i64)(child_stack+sizeof(child_stack)-16), 0, 0, 0, 0);
  if (tid == 0) { child_fn(); }
  wr("[main] clone -> tid="); num(tid); wr("\n");

  // futex wait jusqu'au reveil par l'enfant
  int spins = 0;
  while (futex_word == 0 && spins < 200) { sc(202,(i64)&futex_word,0,0,0,0,0); spins++; }
  wr(futex_word ? "[main] futex reveille par le thread OK\n" : "[main] futex ECHEC\n");
  wr(child_ran ? "[main] le thread enfant a bien tourne\n" : "[main] enfant jamais execute\n");

  // fichiers
  i64 fd = sc(257,-100,(i64)"/tmp-ring3.txt",0100|02|01000,0644,0,0);
  wr("[main] openat fd="); num(fd); wr("\n");
  if (fd >= 0) {
    sc(1,fd,(i64)"bonjour depuis le ring 3\n",25,0,0,0);
    sc(8,fd,0,0,0,0,0); // lseek SEEK_SET 0
    char buf[64]; i64 n = sc(0,fd,(i64)buf,63,0,0,0);
    buf[n>0?n:0]=0;
    wr("[main] relecture ("); num(n); wr(" o): "); wr(buf);
    sc(3,fd,0,0,0,0,0);
  }
  // writev
  struct { const void*b; u64 l; } iov[2] = {{"[main] writev ",14},{"OK\n",3}};
  sc(20,1,(i64)iov,2,0,0,0);

  wr("[main] termine\n");
  sc(231,0,0,0,0,0,0);
  for(;;);
}
