//! Pages internes de Nautile : about:bouchaud, about:system, about:calc,
//! about:wasm, et page Google locale. Chaque page est du HTML autonome
//! rendu par le moteur web local.

use alloc::format;
use alloc::string::String;

/// Page d'accueil Google locale : logo coloré + barre de recherche.
pub fn google_home() -> &'static str {
    r#"<!doctype html><html><head><title>Google</title>
<style>
body{background:#fff;color:#202124;font-family:sans-serif;margin:0;padding:0;text-align:center}
.logo{margin:60px 0 24px;font-size:64px;letter-spacing:-2px;font-weight:bold}
.logo .b{color:#4285f4}.logo .r{color:#ea4335}.logo .y{color:#fbbc05}.logo .g{color:#34a853}
.bar{background:#fff;border:1px solid #dfe1e5;border-radius:24px;padding:10px 20px;
     font-size:14px;width:460px;display:inline-block;margin:0 auto}
.row{margin:20px 0}
.btn{background:#f8f9fa;border:1px solid #dadce0;border-radius:4px;
     color:#3c4043;padding:8px 16px;font-size:13px;margin:4px;text-decoration:none;display:inline-block}
.hint{color:#9aa0a6;font-size:12px;margin-top:32px}
</style>
</head><body>
<div class="logo">
  <span class="b">G</span><span class="r">o</span><span class="y">o</span><span class="b">g</span><span class="g">l</span><span class="r">e</span>
</div>
<div class="row">
  <input class="bar" type="text" placeholder="Rechercher ou saisir une URL" value="">
</div>
<div class="row">
  <a class="btn" href="https://www.google.com/">Recherche Google</a>
  <a class="btn" href="https://www.google.com/?btnI=I">J'ai de la chance</a>
</div>
<div class="hint">Nautile — moteur souverain Bouchaud OS — <a href="about:bouchaud">Accueil</a></div>
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
