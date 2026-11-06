import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const trackerSource = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "..", "tracker.js"),
  "utf8",
);

function createCookieJar() {
  const cookies = new Map();

  return {
    get cookie() {
      const parts = [];
      for (const [name, value] of cookies) {
        parts.push(name + "=" + value);
      }
      return parts.join("; ");
    },
    set cookie(raw) {
      const first = String(raw).split(";")[0];
      const equals = first.indexOf("=");
      if (equals < 0) {
        return;
      }
      const name = first.slice(0, equals).trim();
      const value = first.slice(equals + 1);
      const maxAgeMatch = /(?:^|;\s*)max-age=(-?\d+)/i.exec(raw);
      if (maxAgeMatch && Number(maxAgeMatch[1]) <= 0) {
        cookies.delete(name);
        return;
      }
      cookies.set(name, value);
    },
    clear() {
      cookies.clear();
    },
    entries() {
      return Object.fromEntries(cookies);
    },
  };
}

function createElement(tagName, attrs) {
  const attributes = Object.assign({}, attrs || {});
  const style = {
    display: "",
    visibility: "",
  };
  const element = {
    nodeName: String(tagName || "DIV").toUpperCase(),
    nodeType: 1,
    parentNode: null,
    children: [],
    style: style,
    currentStyle: style,
    id: attributes.id || "",
    getAttribute(name) {
      if (!Object.hasOwn(attributes, name)) {
        return null;
      }
      return attributes[name];
    },
    setAttribute(name, value) {
      attributes[name] = String(value);
      if (name === "id") {
        element.id = String(value);
      }
    },
    removeAttribute(name) {
      delete attributes[name];
    },
    getBoundingClientRect() {
      return (
        element._rect || {
          top: 0,
          left: 0,
          bottom: 100,
          right: 100,
          width: 100,
          height: 100,
        }
      );
    },
    contains(other) {
      let node = other;
      while (node) {
        if (node === element) {
          return true;
        }
        node = node.parentNode;
      }
      return false;
    },
  };
  return element;
}

// Records observers so tests can deliver mutation records by hand.
function createMutationObserverFactory(observers) {
  return function MutationObserver(callback) {
    const observer = {
      callback: callback,
      targets: [],
      observe(target, options) {
        observer.targets.push({ target: target, options: options });
      },
      disconnect() {
        observer.targets.length = 0;
      },
      takeRecords: () => [],
      // Test helper: pretend the browser observed these records.
      emit(records) {
        callback(records, observer);
      },
    };
    observers.push(observer);
    return observer;
  };
}

function createXhrFactory(requests, defaultResponse) {
  return function XMLHttpRequest() {
    const xhr = {
      readyState: 0,
      status: 0,
      responseText: "",
      withCredentials: false,
      onreadystatechange: null,
      _headers: {},
      open(method, url) {
        xhr.method = method;
        xhr.url = url;
        xhr.readyState = 1;
      },
      setRequestHeader(name, value) {
        xhr._headers[name] = value;
      },
      send(body) {
        xhr.body = body;
        requests.push(xhr);
        xhr.readyState = 4;
        xhr.status = xhr._autoStatus == null ? 200 : xhr._autoStatus;
        xhr.responseText =
          xhr._autoResponseText == null
            ? JSON.stringify(
                defaultResponse || {
                  visitor_timestamp_id: "00000000499602d2",
                  storage_allowed: true,
                },
              )
            : xhr._autoResponseText;
        if (typeof xhr.onreadystatechange === "function") {
          xhr.onreadystatechange();
        }
      },
    };
    return xhr;
  };
}

function installGlobals(map) {
  const previous = {};
  for (const key of Object.keys(map)) {
    previous[key] = globalThis[key];
    globalThis[key] = map[key];
  }
  return function restore() {
    for (const key of Object.keys(map)) {
      if (previous[key] === undefined) {
        delete globalThis[key];
      } else {
        globalThis[key] = previous[key];
      }
    }
  };
}

/**
 * Load tracker.js against a fresh mock browser window.
 * Returns { window, rr, requests, cookies, createElement }.
 */
export function loadTracker(options) {
  options = options || {};
  const projectName =
    options.projectName !== undefined
      ? options.projectName
      : options.config && Object.hasOwn(options.config, "projectName")
        ? options.config.projectName
        : "demo";
  const cookieJar = createCookieJar();
  if (options.cookies) {
    for (const [name, value] of Object.entries(options.cookies)) {
      cookieJar.cookie = name + "=" + value;
    }
  }

  const listeners = {};
  const requests = [];
  const beacons = [];
  const observers = [];
  const timers = new Set();
  function trackedSetTimeout(fn, delay) {
    const id = setTimeout(() => {
      timers.delete(id);
      fn();
    }, delay);
    timers.add(id);
    return id;
  }
  function trackedClearTimeout(id) {
    timers.delete(id);
    clearTimeout(id);
  }
  const script = createElement("script", {
    src: "https://127.0.0.1:4320/script.js",
  });
  script.src = script.getAttribute("src");
  if (options.scriptAttrs) {
    for (const [name, value] of Object.entries(options.scriptAttrs)) {
      script.setAttribute(name, value);
    }
  }

  const documentElement = createElement("html");
  documentElement.clientWidth = 1024;
  documentElement.clientHeight = 768;
  documentElement.scrollTop = 0;
  documentElement.scrollHeight = 2000;
  documentElement.offsetHeight = 2000;
  documentElement.contains = () => true;

  const body = createElement("body");
  body.scrollHeight = 2000;
  body.offsetHeight = 2000;
  body.clientHeight = 768;
  body.parentNode = documentElement;

  const locationState = {
    protocol: "https:",
    host: "example.com",
    hostname: "example.com",
    pathname: "/",
    search: "",
    hash: "",
    href: "https://example.com/",
  };
  Object.assign(locationState, options.location || {});

  const config = Object.assign({ projectName: projectName }, options.config || {});
  const dispatchName = options.dispatchName || config.name || "dispatch";

  // Deterministic Math so bot detection never flakes. Values span the unit
  // interval so the createId Math.random fallback does not repeat 16 hex chars.
  let randomSeq = 0;
  const math = Object.create(Math);
  math.random = () => {
    randomSeq += 1;
    return (randomSeq % 256) / 256;
  };

  const ImageCtor = function Image() {
    const image = {
      onload: null,
      onerror: null,
      set src(value) {
        image._src = value;
        requests.push({ type: "gif", url: value, image: image });
        if (typeof image.onload === "function") {
          image.onload();
        }
      },
      get src() {
        return image._src;
      },
    };
    return image;
  };

  function addWindowEventListener(type, handler) {
    if (!listeners[type]) {
      listeners[type] = [];
    }
    listeners[type].push(handler);
  }

  function removeWindowEventListener(type, handler) {
    const handlers = listeners[type];
    if (!handlers) {
      return;
    }
    const index = handlers.indexOf(handler);
    if (index !== -1) {
      handlers.splice(index, 1);
    }
  }

  const windowObj = {
    RR_TEST: true,
    console: {
      warn: options.warn || (() => {}),
      log: () => {},
    },
    navigator: Object.assign(
      {
        userAgent: "mozilla/5.0 test",
        language: "en-US",
        webdriver: false,
        maxTouchPoints: 0,
        sendBeacon: (url, data) => {
          beacons.push({ url: url, data: data });
          return true;
        },
      },
      options.navigator || {},
    ),
    location: locationState,
    document: {
      currentScript: options.currentScript === false ? null : script,
      documentElement: documentElement,
      body: body,
      referrer: options.referrer || "",
      doctype: {},
      hidden: false,
      readyState: "complete",
      querySelector: () => script,
      querySelectorAll: () => [],
      getElementsByTagName: (tagName) =>
        String(tagName).toLowerCase() === "script" ? [script] : [],
      createEvent: () => {
        const event = {
          initEvent: (type, bubbles, cancelable) => {
            event.type = type;
            event.bubbles = bubbles;
            event.cancelable = cancelable;
          },
        };
        return event;
      },
      addEventListener: (type, handler) => {
        addWindowEventListener(type, handler);
      },
    },
    addEventListener: (type, handler) => {
      addWindowEventListener(type, handler);
    },
    removeEventListener: (type, handler) => {
      removeWindowEventListener(type, handler);
    },
    dispatchEvent: (event) => {
      const handlers = (listeners[event.type] || []).slice();
      for (const handler of handlers) {
        handler.call(windowObj, event);
      }
      return !event.defaultPrevented;
    },
    screen: { width: 1920, height: 1080 },
    history: {
      pushState: () => {},
    },
    innerWidth: 1024,
    innerHeight: 768,
    matchMedia: () => ({ matches: false }),
    crypto: {
      getRandomValues: (arr) => {
        for (let i = 0; i < arr.length; i++) {
          randomSeq += 1;
          arr[i] = randomSeq % 256;
        }
        return arr;
      },
    },
    XMLHttpRequest: createXhrFactory(requests, options.ingestResponse),
    MutationObserver:
      options.mutationObserver === false ? undefined : createMutationObserverFactory(observers),
    Image: ImageCtor,
    Blob: function Blob(parts, opts) {
      this.parts = parts;
      this.type = opts && opts.type;
    },
    Event: function Event(type) {
      this.type = type;
    },
    Intl: {
      DateTimeFormat: () => ({
        resolvedOptions: () => ({ timeZone: "UTC" }),
      }),
    },
    isFinite: Number.isFinite,
    Date: Date,
    Math: math,
    Object: Object,
    Array: Array,
    JSON: JSON,
    RegExp: RegExp,
    Error: Error,
    Uint8Array: Uint8Array,
    encodeURIComponent: encodeURIComponent,
    decodeURIComponent: decodeURIComponent,
    setTimeout: trackedSetTimeout,
    clearTimeout: trackedClearTimeout,
    getComputedStyle: (node) => node.style || {},
  };

  if (options.currentScript === false) {
    script.readyState = "interactive";
  }

  Object.defineProperty(windowObj.document, "cookie", {
    get() {
      return cookieJar.cookie;
    },
    set(value) {
      cookieJar.cookie = value;
    },
  });

  function installQueueStub(stubConfig, queuedCalls, stubDispatchName) {
    const propertyName = stubDispatchName || stubConfig.name || "dispatch";
    if (windowObj[propertyName]) {
      return windowObj[propertyName];
    }

    // Project name lives on .c.project so the tracker can find the stub
    // regardless of the window function name.
    const stubOptions = Object.assign({}, stubConfig);
    const stubProjectName = stubOptions.projectName;
    delete stubOptions.projectName;
    if (stubProjectName) {
      stubOptions.project = stubProjectName;
    }
    const queue = (queuedCalls || []).slice();
    function stub(action) {
      const command = [action];
      for (let index = 1; index < arguments.length; index++) {
        command.push(arguments[index]);
      }
      queue.push(command);
    }

    stub.q = queue;
    stub.a = queue;
    stub.c = stubOptions;
    windowObj[propertyName] = stub;
    return stub;
  }

  if (options.installStub !== false) {
    installQueueStub(config, options.queue, dispatchName);
  }
  for (const extraStub of options.extraStubs || []) {
    const extraConfig = Object.assign(
      { projectName: extraStub.projectName },
      extraStub.config || {},
    );
    installQueueStub(extraConfig, extraStub.queue, extraStub.dispatchName);
  }
  if (typeof options.beforeLoad === "function") {
    options.beforeLoad(windowObj);
  }

  function runTracker() {
    // Evaluate in the host realm so Object/Array identity matches test objects.
    // Bare globals used by tracker.js are installed temporarily.
    const restore = installGlobals({
      RR_TEST: true,
      Image: ImageCtor,
      XMLHttpRequest: windowObj.XMLHttpRequest,
      Blob: windowObj.Blob,
      Event: windowObj.Event,
      Math: math,
    });

    try {
      // tracker.js ends with })(window); — the parameter supplies our mock.
      const run = new Function("window", trackerSource);
      run(windowObj);
    } finally {
      restore();
    }
  }

  runTracker();

  if (!windowObj.__rrTest) {
    for (const id of timers) {
      clearTimeout(id);
    }
    throw new Error("tracker did not export __rrTest (early return or load failure)");
  }

  return {
    window: windowObj,
    rr: windowObj.__rrTest,
    dispatch(action, ...args) {
      return windowObj[dispatchName](action, ...args);
    },
    installStub(nextConfig, nextQueue, nextDispatchName) {
      return installQueueStub(nextConfig || config, nextQueue, nextDispatchName || dispatchName);
    },
    runTracker,
    dispatchName,
    requests,
    beacons,
    observers,
    cookies: cookieJar,
    listeners,
    createElement,
    documentElement,
    body,
    cleanup() {
      for (const id of timers) {
        clearTimeout(id);
      }
      timers.clear();
    },
  };
}
