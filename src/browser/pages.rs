//! Pages internes de Nautile : about:bouchaud, about:system, about:calc,
//! about:wasm, et page Google locale. Chaque page est du HTML autonome
//! rendu par le moteur web local.

use alloc::format;
use alloc::string::String;

/// Page d'accueil Google locale (souveraine) : la vraie home Google est
/// construite par JavaScript et illisible en rendu statique. On affiche une
/// page Google nette et fonctionnelle ; la recherche se fait depuis la barre
/// d'adresse (qui route vers google.com/search, rendu par notre moteur).
pub fn google_home() -> &'static str {
    r#"<!doctype html><html><head><title>Google</title>
<style>
body{background:#fff;color:#202124;font-family:sans-serif;margin:0;padding:0;text-align:center}
.logo{margin:64px 0 22px;font-size:68px;letter-spacing:-3px;font-weight:bold}
.logo .b{color:#4285f4}.logo .r{color:#ea4335}.logo .y{color:#fbbc05}.logo .g{color:#34a853}
.bar{background:#fff;border:1px solid #dfe1e5;border-radius:24px;padding:11px 18px;
     font-size:14px;width:480px;max-width:90%;margin:0 auto;color:#9aa0a6;
     box-shadow:0 1px 6px rgba(32,33,36,.12);text-align:left}
.cta{margin:22px 0}
.btn{background:#f8f9fa;border:1px solid #f8f9fa;border-radius:4px;
     color:#3c4043;padding:9px 16px;font-size:13px;margin:4px;text-decoration:none;display:inline-block}
.tip{color:#5f6368;font-size:13px;margin:18px auto;max-width:520px;line-height:1.7}
.tip b{color:#1a73e8}
.quick{margin-top:14px}
.quick a{color:#1a73e8;font-size:13px;margin:0 10px;text-decoration:none}
.foot{color:#9aa0a6;font-size:12px;margin-top:40px}
</style>
</head><body>
<div class="logo">
  <span class="b">G</span><span class="r">o</span><span class="y">o</span><span class="b">g</span><span class="g">l</span><span class="r">e</span>
</div>
<div class="bar">&#x1f50d;&nbsp;&nbsp;Rechercher sur Google</div>
<div class="tip">Tapez votre recherche dans la <b>barre d'adresse</b> tout en haut,
   puis appuyez sur <b>Entree</b> &mdash; Nautile l'enverra a Google et affichera
   les resultats.</div>
<div class="quick">
  <a href="https://www.google.com/search?q=actualites">Actualites</a>
  <a href="https://www.google.com/search?q=meteo">Meteo</a>
  <a href="https://www.wikipedia.org/">Wikipedia</a>
  <a href="https://example.com/">example.com</a>
</div>
<div class="foot">Nautile &mdash; moteur de rendu souverain Bouchaud OS &mdash;
   <a href="about:bouchaud">Accueil</a></div>
</body></html>"#
}

pub fn bouchaud_home() -> String {
    format!(
        "<!doctype html><html><head><title>Nautile Navigateur</title><style>\
         *{{box-sizing:border-box;margin:0;padding:0}}\
         body{{background:#0d1b2a;color:#e0e8f4;font-family:sans-serif}}\
         .hero{{background:linear-gradient(160deg,#0a2540 0%,#163870 60%,#0d4f7a 100%);\
                padding:28px 20px 20px;text-align:center;border-bottom:2px solid #1e5f9a}}\
         .logo{{display:inline-flex;align-items:center;gap:12px;margin-bottom:10px}}\
         .shell{{font-size:36px;line-height:1}}\
         .brand{{font-size:26px;font-weight:bold;letter-spacing:2px;\
                 background:linear-gradient(90deg,#f5c040,#ffa020);-webkit-background-clip:text;\
                 color:#f5c040}}\
         .ver{{color:#7ba8d4;font-size:12px;margin-top:6px}}\
         .tagline{{color:#a8c4e0;font-size:13px;margin-top:4px}}\
         .grid{{display:grid;grid-template-columns:1fr 1fr;gap:10px;padding:14px}}\
         .card{{background:#0f2035;border:1px solid #1e4060;border-radius:8px;\
                padding:12px 14px;box-shadow:0 2px 6px rgba(0,0,0,.4)}}\
         .card.full{{grid-column:1/-1}}\
         .card h3{{color:#f5c040;font-size:13px;margin-bottom:8px;\
                   border-bottom:1px solid #1e4060;padding-bottom:5px}}\
         .card p,.card li{{font-size:12px;color:#b8cfe0;line-height:1.6;margin:3px 0}}\
         .card ul{{padding-left:14px}}\
         a{{color:#4fa8f0;text-decoration:none}}\
         a:hover{{color:#7fc0ff;text-decoration:underline}}\
         .badge{{display:inline-block;border-radius:3px;padding:1px 5px;\
                 font-size:10px;margin-left:4px;font-weight:bold}}\
         .https{{background:#0d3d20;color:#34c26a}}\
         .info{{background:#0a2f50;color:#4fa8f0}}\
         .kb{{display:inline-block;background:#1a3a5c;border:1px solid #2a5a8c;\
              border-radius:3px;padding:1px 5px;font-size:11px;font-family:monospace}}\
         </style></head><body>\
         <div class=\"hero\">\
           <div class=\"logo\">\
             <span class=\"shell\">&#x1f41a;</span>\
             <span class=\"brand\">Nautile Navigateur</span>\
           </div>\
           <div class=\"ver\">Bouchaud OS v{os_ver} &mdash; Nautile {nautile_merge} ({nautile_date})</div>\
           <div class=\"ver\">Source {nautile_source} &mdash; {nautile_ref}</div>\
           <div class=\"tagline\">Navigation locale &bull; TLS 1.3 integre &bull; HTML5 &bull; JS &bull; WebAssembly</div>\
         </div>\
         <div class=\"grid\">\
           <div class=\"card\">\
             <h3>&#x1f517; Navigation rapide</h3>\
             <ul>\
               <li><a href=\"https://example.com/\">example.com</a>\
                   <span class=\"badge https\">HTTPS</span></li>\
               <li><a href=\"https://www.wikipedia.org/\">wikipedia.org</a>\
                   <span class=\"badge https\">HTTPS</span></li>\
               <li><a href=\"about:calc\">Calculatrice</a>\
                   <span class=\"badge info\">JS natif</span></li>\
               <li><a href=\"about:wasm\">Demo WebAssembly</a>\
                   <span class=\"badge info\">wasm</span></li>\
               <li><a href=\"about:modern\">Demo rendu moderne</a>\
                   <span class=\"badge info\">flex/grid</span></li>\
               <li><a href=\"about:system\">Informations systeme</a></li>\
               <li><a href=\"file:/readme.txt\">readme.txt</a>\
                   <span class=\"badge info\">RAMFS</span></li>\
             </ul>\
           </div>\
           <div class=\"card\">\
             <h3>&#x2328; Raccourcis</h3>\
             <ul>\
               <li><span class=\"kb\">Entree</span> naviguer vers l'URL</li>\
               <li><span class=\"kb\">Alt+Gauche</span> reculer</li>\
               <li><span class=\"kb\">Alt+Droite</span> avancer</li>\
               <li><span class=\"kb\">F5</span> recharger</li>\
               <li><span class=\"kb\">Ctrl+T</span> nouvel onglet</li>\
               <li><span class=\"kb\">1..9</span> suivre lien n</li>\
               <li><span class=\"kb\">Molette</span> defiler</li>\
             </ul>\
           </div>\
           <div class=\"card full\">\
             <h3>&#x1f4e1; Moteur Nautile Navigateur</h3>\
             <p>Navigateur web integre nativement dans Bouchaud OS &mdash; <b>zero dependance externe</b>.\
                Tout s'execute dans le noyau Rust <code>no_std</code> :</p>\
             <ul>\
               <li>Pile reseau complete : DNS, DHCP, TLS 1.3, HTTP/1.1, HTTP/2</li>\
               <li>Parseur HTML5 tolerant, DOM complet, CSS cascade (tag / .classe / #id)</li>\
               <li>Flexbox, marges, paddings, text-align, font-weight, images PNG</li>\
               <li>Interpreteur JavaScript avec DOM, timers, fetch et WebAssembly (wasmi)</li>\
               <li>Onglets multiples, historique de navigation, barre d'adresse URL</li>\
             </ul>\
           </div>\
         </div>\
         </body></html>",
        os_ver = crate::VERSION,
        nautile_merge = crate::browser::NAUTILE_MERGE_SHORT,
        nautile_date = crate::browser::NAUTILE_MERGE_DATE,
        nautile_source = crate::browser::NAUTILE_SOURCE_SHORT,
        nautile_ref = crate::browser::NAUTILE_MERGE_SUBJECT,
    )
}

/// Page de demonstration du moteur de rendu moderne : exerce flexbox
/// (row/column, justify-content, align-items, gap), CSS grid (colonnes +
/// cellules pleine largeur) et le box model par cote (padding/margin).
/// Sert de banc d'essai visuel directement dans l'interface (about:modern).
pub fn modern_demo() -> &'static str {
    r#"<!doctype html><html><head><title>Nautile — Rendu moderne</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{background:#0e1726;color:#e6edf6;font-family:sans-serif}
.nav{display:flex;justify-content:space-between;align-items:center;
     background:#10243f;padding:12px 18px;border-bottom:2px solid #1e63b0}
.nav .brand{font-size:18px;font-weight:bold;color:#f5c040}
.nav .menu{display:flex;gap:16px}
.nav .menu a{color:#9fc2e8;font-size:13px}
.hero{display:flex;flex-direction:column;align-items:center;
      padding:26px 16px;background:linear-gradient(160deg,#13294a,#0b3a5c)}
.hero h1{font-size:26px;color:#ffffff;margin-bottom:6px}
.hero p{color:#a8c6e4;font-size:13px}
.section{padding:16px}
.section h2{font-size:15px;color:#f5c040;margin-bottom:10px}
.cards{display:grid;grid-template-columns:repeat(3,1fr);gap:12px}
.card{background:#13233c;border:1px solid #25456f;
      border-radius:8px;padding:14px 16px}
.card h3{color:#7fb4ee;font-size:14px;margin-bottom:6px}
.card p{color:#b6cae0;font-size:12px;line-height:1.6}
.full{grid-column:1/-1;background:#0f2d22;border-color:#2f7a4f}
.full h3{color:#5fd08a}
.row{display:flex;gap:12px;margin-top:14px}
.pill{flex:1;background:#1a2c49;border-radius:6px;padding:16px;text-align:center}
.pill .n{font-size:22px;font-weight:bold;color:#f5c040}
.pill .l{font-size:11px;color:#9fc2e8}
.sidebar-layout{display:flex;gap:14px;margin-top:14px}
.sidebar{width:140px;background:#15233a;border-radius:6px;padding:12px}
.sidebar a{display:block;color:#9fc2e8;font-size:12px;padding:4px 0}
.content{flex:1;background:#13233c;border-radius:6px;padding:14px}
.content p{color:#b6cae0;font-size:12px;line-height:1.6}
.posrow{display:flex;gap:14px;margin-top:14px}
.relcard{position:relative;flex:1;background:#13233c;padding:16px;border-radius:8px;
         box-shadow:0 4px 10px #04101f}
.relcard h3{color:#7fb4ee;font-size:14px}
.relcard p{color:#b6cae0;font-size:12px;line-height:1.7}
.badge-abs{position:absolute;top:8px;right:8px;background:#e0483a;color:#fff;
           font-size:10px;font-weight:bold;padding:3px 7px;border-radius:10px}
.clipbox{flex:1;height:90px;overflow:hidden;background:#0f2d22;border:1px solid #2f7a4f;
         border-radius:8px;padding:12px}
.clipbox h3{color:#5fd08a;font-size:13px}
.clipbox p{color:#b6cae0;font-size:12px;line-height:1.7}
.fixednote{position:fixed;bottom:10px;right:10px;background:#1a73e8;color:#fff;
           font-size:11px;padding:6px 10px;border-radius:6px;box-shadow:0 2px 6px #02060d}
</style></head><body>
<div class="fixednote">position: fixed &#x2693;</div>
<div class="nav">
  <div class="brand">&#x1f41a; Nautile</div>
  <div class="menu">
    <a href="about:bouchaud">Accueil</a>
    <a href="about:system">Systeme</a>
    <a href="about:wasm">WASM</a>
  </div>
</div>
<div class="hero">
  <h1>Rendu de pages modernes</h1>
  <p>Flexbox &bull; CSS Grid &bull; box model par cote &mdash; moteur souverain Rust</p>
</div>
<div class="section">
  <h2>CSS Grid &mdash; repeat(3, 1fr) + gap</h2>
  <div class="cards">
    <div class="card"><h3>Flexbox</h3><p>flex-direction, justify-content, align-items et gap sont interpretes nativement.</p></div>
    <div class="card"><h3>Grid</h3><p>grid-template-columns avec repeat() et retour a la ligne automatique des cellules.</p></div>
    <div class="card"><h3>Box model</h3><p>padding et margin par cote (1 a 4 valeurs), bordures et rayons.</p></div>
    <div class="card"><h3>Couleurs</h3><p>hex, rgb(), hsl(), degrades (premiere couleur) et 140+ noms CSS.</p></div>
    <div class="card"><h3>Typographie</h3><p>police vectorielle TrueType antialiasee, tailles em/px/pt.</p></div>
    <div class="full"><h3>Cellule pleine largeur (grid-column: 1 / -1)</h3><p>Une cellule peut couvrir toutes les colonnes de la grille, comme dans un tableau de bord moderne.</p></div>
  </div>

  <h2 style="margin-top:18px">Flexbox &mdash; justify + flex:1</h2>
  <div class="row">
    <div class="pill"><div class="n">100%</div><div class="l">Souverain</div></div>
    <div class="pill"><div class="n">0</div><div class="l">Dependance externe</div></div>
    <div class="pill"><div class="n">TLS</div><div class="l">1.3 integre</div></div>
  </div>

  <h2 style="margin-top:18px">Layout sidebar (flex row, largeur fixe + flex:1)</h2>
  <div class="sidebar-layout">
    <div class="sidebar">
      <a href="about:bouchaud">&#x1f3e0; Accueil</a>
      <a href="about:calc">&#x1f9ee; Calculatrice</a>
      <a href="about:wasm">&#x26a1; WebAssembly</a>
      <a href="https://example.com/">&#x1f310; example.com</a>
    </div>
    <div class="content">
      <p>La barre laterale a une largeur fixe (140px) et la zone de contenu occupe
         l'espace restant via <b>flex:1</b>. C'est le patron de mise en page le
         plus courant des applications web modernes, desormais rendu nativement
         par Nautile.</p>
    </div>
  </div>

  <h2 style="margin-top:18px">Positionnement &mdash; relative/absolute, overflow, ombre</h2>
  <div class="posrow">
    <div class="relcard">
      <div class="badge-abs">NOUVEAU</div>
      <h3>position: relative + absolute</h3>
      <p>Le badge rouge est en <b>position:absolute; top:8px; right:8px</b>, ancre
         au coin de cette carte <b>position:relative</b>. L'ombre portee vient de
         <b>box-shadow</b>.</p>
    </div>
    <div class="clipbox">
      <h3>overflow: hidden</h3>
      <p>Ce conteneur a une hauteur fixe de 90px et <b>overflow:hidden</b> : le
         texte qui depasse est decoupe au bord de la boite au lieu de deborder.
         Ligne supplementaire un. Ligne supplementaire deux. Ligne supplementaire
         trois. Ligne supplementaire quatre. Ligne supplementaire cinq, qui ne
         doit pas etre visible si le clipping fonctionne correctement.</p>
    </div>
  </div>
  <p style="color:#5f7da0;font-size:11px;margin-top:10px">La pastille bleue en bas
     a droite reste fixe a l'ecran (position:fixed) meme en defilant.</p>
</div>
</body></html>"#
}

pub fn system_info() -> String {
    let dt   = crate::arch::x86_64::rtc::now();
    let (used, free, total) = crate::kernel::heap::stats();
    let uptime = crate::kernel::timer::seconds();
    let pci_n  = crate::arch::x86_64::pci::count();
    let user   = crate::users::session().username();
    format!(
        "<!doctype html><html><head><title>Informations système</title><style>\
         body{{background:#f8f9fa;color:#202124;font-family:sans-serif;padding:12px}}\
         h1{{color:#1a73e8;font-size:18px}}\
         table{{border-collapse:collapse;width:100%;font-size:12px}}\
         td{{padding:5px 8px;border-bottom:1px solid #dadce0}}\
         td:first-child{{color:#5f6368;width:40%}}\
         td:last-child{{font-weight:bold}}\
         </style></head><body>\
         <h1>Systeme Bouchaud OS {ver}</h1>\
         <table>\
           <tr><td>Heure RTC</td><td>{h:02}:{m:02}:{s:02}</td></tr>\
           <tr><td>Uptime</td><td>{up} secondes</td></tr>\
           <tr><td>Heap utilisée</td><td>{used} / {total} octets</td></tr>\
           <tr><td>Heap libre</td><td>{free} octets</td></tr>\
           <tr><td>Périphériques PCI</td><td>{pci}</td></tr>\
           <tr><td>Utilisateur</td><td>{user}</td></tr>\
           <tr><td>Nautile merge</td><td>{nautile_merge} ({nautile_date})</td></tr>\
           <tr><td>Nautile source</td><td>{nautile_source}</td></tr>\
           <tr><td>Nautile ref</td><td>{nautile_ref}</td></tr>\
         </table>\
         <p><a href=\"about:bouchaud\">← Accueil</a></p>\
         </body></html>",
        ver = crate::VERSION,
        h = dt.hour, m = dt.minute, s = dt.second,
        up = uptime, used = used, total = total, free = free,
        pci = pci_n, user = user,
        nautile_merge = crate::browser::NAUTILE_MERGE_SHORT,
        nautile_date = crate::browser::NAUTILE_MERGE_DATE,
        nautile_source = crate::browser::NAUTILE_SOURCE_SHORT,
        nautile_ref = crate::browser::NAUTILE_MERGE_SUBJECT,
    )
}

// Application calculatrice HTML+CSS+JS — tourne entièrement dans le moteur web.
pub const CALC_APP: &str = r#"<!doctype html><html><head>
<title>Calculatrice</title>
<style>
body{background:#202124;color:#e8eaed;font-family:sans-serif;padding:8px}
h2{color:#8ab4f8;text-align:center;font-size:16px;margin:4px 0 8px}
#disp{background:#111;color:#8ab4f8;font-size:24px;text-align:right;
      padding:8px 10px;border-radius:4px;min-height:36px;margin-bottom:8px;
      border:1px solid #3c4043}
.grid{display:flex;flex-direction:column;gap:4px;max-width:220px;margin:0 auto}
.row{display:flex;gap:4px}
button{background:#3c4043;color:#e8eaed;font-size:18px;border:none;
       border-radius:4px;padding:8px 0;flex:1;cursor:pointer}
button:active{background:#5f6368}
.op button{background:#28292c;color:#fdd663}
.eq button{background:#1a73e8;color:#fff}
.cls button{background:#e53935;color:#fff}
</style></head><body>
<h2>Calculatrice</h2>
<div id="disp">0</div>
<div class="grid">
  <div class="row">
    <div class="cls"><button onclick="clr()">C</button></div>
    <button onclick="press('(')">(</button>
    <button onclick="press(')')">)</button>
    <div class="op"><button onclick="press('/')">÷</button></div>
  </div>
  <div class="row">
    <button onclick="press('7')">7</button>
    <button onclick="press('8')">8</button>
    <button onclick="press('9')">9</button>
    <div class="op"><button onclick="press('*')">×</button></div>
  </div>
  <div class="row">
    <button onclick="press('4')">4</button>
    <button onclick="press('5')">5</button>
    <button onclick="press('6')">6</button>
    <div class="op"><button onclick="press('-')">−</button></div>
  </div>
  <div class="row">
    <button onclick="press('1')">1</button>
    <button onclick="press('2')">2</button>
    <button onclick="press('3')">3</button>
    <div class="op"><button onclick="press('+')">+</button></div>
  </div>
  <div class="row">
    <button onclick="press('0')">0</button>
    <button onclick="press('.')">.</button>
    <button onclick="press('%')">%</button>
    <div class="eq"><button onclick="equals()">=</button></div>
  </div>
</div>
<script>
var cur = '';
function show(){ document.getElementById('disp').textContent = cur || '0'; }
function press(c){ if(cur==='0'&&c>='0'&&c<='9') cur=''; cur += c; show(); }
function clr(){ cur = ''; show(); }
function equals(){
  try { cur = String(Math.round(evalExpr(cur)*1e9)/1e9); } catch(e){ cur='Err'; }
  show();
}
function evalExpr(s){
  var toks=[], num='', i;
  for(i=0;i<s.length;i++){
    var ch=s[i];
    if((ch>='0'&&ch<='9')||ch==='.'){num+=ch;}
    else{if(num!==''){toks.push(parseFloat(num));num='';}toks.push(ch);}
  }
  if(num!=='')toks.push(parseFloat(num));
  var p2=[],j=0;
  while(j<toks.length){
    var t=toks[j];
    if(t==='*'||t==='/'){var a=p2.pop(),b=toks[j+1];p2.push(t==='*'?a*b:a/b);j+=2;}
    else{p2.push(t);j++;}
  }
  var r=typeof p2[0]==='number'?p2[0]:0;
  for(i=1;i<p2.length;i+=2){
    if(p2[i]==='+')r+=p2[i+1];
    else if(p2[i]==='-')r-=p2[i+1];
  }
  return r;
}
</script></body></html>"#;

// Démonstration WebAssembly : module .wasm fourni en bytes, instancié via wasmi.
pub const WASM_DEMO: &str = r#"<!doctype html><html><head>
<title>WebAssembly — Nautile</title>
<style>
body{background:#0d1117;color:#e6edf3;font-family:sans-serif;padding:14px}
h2{color:#58a6ff;font-size:16px}
.card{background:#161b22;border:1px solid #30363d;border-radius:6px;
      padding:12px 14px;margin:8px 0}
code{color:#7ee787;font-size:12px}
#out{font-size:20px;color:#ffa657;font-weight:bold;margin-top:8px}
</style></head><body>
<h2>WebAssembly dans Bouchaud OS</h2>
<div class="card">
  <p>Module <code>add(i32,i32)→i32</code> compilé en WebAssembly,
     instancié par le runtime <code>wasmi</code> (no_std) et appelé depuis JS :</p>
  <div id="out">calcul en cours...</div>
</div>
<script>
var bytes=[0,97,115,109,1,0,0,0,1,7,1,96,2,127,127,1,127,3,2,1,0,
           7,7,1,3,97,100,100,0,0,10,9,1,7,0,32,0,32,1,106,11];
try {
  var r=WebAssembly.instantiate(bytes);
  var a=21,b=21;
  var sum=r.instance.exports.add(a,b);
  document.getElementById('out').textContent=a+' + '+b+' = '+sum+' (WebAssembly)';
} catch(e) {
  document.getElementById('out').textContent='Erreur : '+e;
}
</script></body></html>"#;
