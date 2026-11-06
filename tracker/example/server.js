/**
 * Serves the demo site (port 3446) and a minimal cross-origin page (port 3447)
 * so document.referrer is set when entering the demo from the “external” origin.
 *
 * Usage: node server.js
 *   MAIN_PORT=3446 EXTERNAL_PORT=3447 node server.js
 *
 * Tracker script + ingest go to the revscope backend at localhost:4320 (queue
 * stub in demo-config / page stubs). Start that server separately.
 */

var http = require("http");
var fs = require("fs");
var path = require("path");

var MAIN_PORT = parseInt(process.env.MAIN_PORT || "3446", 10);
var EXTERNAL_PORT = parseInt(process.env.EXTERNAL_PORT || "3447", 10);

var ROOT = path.join(__dirname, "public");
var TRACKER_JS = path.join(__dirname, "..", "tracker.js");

var MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
};

function safeJoin(base, unsafe) {
  var resolved = path.normalize(path.join(base, unsafe));
  if (!resolved.startsWith(base)) {
    return null;
  }
  return resolved;
}

function sendText(res, status, body, contentType) {
  res.writeHead(status, { "Content-Type": contentType || "text/plain; charset=utf-8" });
  res.end(body);
}

function serveTracker(req, res) {
  fs.readFile(TRACKER_JS, function (err, data) {
    if (err) {
      sendText(res, 500, "Could not read tracker.js", "text/plain; charset=utf-8");
      return;
    }
    res.writeHead(200, {
      "Content-Type": "application/javascript; charset=utf-8",
      "Cache-Control": "no-store",
    });
    res.end(data);
  });
}

function resolvePublicFile(urlPath) {
  if (urlPath.indexOf("/spa-demo") === 0) {
    return path.join(ROOT, "spa.html");
  }
  if (urlPath.indexOf("/filters-demo") === 0) {
    return path.join(ROOT, "filters.html");
  }
  if (urlPath.indexOf("/ignored-demo") === 0 || /^\/secret(-|$)/.test(urlPath)) {
    return path.join(ROOT, "filters.html");
  }
  if (urlPath.indexOf("/shop/") === 0) {
    return path.join(ROOT, "path-rewrite.html");
  }
  return safeJoin(ROOT, urlPath);
}

function serveStatic(req, res) {
  var raw = req.url.split("?")[0];
  var urlPath = raw === "/" ? "/index.html" : raw;
  var filePath = resolvePublicFile(urlPath);
  if (!filePath || !filePath.startsWith(ROOT)) {
    sendText(res, 403, "Forbidden");
    return;
  }
  fs.stat(filePath, function (err, st) {
    if (err || !st.isFile()) {
      sendText(res, 404, "Not found");
      return;
    }
    var ext = path.extname(filePath);
    var ct = MIME[ext] || "application/octet-stream";
    res.writeHead(200, { "Content-Type": ct });
    fs.createReadStream(filePath).pipe(res);
  });
}

function mainServer(req, res) {
  var pathname = req.url.split("?")[0];
  var qs = req.url.indexOf("?") >= 0 ? req.url.slice(req.url.indexOf("?")) : "";

  // Optional local copy of tracker.js (pages load example.js from the backend).
  if (pathname === "/script.js" || pathname === "/t.js") {
    serveTracker(req, res);
    return;
  }

  if (pathname === "/filters.html") {
    res.writeHead(302, { Location: "/filters-demo" + qs });
    res.end();
    return;
  }
  if (pathname === "/spa.html") {
    res.writeHead(302, { Location: "/spa-demo" + qs });
    res.end();
    return;
  }
  if (pathname === "/path-rewrite.html") {
    res.writeHead(302, { Location: "/shop/sku-42" + qs });
    res.end();
    return;
  }

  serveStatic(req, res);
}

/**
 * Minimal page on another port/origin so navigation to MAIN_PORT sends Referer.
 * Link targets 127.0.0.1 so it matches how users often hit the main demo.
 */
function externalServer(req, res) {
  var mainHost = "127.0.0.1:" + MAIN_PORT;
  var body =
    "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>External entry (simulated)</title>" +
    "<link rel=\"stylesheet\" href=\"http://" +
    mainHost +
    "/styles.css\"></head><body><main class=\"wrap\">" +
    "<p class=\"badge\">Port " +
    EXTERNAL_PORT +
    " — different origin from port " +
    MAIN_PORT +
    "</p>" +
    "<h1>Simulated external landing</h1>" +
    "<p>In production, the first pageview’s referrer often shows where visitors came from " +
    "(for example <strong>google.com</strong> after a search). Browsers do not let JavaScript " +
    "set <code>document.referrer</code>, so this demo uses a second origin on loopback. " +
    "After you click through, open DevTools → Network and inspect POSTs to " +
    "<code>localhost:4320</code> for the <code>r</code> (referrer) field.</p>" +
    "<p><a class=\"btn primary\" href=\"http://" +
    mainHost +
    "/landing.html?utm_source=google&utm_medium=organic&utm_campaign=demo&utm_content=hero&utm_term=analytics&gclid=demo-gclid-123&ref=https%3A%2F%2Fwww.google.com%2F\">" +
    "Continue to demo landing (sets referrer + UTMs + gclid)</a></p>" +
    "<p class=\"muted\">The <code>ref</code> query param is included in allowed URL params and appears in " +
    "the pageview metadata payload (<code>meta</code>) alongside UTMs — separate from the HTTP referrer header.</p>" +
    "</main></body></html>";

  sendText(res, 200, body, MIME[".html"]);
}

http.createServer(mainServer).listen(MAIN_PORT, function () {
  console.log("Demo site:     http://localhost:" + MAIN_PORT + "/");
  console.log("Tracker URL:   http://localhost:4320/example.js  (start revscope separately)");
  console.log("Ingest host:   http://localhost:4320/");
});

http.createServer(externalServer).listen(EXTERNAL_PORT, function () {
  console.log("External stub: http://127.0.0.1:" + EXTERNAL_PORT + "/  (open this first, then click through)");
});
