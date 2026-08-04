// Test du "serveur graphique" : /dev/fb0 + ioctl + mmap, comme le fait le
// plugin linuxfb de Qt.
typedef unsigned long u64; typedef long i64; typedef unsigned int u32;
static inline i64 sc(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f){
  i64 r; register i64 r10 __asm__("r10")=d; register i64 r8 __asm__("r8")=e; register i64 r9 __asm__("r9")=f;
  __asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(r10),"r"(r8),"r"(r9):"rcx","r11","memory");
  return r;
}
static void wr(const char*s){ int n=0; while(s[n]) n++; sc(1,1,(i64)s,n,0,0,0); }
static void num(i64 v){ char b[24]; int i=23; b[i--]=0; if(!v) b[i--]='0'; int neg=v<0; if(neg)v=-v;
  while(v){ b[i--]='0'+(v%10); v/=10; } if(neg) b[i--]='-'; wr(&b[i+1]); }

void _start(void){
  i64 fd = sc(2,(i64)"/dev/fb0",2,0,0,0,0); // open O_RDWR
  wr("[fb] open /dev/fb0 -> fd="); num(fd); wr("\n");
  if (fd < 0) { sc(231,1,0,0,0,0,0); }

  u32 var[40]; u64 fix[10];
  wr("[fb] FBIOGET_VSCREENINFO="); num(sc(16,fd,0x4600,(i64)var,0,0,0)); wr("\n");
  wr("[fb] resolution "); num(var[0]); wr("x"); num(var[1]);
  wr(" bpp="); num(var[6]); wr("\n");
  wr("[fb] FBIOGET_FSCREENINFO="); num(sc(16,fd,0x4602,(i64)fix,0,0,0)); wr("\n");
  u32* f32 = (u32*)fix;
  wr("[fb] smem_start="); num((i64)fix[2]); wr(" line_length="); num(f32[12]); wr("\n");

  u64 len = (u64)var[0]*var[1]*4;
  i64 p = sc(9,0,len,3,1 /*MAP_SHARED*/,fd,0);
  wr("[fb] mmap -> "); num(p); wr("\n");
  if (p < 0) { sc(231,2,0,0,0,0,0); }

  // Degrade + un rectangle : preuve visuelle que le ring 3 ecrit dans la VRAM.
  volatile u32* px = (volatile u32*)p;
  for (u32 y = 0; y < var[1]; y++)
    for (u32 x = 0; x < var[0]; x++)
      px[y*var[0]+x] = ((x*255/var[0])<<16) | ((y*255/var[1])<<8) | 0x40;
  for (u32 y = 200; y < 400; y++)
    for (u32 x = 300; x < 900; x++)
      px[y*var[0]+x] = 0x00FFFFFF;

  wr("[fb] framebuffer dessine depuis le ring 3\n");
  // Boucle d'affichage : la capture d'ecran doit voir le mode graphique actif.
  for(;;) { u64 ts[2] = {1,0}; sc(35,(i64)ts,0,0,0,0,0); }
}
