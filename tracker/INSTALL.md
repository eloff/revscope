To install the tracker on your site, add this stub near the top of each page,
ideally inside `<head>`. It loads the tracker script, and queues commands
issued before the tracker is ready so they can be replayed:

```html
<script>
  (function (w, n, u, c) {
    if (u && typeof u === "object") {
      c = u;
      u = null;
    }
    c = c || {};
    u = u || "https://in.revscope.co/";
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
  })(window, "REPLACE_WITH_PROJECT_NAME");
</script>
```

Replace `REPLACE_WITH_PROJECT_NAME` with your project name. Project names must
be at least 3 characters and may contain only lowercase ASCII letters, digits,
and underscores (`a-z0-9_`).

The JavaScript API is `dispatch(...)`. If that name is already used on the
page, pass `{ name: "yourName" }` in the stub config and call `yourName(...)`
instead.

The server URL is optional and defaults to `https://in.revscope.co/`. Pass a
custom `https://` URL (ending in a slash) as the third argument when you proxy
the tracker through your own domain. When the URL is omitted, an optional config
object can be the third argument.

For local development against the example app (`tracker/example`):

1. Start the revscope backend on `localhost:4320` (default listen is
   `127.0.0.1:4320`).
2. From `tracker/example`, run `node server.js` (or `pnpm start`).
3. Open `http://localhost:3446/` — pages install the queue stub for project
   `example`, which loads `http://localhost:4320/example.js` and sends ingest
   POSTs to `http://localhost:4320/example`.

See https://revscope.co/docs/tracking-script and https://revscope.co/docs/custom-events
for the full configuration and event API reference.
