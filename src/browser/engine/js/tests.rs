//! Selftests du moteur JS (commande shell `js-selftest`) : globales, DOM,
//! hoisting, polyfills, viewport, location/URL, modules ES, collections.

use super::*;

// ============================================================================
// Selftest
// ============================================================================

pub fn selftest() -> Result<(), &'static str> {
    let html = br#"<div id="app">old</div><script>
        var who = 'Bouchaud';
        document.getElementById('app').innerHTML = '<b>' + who + '</b>';
        document.write('<p>OK</p>');
    </script>"#;
    let out = execute_inline(html, "");
    let s = core::str::from_utf8(&out).map_err(|_| "utf8")?;
    if !s.contains("<div id=\"app\"><b>Bouchaud</b></div>") { return Err("innerHTML"); }
    if !s.contains("<p>OK</p>") || s.contains("<script>") { return Err("write"); }
    // langage : arithmetique, boucle, fonction
    let mut it = Interp::new();
    it.run("var s=0; for(var i=1;i<=10;i++){s+=i;} function f(x){return x*x;} console.log(s+','+f(5));").map_err(|_| "run")?;
    if it.out.last().map(|x| x.as_str()) != Some("55,25") { return Err("lang"); }
    // closures + liaison let par iteration
    let mut it = Interp::new();
    it.run("var f=[]; for(let i=0;i<3;i++){f.push(()=>i);} console.log(f[0]()+''+f[1]()+f[2]());").map_err(|_| "run2")?;
    if it.out.last().map(|x| x.as_str()) != Some("012") { return Err("closure"); }
    // tableaux d'ordre superieur + objets + JSON
    let mut it = Interp::new();
    it.run("var a=[1,2,3,4]; var o={n:a.filter(x=>x%2===0).reduce((s,x)=>s+x,0)}; console.log(JSON.stringify(o));").map_err(|_| "run3")?;
    if it.out.last().map(|x| x.as_str()) != Some("{\"n\":6}") { return Err("hof"); }
    // Promise + microtaches (drainees par pump)
    let mut it = Interp::new();
    it.run("var px=0; Promise.resolve(41).then(function(v){ px=v+1; });").map_err(|_| "run4")?;
    it.pump();
    it.run("console.log(px);").map_err(|_| "run5")?;
    if it.out.last().map(|x| x.as_str()) != Some("42") { return Err("promise"); }
    // setTimeout (macrotache, drainee par pump)
    let mut it = Interp::new();
    it.run("var ty=0; setTimeout(function(){ ty=7; }, 0);").map_err(|_| "run6")?;
    it.pump();
    it.run("console.log(ty);").map_err(|_| "run7")?;
    if it.out.last().map(|x| x.as_str()) != Some("7") { return Err("timer"); }
    // addEventListener('click') + dispatch via element.click()
    let (mut ctx, _o) = open_page(br#"<button id="b">x</button><script>
        var n=0;
        document.getElementById('b').addEventListener('click', function(){ n++; document.getElementById('b').textContent = 'clic'+n; });
    </script>"#, "");
    let html = ctx.dispatch("document.getElementById('b').click()");
    let s = core::str::from_utf8(&html).unwrap_or("");
    if !s.contains("clic1") { return Err("event"); }
    // style live -> attribut style (repris par le layout)
    let out = execute_inline(br#"<div id="d">hi</div><script>document.getElementById('d').style.color='red';</script>"#, "");
    let s = core::str::from_utf8(&out).unwrap_or("");
    if !s.contains("color:red") { return Err("style"); }
    // --- objet global : coherence window/globalThis/self/this + pont var<->window ---
    // Identite des alias (window === globalThis === self === this racine).
    let mut it = Interp::new();
    it.run("console.log((window===globalThis)+','+(window===self)+','+(this===window));").map_err(|_| "run-glob1")?;
    if it.out.last().map(|x| x.as_str()) != Some("true,true,true") { return Err("global-alias"); }
    // var global visible via window ; propriete window visible en variable nue.
    let mut it = Interp::new();
    it.run("var gx=1; window.gy=2; console.log(window.gx+','+gy);").map_err(|_| "run-glob2")?;
    if it.out.last().map(|x| x.as_str()) != Some("1,2") { return Err("global-bridge"); }
    // Ecriture par index (Closure-compiler : window[name]=...) -> variable nue.
    let mut it = Interp::new();
    it.run("window['gz']=3; console.log(gz+','+(typeof self.gz));").map_err(|_| "run-glob3")?;
    if it.out.last().map(|x| x.as_str()) != Some("3,number") { return Err("global-index"); }
    // Motif d'amorçage Google : _ / _DumpException definis via window puis lus nus.
    let mut it = Interp::new();
    it.run("window._ = window._ || {}; window._DumpException = _._DumpException = function(e){ throw e; }; \
            console.log((typeof _DumpException)+','+(typeof window._DumpException)+','+(typeof _._DumpException));").map_err(|_| "run-glob4")?;
    if it.out.last().map(|x| x.as_str()) != Some("function,function,function") { return Err("dumpexception"); }
    // --- Sprint 2 : collections DOM (length toujours defini, jamais undefined) ---
    let html = br#"<ul id="l"><li>a</li><li class="x">b</li><li>c</li></ul>
<form id="f"><input name="q"><button>go</button></form>
<div id="o"></div>
<script>
  var l = document.getElementById('l');
  var d = document.createElement('div');
  var f = document.getElementById('f');
  var r = [l.children.length, document.querySelectorAll('li').length,
           document.getElementsByTagName('li').length, d.children.length,
           l.children[0].nextElementSibling.className, f.elements.length,
           l.firstElementChild.textContent].join('|');
  document.getElementById('o').textContent = r;
</script>"#;
    let out = execute_inline(html, "");
    let s = core::str::from_utf8(&out).unwrap_or("");
    if !s.contains("3|3|3|0|x|2|a") { return Err("dom-collections"); }
    // classList array-like : length, indexation, contains.
    let out = execute_inline(br#"<div id="c" class="alpha beta"></div><div id="o2"></div><script>
  var c = document.getElementById('c');
  document.getElementById('o2').textContent = c.classList.length + ',' + c.classList.contains('beta') + ',' + c.classList[0];
</script>"#, "");
    let s = core::str::from_utf8(&out).unwrap_or("");
    if !s.contains("2,true,alpha") { return Err("classlist"); }
    // --- Sprint 3 : hoisting `var` fonction-scope (racine de `b is not defined`) ---
    let mut it = Interp::new();
    it.run("function f(){ var r=(x===undefined); var x=1; return r; } \
            function g(){ for(var i=0;i<3;i++){} return i; } \
            function h(){ { var y=7; } return y; } \
            function k(){ {let z=1;} try{ return ''+z; }catch(e){ return 'ok'; } } \
            console.log(f()+','+g()+','+h()+','+k());").map_err(|_| "run-hoist")?;
    if it.out.last().map(|x| x.as_str()) != Some("true,3,7,ok") { return Err("var-hoisting"); }
    // --- Sprint 4 : motif polyfill Closure (`in` sur window, defineProperty, Number statics) ---
    // Reproduit le detecteur de la page Google : marche le chemin "Number.isSafeInteger"
    // sur globalThis avec `in`, pose le polyfill via Object.defineProperty, puis l'appelle.
    let mut it = Interp::new();
    it.run("var w=('Number' in globalThis)&&('Object' in window); \
            var q=function(p,f){var c=globalThis;p=p.split('.');for(var i=0;i<p.length-1;i++){if(!(p[i] in c))return 'miss';c=c[p[i]];}var k=p[p.length-1];var e=c[k];var v=f(e);if(v!=e&&v!=null)Object.defineProperty(c,k,{configurable:true,writable:true,value:v});return 'ok';}; \
            var r1=q('Number.isSafeInteger',function(a){return a?a:function(b){return typeof b==='number'&&b===b&&b<=9007199254740991;}}); \
            var r2=q('Object.zzTest',function(a){return a?a:function(){return 42;}}); \
            console.log(w+','+r1+','+Number.isSafeInteger(3)+','+Number.isFinite('1')+','+r2+','+Object.zzTest());").map_err(|_| "run-closure")?;
    if it.out.last().map(|x| x.as_str()) != Some("true,ok,true,false,ok,42") { return Err("closure-polyfill"); }
    // _DumpException par defaut : appelable avant que la page le definisse.
    let mut it = Interp::new();
    it.run("var d1=(typeof _DumpException); _DumpException(Error('x')); \
            window._DumpException=function(e){return 'page';}; \
            console.log(d1+','+_DumpException(0));").map_err(|_| "run-dump")?;
    if it.out.last().map(|x| x.as_str()) != Some("function,page") { return Err("dumpexception-default"); }
    // --- Sprint 5 : viewport + mesures typees ---
    let (ctx, _o) = open_page_sized(br#"<body><div id="x">t</div><script>
  var r = document.getElementById('x').getBoundingClientRect();
  window.__m = [window.innerWidth, window.innerHeight, document.documentElement.clientHeight,
                typeof r.width, document.getElementById('x').offsetWidth].join(',');
</script></body>"#, "", 800, 500);
    let mut ctx = ctx;
    let v = ctx.interp.run("__m").map_err(|_| "run-viewport")?;
    if ctx.interp.to_string(&v) != "800,500,500,number,800" { return Err("viewport"); }
    // --- Render Core v1 : ComputedStyle + LayoutTree + reflow force ---
    let (mut ctx, _o) = open_page_sized(br#"<body><div id="r" style="display:block;width:320px;height:40px;padding:10px;color:red">x</div><script>
      var e=document.getElementById('r');
      var r1=e.getBoundingClientRect();
      e.style.width='360px'; e.style.height='40px'; e.style.color='red';
      var r2=e.getBoundingClientRect();
      var cs=getComputedStyle(e);
      window.__rc=[r1.width,r2.width,cs.getPropertyValue('display'),cs.getPropertyValue('color'),e.offsetHeight].join('|');
    </script></body>"#, "", 800, 500);
    let v = ctx.interp.run("__rc").map_err(|_| "run-render-core")?;
    if ctx.interp.to_string(&v) != "320|360|block|rgb(255, 0, 0)|40" { return Err("render-core-v1"); }

    // --- Sprint 6 : location complet, URL/URLSearchParams, modules ES ---
    // location derive de l'URL reelle (search/hash/hostname/origin presents).
    let mut it = Interp::new();
    it.base_url = String::from("https://www.ecosia.org/search?q=chat&lang=fr");
    it.install_location();
    it.run("console.log([location.hostname, location.pathname, location.search, location.origin, \
            window.location.search===document.location.search].join('|'));").map_err(|_| "run-location")?;
    if it.out.last().map(|x| x.as_str()) != Some("www.ecosia.org|/search|?q=chat&lang=fr|https://www.ecosia.org|true") { return Err("location"); }
    // URLSearchParams + new URL : parsing et resolution relative.
    it.run("var p=new URLSearchParams(location.search); \
            var u=new URL('../img/logo.png','https://a.b/c/d/page.html'); \
            console.log([p.get('q'), p.has('lang'), (p.get('absent')===null), u.href, u.hostname].join('|'));").map_err(|_| "run-url")?;
    if it.out.last().map(|x| x.as_str()) != Some("chat|true|true|https://a.b/c/img/logo.png|a.b") { return Err("url-searchparams"); }
    // Modules ES : export (const/function/default/named) collectes dans le
    // namespace, import consomme le namespace, portee isolee du global.
    let mut it = Interp::new();
    let ns = new_obj(Obj::plain());
    it.run_module("export const a=1; export function double(x){return 2*x;} \
                   const secret=9; export default 'dflt'; export {secret as s};",
                  "https://x/m.js", ns.clone()).map_err(|_| "run-module")?;
    it.modules.insert(String::from("https://x/m.js"), ns);
    it.run_module("import d, {a, double as dbl, s} from './m.js'; \
                   window.__mod = [d, a, dbl(21), s, typeof secret].join('|');",
                  "https://x/main.js", new_obj(Obj::plain())).map_err(|_| "run-import")?;
    let v = it.run("__mod").map_err(|_| "read-mod")?;
    if it.to_string(&v) != "dflt|1|42|9|undefined" { return Err("modules-es"); }
    // Le DOM expose desormais les noeuds <script> (parentNode des loaders).
    let (mut ctx, _o) = open_page_sized(br#"<body><script>
        window.__s = (function(){ var t=document.getElementsByTagName('script');
            return [t.length>0, t[0]&&t[0].parentNode?t[0].parentNode.tagName:'?'].join('|'); })();
    </script></body>"#, "", 800, 500);
    let v = ctx.interp.run("__s").map_err(|_| "run-script-dom")?;
    if ctx.interp.to_string(&v) != "true|BODY" { return Err("script-in-dom"); }
    // --- Sprint 7 : WeakMap/WeakSet/Proxy/Reflect + elements custom a tiret ---
    let mut it = Interp::new();
    it.run("var w=new WeakSet(); var o={a:1}; w.add(o); var m=new WeakMap(); m.set(o,7); \
            var p=new Proxy(o,{}); \
            console.log([w.has(o), m.get(o), p.a, typeof Reflect.get, Reflect.get(o,'a')].join('|'));").map_err(|_| "run-weak")?;
    if it.out.last().map(|x| x.as_str()) != Some("true|7|1|function|1") { return Err("weakset-proxy"); }
    // <audio-recorder> ne doit plus etre confondu avec <audio> (tags a tiret).
    let (mut ctx, _o) = open_page_sized(br#"<body><audio-recorder locale="fr"></audio-recorder><script>
        window.__t = [document.getElementsByTagName('audio').length,
                      document.getElementsByTagName('audio-recorder').length].join('|');
    </script></body>"#, "", 800, 500);
    let v = ctx.interp.run("__t").map_err(|_| "run-custom-tag")?;
    if ctx.interp.to_string(&v) != "0|1" { return Err("custom-element-tag"); }
    // --- Sprint 8 : document.scripts / currentScript + stub bare specifier ---
    let (mut ctx, _o) = open_page_sized(br#"<body><script>
        window.__cs = [document.scripts.length>0,
                       document.currentScript ? 'oui' : 'non',
                       document.currentScript.tagName].join('|');
    </script></body>"#, "", 800, 500);
    let v = ctx.interp.run("__cs").map_err(|_| "run-currentscript")?;
    if ctx.interp.to_string(&v) != "true|oui|SCRIPT" { return Err("current-script"); }
    // currentScript redevient null apres l'execution des scripts.
    let v = ctx.interp.run("document.currentScript===null").map_err(|_| "run-cs-null")?;
    if ctx.interp.to_string(&v) != "true" { return Err("current-script-null"); }
    // Bare specifier : namespace stub loggue, sans erreur ni fetch.
    let mut it = Interp::new();
    it.run_module("import React from 'react'; window.__r = typeof React;",
                  "https://x/main.js", new_obj(Obj::plain())).map_err(|_| "run-bare")?;
    let v = it.run("__r").map_err(|_| "read-bare")?;
    if it.to_string(&v) != "object" { return Err("bare-specifier-stub"); }
    // --- Sprint 9 : hit-test reel — <button> devient une cible de clic DOM ---
    // Avant ce sprint, <button> ne produisait NI boite NI cible : c'etait la
    // cause n°1 des "clic sans cible" (icones/boutons Google/Ecosia inertes).
    {
        use crate::browser::engine::web::Session;
        let html = br#"<button id="b">Go</button><script>
            document.getElementById('b').addEventListener('click', function(){
                document.getElementById('b').textContent = 'clicked';
                console.log('button-click-fired');
            });
        </script>"#;
        let (mut session, page) = Session::open(html, "https://t.example/", 800);
        let target = page.links.iter().find(|l| l.href.starts_with("js-click:"))
            .ok_or("button-no-click-target")?;
        let key = target.href.strip_prefix("js-click:").ok_or("button-bad-key")?.to_string();
        if key != "b" { return Err("button-key-not-id"); }
        let _page2 = session.click_node(&key);
        if session.console_out().last().map(|s| s.as_str()) != Some("button-click-fired") {
            return Err("button-click-not-dispatched");
        }
    }
    Ok(())
}
