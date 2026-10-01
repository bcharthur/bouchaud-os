import re,sys
from pathlib import Path
PFX=re.compile(r'\[\d\d:\d\d:\d\d\]\[[^\]]*\]\[FPS: [^\]]*\]')
for camp in sys.argv[1:]:
    tot=frag=double=imb=0; boots=0
    for f in sorted(Path(camp).glob('smp*-*.log')):
        boots+=1
        t=f.read_bytes().decode('latin1')
        # ne garder que la fenetre du banc (DEBUT..FIN) : le reste est le demarrage
        i=t.find('sec=DEBUT'); j=t.rfind('sec=FIN')
        if i<0 or j<0: continue
        lignes=t[i:j].splitlines()[1:-1]
        for l in lignes:
            l=re.sub(r'\x1b\[[0-9;]*m','',l)
            if not l.strip(): continue
            tot+=1
            n=len(PFX.findall(l))
            if n>=2: double+=1
            elif n==0 and not l.startswith('['): frag+=1
        m=re.findall(r'serie_imbriquees=(\d+)',t)
        if m: imb+=int(m[-1])
    print(f"{Path(camp).name:8} boots={boots} lignes={tot} deux_prefixes={double} fragments_sans_prefixe={frag} serie_imbriquees(cumul fin)={imb}")
