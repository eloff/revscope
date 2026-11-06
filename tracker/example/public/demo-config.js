/**
 * Standard install queue stub for the tracker example app.
 * Loads {url}{project}.js from the local backend and queues commands until ready.
 *
 * hostname must be a real DNS name (not localhost / IP) or the tracker
 * suppresses events. Override to example.com so the loopback demo tracks.
 *
 * Run the revscope backend on localhost:4320 so script + ingest succeed.
 */
(function (w, n, u, c) {
  c = c || {};
  c.url = u;
  u += n + ".js";
  c.project = n;
  n = c.name || "dispatch";
  if (typeof w[n] === "function") return;
  var q = [];
  function f() {
    q.push([].slice.call(arguments));
  }
  f.q = f.a = q;
  f.c = c;
  w[n] = f;
  var s = document.createElement("script");
  s.src = u;
  s.defer = true;
  document.head.appendChild(s);
})(window, "example", "http://localhost:4320/", { hostname: "example.com" });

window.RR_DEMO = {
  projectName: "example",
  backend: "http://localhost:4320",
};
