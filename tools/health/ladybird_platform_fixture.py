#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, HTTPServer

PAGE = r'''<!doctype html>
<meta charset="utf-8">
<title>Bouchaud Ladybird full platform</title>
<script>
(async () => {
  const results = {};
  const mark = (name, ok, detail = "") => {
    results[name] = !!ok;
    console.log(`PLATFORM_${name.toUpperCase()} ${ok ? "OK" : "FAIL"} ${detail}`.trim());
  };

  try {
    localStorage.setItem("bouchaud", "ladybird");
    mark("localstorage", localStorage.getItem("bouchaud") === "ladybird");
  } catch (e) { mark("localstorage", false, String(e)); }

  try {
    sessionStorage.setItem("session", "ok");
    mark("sessionstorage", sessionStorage.getItem("session") === "ok");
  } catch (e) { mark("sessionstorage", false, String(e)); }

  try {
    document.cookie = "bouchaud_cookie=ok; SameSite=Lax";
    mark("cookie", document.cookie.includes("bouchaud_cookie=ok"));
  } catch (e) { mark("cookie", false, String(e)); }

  try {
    const canvas = document.createElement("canvas");
    canvas.width = 2; canvas.height = 2;
    const c = canvas.getContext("2d");
    c.fillStyle = "rgb(11,22,33)";
    c.fillRect(0,0,2,2);
    const p = c.getImageData(0,0,1,1).data;
    mark("canvas", p[0] === 11 && p[1] === 22 && p[2] === 33 && p[3] === 255);
  } catch (e) { mark("canvas", false, String(e)); }

  try {
    const src = `onmessage=e=>{if(e.data==="ping")postMessage("pong")}`;
    const worker = new Worker(URL.createObjectURL(new Blob([src], {type:"text/javascript"})));
    const answer = await new Promise((resolve,reject) => {
      const t=setTimeout(()=>reject(new Error("timeout")),10000);
      worker.onmessage=e=>{clearTimeout(t);resolve(e.data)};
      worker.onerror=()=>{clearTimeout(t);reject(new Error("worker"))};
      worker.postMessage("ping");
    });
    mark("worker", answer === "pong");
    worker.terminate();
  } catch (e) { mark("worker", false, String(e)); }

  try {
    const bytes = new Uint8Array([
      0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,
      7,5,1,1,102,0,0,10,6,1,4,0,65,42,11
    ]);
    const {instance} = await WebAssembly.instantiate(bytes);
    mark("wasm", instance.exports.f() === 42);
  } catch (e) { mark("wasm", false, String(e)); }

  try {
    const response = await fetch("/api");
    mark("fetch", response.ok && (await response.text()) === "fetch-ok");
  } catch (e) { mark("fetch", false, String(e)); }

  try {
    const openRequest = indexedDB.open("bouchaud-platform", 1);
    const db = await new Promise((resolve,reject) => {
      openRequest.onupgradeneeded = () => openRequest.result.createObjectStore("kv");
      openRequest.onsuccess = () => resolve(openRequest.result);
      openRequest.onerror = () => reject(openRequest.error);
    });
    const tx = db.transaction("kv", "readwrite");
    tx.objectStore("kv").put("ok", "key");
    await new Promise((resolve,reject) => {
      tx.oncomplete=resolve; tx.onerror=()=>reject(tx.error);
    });
    const rx = db.transaction("kv").objectStore("kv").get("key");
    const value = await new Promise((resolve,reject) => {
      rx.onsuccess=()=>resolve(rx.result); rx.onerror=()=>reject(rx.error);
    });
    mark("indexeddb", value === "ok");
    db.close();
  } catch (e) { mark("indexeddb", false, String(e)); }

  mark("fullscreen_api", "fullscreenEnabled" in document);
  mark("clipboard_api", !!navigator.clipboard || typeof document.execCommand === "function");

  const required = ["localstorage","sessionstorage","cookie","canvas","worker","wasm","fetch","indexeddb"];
  const good = required.every(k => results[k]);
  console.log(`PLATFORM_FULL_${good ? "OK" : "FAIL"} passed=${required.filter(k=>results[k]).length}/${required.length}`);
})();
</script>
'''

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?",1)[0]
        if path == "/platform.html":
            body = PAGE.encode()
            ctype = "text/html; charset=utf-8"
        elif path == "/api":
            body = b"fetch-ok"
            ctype = "text/plain"
        elif path == "/download.bin":
            body = b"Bouchaud download proof\n"
            ctype = "application/octet-stream"
        else:
            self.send_response(404); self.end_headers(); return

        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        print(f"PLATFORM_FIXTURE path={path}", flush=True)

    def log_message(self, fmt, *args):
        print("[platform-fixture]", fmt % args, flush=True)

HTTPServer(("0.0.0.0", 18083), Handler).serve_forever()
