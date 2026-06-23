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
        "<!doctype html><html><head><title>Nautile — Bouchaud OS</title><style>\
         *{{box-sizing:border-box}}\
         body{{background:#f8f9fa;color:#202124;font-family:sans-serif;margin:0;padding:0}}\
         .header{{background:linear-gradient(135deg,#1a73e8 0%,#0d47a1 100%);\
                  color:#fff;padding:24px 20px;text-align:center}}\
         .header h1{{margin:0 0 4px;font-size:28px;letter-spacing:1px}}\
         .header .sub{{color:#d2e3fc;font-size:13px}}\
         .cards{{padding:12px 16px}}\
         .card{{background:#fff;border:1px solid #dadce0;border-radius:6px;\
                padding:12px 14px;margin:8px 0;box-shadow:0 1px 3px rgba(0,0,0,.08)}}\
         .card h3{{margin:0 0 6px;color:#1a73e8;font-size:14px}}\
         .card p,.card li{{font-size:12px;margin:4px 0;line-height:1.5}}\
         .card ul{{margin:4px 0;padding-left:16px}}\
         a{{color:#1a73e8;text-decoration:none}}\
         a:hover{{text-decoration:underline}}\
         .badge{{display:inline-block;background:#e8f0fe;color:#1a73e8;\
                 border-radius:3px;padding:1px 5px;font-size:11px;margin-left:4px}}\
         .tag-https{{color:#1e8e3e}}.tag-http{{color:#e53935}}\
         </style></head><body>\
         <div class=\"header\">\
           <h1>Nautile</h1>\
           <div class=\"sub\">Bouchaud OS {ver} — Moteur de rendu souverain</div>\
         </div>\
         <div class=\"cards\">\
           <div class=\"card\">\
             <h3>Navigation rapide</h3>\
             <ul>\
               <li><a href=\"https://example.com/\">example.com</a> \
                   <span class=\"badge tag-https\">HTTPS</span> — page de test</li>\
               <li><a href=\"https://www.wikipedia.org/\">wikipedia.org</a> — encyclopedie</li>\
               <li><a href=\"about:calc\">Calculatrice</a> — application JS native</li>\
               <li><a href=\"about:wasm\">Demo WebAssembly</a> — module wasm integre</li>\
               <li><a href=\"about:system\">Informations systeme</a></li>\
               <li><a href=\"file:/readme.txt\">readme.txt</a> — fichier local (RAMFS)</li>\
             </ul>\
           </div>\
           <div class=\"card\">\
             <h3>Moteur Nautile</h3>\
             <p>Rendu <b>local et souverain</b> : aucun binaire tiers. \
                Pile réseau TLS 1.3 intégrée au noyau Rust no_std.</p>\
             <ul>\
               <li>HTML5 tolérant — DOM + CSS cascade (selecteurs tag / .classe / #id)</li>\
               <li>Flexbox, marges, paddings, text-align, font-weight</li>\
               <li>Images PNG décodeées en mémoire</li>\
               <li>Interpréteur JavaScript (DOM, timers, WebAssembly via wasmi)</li>\
               <li>HTTP/1.1 + HTTP/2 + TLS 1.3 + DNS + DHCP</li>\
             </ul>\
           </div>\
           <div class=\"card\">\
             <h3>Raccourcis clavier</h3>\
             <ul>\
               <li><b>↑ / ↓</b> — défilement de page</li>\
               <li><b>Entrée</b> — naviguer vers l'URL saisie</li>\
               <li>Entrer un <b>numéro</b> seul = suivre le lien correspondant</li>\
             </ul>\
           </div>\
         </div>\
         </body></html>",
        ver = crate::VERSION
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
         </table>\
         <p><a href=\"about:bouchaud\">← Accueil</a></p>\
         </body></html>",
        ver = crate::VERSION,
        h = dt.hour, m = dt.minute, s = dt.second,
        up = uptime, used = used, total = total, free = free,
        pci = pci_n, user = user,
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
