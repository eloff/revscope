// Browser support. We need to support a lot of older browsers down to IE9 and Opera.
// We target JavaScript ES5. No promises, async, const/let, arrow functions, destructuring.
!(function (window) {
  try {
    // These are variables so the script packs a little smaller
    var TRUE = true;
    var FALSE = false;
    var version = "1.26";
    var httpsProtocol = "https:";
    var cookieMaxAge = 90 * 24 * 60 * 60;

    var productName = "revscope";
    var pageviewType = "pageview";
    var crawlerType = "crawler";
    var eventType = "event";
    var metadataType = "metadata";
    var contextType = "context";
    var forgetType = "forget";
    var setConsentType = "set_consent";
    var consentStatusType = "consent_status";
    var consentGranted = "granted";
    var consentDenied = "denied";
    var consentDisabled = "disabled";
    var errorType = "error";

    var console = window.console;
    var navigator = window.navigator;
    var location = window.location;
    var document = window.document;
    var addEventListener = window.addEventListener;
    var screen = window.screen;
    var history = window.history;
    var dispatchEvent = window.dispatchEvent;
    var setTimeout = window.setTimeout;

    function warn() {
      var warningArgs = Array.prototype.slice.call(arguments);
      warningArgs.unshift(productName + ":");
      return console.warn.apply(console, warningArgs);
    }

    // Discover the queue stub: a function with .q/.a/.c. Project name is
    // config.project (or, for older installs, the dispatch_<project> suffix).
    // projectName on the function marks an already-initialized tracker.
    var rrQueue = [];
    var dispatchProperty;
    var projectName;
    var config;
    for (var windowProperty in window) {
      if (!hasOwn(window, windowProperty)) {
        continue;
      }
      var candidate = window[windowProperty];
      if (
        typeof candidate != "function" ||
        !Array.isArray(candidate.q) ||
        candidate.a !== candidate.q ||
        !isPlainObject(candidate.c)
      ) {
        continue;
      }
      var candidateProject = candidate.c.project;
      if (typeof candidateProject != "string" || !/^[a-z0-9_]{3,}$/.test(candidateProject)) {
        var legacyMatch = /^dispatch_([a-z0-9_]{3,})$/.exec(windowProperty);
        candidateProject = legacyMatch ? legacyMatch[1] : "";
      }
      if (!candidateProject) {
        continue;
      }
      // projectName on the function means a prior load already initialized.
      if (candidate.projectName) {
        return;
      }
      if (dispatchProperty) {
        return warn("Multiple stub functions found");
      }
      rrQueue = candidate.q;
      dispatchProperty = windowProperty;
      projectName = candidateProject;
      config = candidate.c;
    }
    if (!projectName) {
      return warn("Project name missing; install the stub script");
    }

    // Per-project cookie names avoid easy ad-blocker targeting of fixed names.
    var consentCookie = "rc" + projectName;
    var landingCookie = "rs" + projectName;

    var ssl = location.protocol === httpsProtocol;
    var locationHost = location.host;
    var userAgentData = navigator.userAgentData;

    var encode = encodeURIComponent;
    var decode = decodeURIComponent;
    var stringify = JSON.stringify;
    var documentElement = document.documentElement || {};
    var languageKey = "language";
    var heightSuffix = "Height";
    var scrollEventName = "scroll";
    var clickEventName = "click";
    var scrollHeightKey = scrollEventName + heightSuffix;
    var outboundClickEventName = "revscope-outbound-click";
    var offsetHeightKey = "offset" + heightSuffix;
    var clientHeightKey = "client" + heightSuffix;
    var pagehideEventName = "pagehide";

    // IE9 does not expose currentScript, but marks the executing script interactive.
    var scriptElement = document.currentScript;
    if (!scriptElement) {
      var scriptElements = document.getElementsByTagName("script");
      for (var sei = scriptElements.length - 1; sei >= 0; sei--) {
        if (scriptElements[sei].readyState === "interactive") {
          scriptElement = scriptElements[sei];
          break;
        }
      }
    }
    // Default ingest host to the host that served this script
    var scriptSrc = (scriptElement && scriptElement.src) || "in.revscope.co";
    var hostStart = scriptSrc.indexOf("://");
    var trackerHost;
    if (hostStart >= 0) {
      hostStart += 3;
    } else {
      hostStart = 0;
    }
    var hostEnd = scriptSrc.indexOf("/", hostStart);
    trackerHost = hostEnd === -1 ? scriptSrc.slice(hostStart) : scriptSrc.slice(hostStart, hostEnd);
    var trackerUrl = (config.url || trackerHost)
      .replace(/^(?!\/|.*:\/\/)/, (ssl ? httpsProtocol : "http:") + "//")
      .replace(/\/?$/, "/");

    // Create a 96bit timestamp + random unique identifier
    function createId() {
      var crypto = window.crypto || window.msCrypto;
      var i;
      // First 4 bytes: low 32 bits of Date.now() as 8 hex chars
      var id = ("0000000" + (Date.now() >>> 0).toString(16)).slice(-8);

      if (crypto && crypto.getRandomValues && typeof Uint8Array === "function") {
        /* eslint-disable-next-line */
        var bytes = new Uint8Array(8); // 8 bytes => 16 hex chars
        crypto.getRandomValues(bytes);
        for (i = 0; i < bytes.length; i++) {
          // 0x100 forces the leading digit so slice(1) leaves a zero-padded byte
          id += (0x100 | bytes[i]).toString(16).slice(1);
        }
        return id;
      }
      // Fallback for older environments: remaining 16 hex chars via Math.random
      for (i = 0; i < 16; i++) {
        id += ((Math.random() * 16) | 0).toString(16);
      }
      return id;
    }

    // Bot detection. Headless browsers send a crawler hit instead of pageviews.
    // We do more bot detection server side; this is just a first screening.
    // A fake RNG repeats the random half of createId (last 16 hex chars).
    var reportCrawler = FALSE;
    var botDetected =
      navigator.webdriver ||
      Math.random() == Math.random() ||
      createId().slice(-16) == createId().slice(-16);
    // Not window.phantom: the Phantom crypto wallet sets that on real browsers.
    var botIndicators = ["callPhantom", "_phantom", "_polypane", "_nightmare"];
    for (var bi = 0; bi < botIndicators.length; bi++) {
      var bot = botIndicators[bi];
      if (botDetected || window[bot] || window["_" + bot]) {
        reportCrawler = TRUE;
        break;
      }
    }

    function warnUserFunctionError(functionName, error) {
      warn("Error in user function " + functionName + ":", error);
    }

    function hasOwn(object, key) {
      return Object.prototype.hasOwnProperty.call(object, key);
    }

    function makeSet(keys) {
      var map = {};
      keys.forEach(function (key) {
        if (key) {
          map[key] = TRUE;
        }
      });
      return map;
    }

    function escapeRegExp(value) {
      return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    }

    function getDataAttribute(element, name) {
      return element && element.getAttribute("data-" + name);
    }

    // Real DNS names have a "."; localhost has none. Bare IPs are digits/./: only.
    function isReportableHostname(hostname) {
      return /\./.test(hostname) && !/^[\d.:]+$/.test(hostname);
    }

    // Prefer a configured hostname override for the local/IP gate so demos and
    // white-label embeds can still track from loopback. Without an override,
    // use location.hostname (no port).
    var configuredHostname = config.hostname || getDataAttribute(scriptElement, "hostname");
    if (!isReportableHostname(configuredHostname || location.hostname || "")) {
      warn("Local or IP hostname, not tracking");
      return;
    }
    var trackedHostname = configuredHostname || locationHost;

    function parseList(value) {
      return Array.isArray(value)
        ? value
        : typeof value == "string" && value.length
          ? value.split(/,\s*/)
          : [];
    }

    function isPlainObject(value) {
      return value && value.constructor === Object;
    }

    function mergeObjects() {
      var args = arguments;
      var merged = {};

      for (var index = 0; index < args.length; index++) {
        var source = args[index];

        if (isPlainObject(source)) {
          for (var key in source) {
            if (hasOwn(source, key)) {
              merged[key] = source[key];
            }
          }
        }
      }

      return merged;
    }

    function isFinite(value) {
      return typeof value === "number" && window.isFinite(value);
    }

    function getCookie(name) {
      var prefix = name + "=";
      var cookies = document.cookie ? document.cookie.split(";") : [];
      for (var index = 0; index < cookies.length; index++) {
        var cookie = cookies[index].replace(/^\s+/, "");
        if (cookie.indexOf(prefix) === 0) {
          return cookie.slice(prefix.length);
        }
      }
    }

    function getCookieDomain() {
      var hostname = location.hostname || "";
      // No leading "." needed; hostname covers its subdomains.
      return isReportableHostname(hostname) ? hostname : "";
    }

    // Called when the server says this browser may persist the session id.
    function setCookie(name, value, maxAgeSeconds) {
      var cookieValue = name + "=" + value + "; path=/; max-age=" + maxAgeSeconds;
      var domain = getCookieDomain();
      if (domain) {
        cookieValue += "; domain=" + domain;
      }
      document.cookie = cookieValue;
    }

    function isValidSessionId(value) {
      return /^[0-9a-f]{16}$/.test(value);
    }

    var sessionId = getCookie(landingCookie);
    if (sessionId && !isValidSessionId(sessionId)) {
      sessionId = undefined;
    }
    var hadSessionCookie = !!sessionId;
    // A consent value is stored as its index in this list, so the cookie code
    // and the value are always converted through the same table.
    var consentValues = [consentDenied, consentGranted, consentDisabled];
    var consentCodePattern = /^[012]$/;
    var consentCode = getCookie(consentCookie);
    var consent = consentCodePattern.test(consentCode) ? consentValues[consentCode] : undefined;
    // Anything but a granted decision means no stored session.
    if (consent && consent !== consentGranted) {
      setCookie(landingCookie, "", 0);
      sessionId = undefined;
    }
    var trackingDisabled = consent === consentDisabled;
    var hadConsentCookie = consent !== undefined;
    var consentStatusCallbacks = [];
    var consentRequirementKnown = FALSE;
    var consentStatusResult;

    // c only rides along once the visitor made an explicit choice.
    function applyConsentFlag(target) {
      if (consent === consentGranted) {
        target.c = TRUE;
      } else if (consent === consentDenied) {
        target.c = FALSE;
      }
    }

    function notifyConsentStatus(status) {
      consentRequirementKnown = TRUE;
      consentStatusResult = status;
      for (var csi = 0; csi < consentStatusCallbacks.length; csi++) {
        consentStatusCallbacks[csi](consentStatusResult);
      }
      consentStatusCallbacks = [];
    }

    function resolveConsentStatus(storageAllowed) {
      if (consentRequirementKnown) {
        return;
      }
      // granted/denied from a stored decision; otherwise required only when the
      // server disallows storage and we have no prior session.
      if (consent !== undefined) {
        notifyConsentStatus(consent);
      } else if (hadSessionCookie || storageAllowed === TRUE) {
        notifyConsentStatus("not_required");
      } else {
        notifyConsentStatus("required");
      }
    }

    function updateSessionId(nextSessionId, storageAllowed) {
      if (!nextSessionId || trackingDisabled) {
        return;
      }
      nextSessionId += "";
      if (!isValidSessionId(nextSessionId)) {
        return;
      }
      sessionId = nextSessionId;
      // Refresh the cookie whenever storage is allowed — the server does not
      // suppress this just because a session id already exists.
      if (storageAllowed && consent !== consentDenied) {
        setCookie(landingCookie, sessionId, cookieMaxAge);
      }
    }

    function getSession(callback) {
      var id = sessionId || pageId;
      if (typeof callback != "function") {
        warn("callback must be a function, not: " + callback);
        return;
      }
      callback(id);
    }

    function collectMetadata(eventContext) {
      var configuredMetadata = config.metadata;

      if (typeof configuredMetadata == "function") {
        try {
          return configuredMetadata(eventContext);
        } catch (error) {
          warnUserFunctionError("metadata", error);
        }
      }

      return isPlainObject(configuredMetadata) ? configuredMetadata : undefined;
    }

    // Send the payload as application/x-www-form-urlencoded fields.
    // Project name is the ingest URL path (/{projectName} or /{projectName}.gif).
    // The user agent is never sent, the server reads the User-Agent header.
    // The ty=context keys (tz, l, vw, vh, sw, sh, tch) are read from the device,
    // so they are only sent once the server answered storage_allowed.
    // Payload can contain the keys:
    // v=version,
    // h=hostname,
    // c=consent,
    // ty=type (pageview, crawler, engagement, event, …),
    // t=timestamp,
    // st=send_time (when the request left the browser; lets the server backdate
    //    queued hits to when they happened),
    // e=error,
    // p=path,
    // pg=page_id,
    // s=session,
    // d=duration,
    // sc=scrolled,
    // vw=viewport_width,
    // vh=viewport_height,
    // l=language,
    // sw=screen_width,
    // sh=screen_height,
    // tz=timezone,
    // r=referrer,
    // brave,
    // duck,
    // m=mobile,
    // tch=touch,
    // meta=metadata (includes URL query params),
    // ev=event,
    // batch=1,
    // items=JSON array of batched payloads
    function payloadQueryString(payload) {
      return Object.keys(payload)
        .filter(function (key) {
          return payload[key] != null;
        })
        .map(function (key) {
          return encode(key) + "=" + encode(payload[key]);
        })
        .join("&");
    }

    function supportsCrossOriginXhr() {
      try {
        return window.XMLHttpRequest && "withCredentials" in new window.XMLHttpRequest();
      } catch (error) {
        warn(error);
        return FALSE;
      }
    }

    var useXhr = supportsCrossOriginXhr();
    var formContentType = "application/x-www-form-urlencoded";

    function trackingUrl(gif) {
      return trackerUrl + projectName + (gif ? ".gif" : "");
    }

    function payloadWithRuntimeState(payload) {
      var nextPayload = mergeObjects(payload, {
        t: payload.t || Date.now(),
      });
      if (sessionId) {
        nextPayload.s = sessionId;
      }
      if (payload.ty === pageviewType && !nextPayload.h && trackedHostname) {
        nextPayload.h = trackedHostname;
      }
      return nextPayload;
    }

    function payloadBody(payload) {
      var nextPayload = payloadWithRuntimeState(payload);
      nextPayload.st = Date.now();
      return payloadQueryString(nextPayload);
    }

    function applyIngestResponse(responseText) {
      var response;
      if (!responseText) {
        return;
      }
      try {
        response = JSON.parse(responseText);
      } catch (error) {
        warn(error);
        return;
      }
      var storageAllowed = response.storage_allowed === TRUE;
      if (response.visitor_timestamp_id != null) {
        updateSessionId(response.visitor_timestamp_id, storageAllowed);
      }
      if (hasOwn(response, "storage_allowed")) {
        resolveConsentStatus(storageAllowed);
      }
      if (storageAllowed) {
        sendDeviceContext();
      }
    }

    // Guard a completion callback so it runs exactly once, no matter whether the
    // request succeeded, errored, or threw before it left the browser.
    function once(onComplete) {
      var hasCompleted = FALSE;
      return function () {
        if (hasCompleted) {
          return;
        }
        hasCompleted = TRUE;
        if (onComplete) {
          onComplete();
        }
      };
    }

    function sendXhr(body, onComplete) {
      var complete = once(onComplete);
      var xhr;

      if (!body) {
        return complete();
      }

      xhr = new window.XMLHttpRequest();
      xhr.onreadystatechange = function () {
        if (xhr.readyState === 4) {
          if (xhr.status >= 200 && xhr.status < 300) {
            applyIngestResponse(xhr.responseText);
          }
          complete();
        }
      };

      try {
        xhr.open("POST", trackingUrl(FALSE), TRUE);
        xhr.setRequestHeader("Content-Type", formContentType);
        xhr.send(body);
      } catch (error) {
        warn(error);
        complete();
      }
    }

    function sendGif(body, onComplete) {
      var image = new Image();
      // Both handlers share one guard, so whichever fires first wins.
      image.onload = image.onerror = once(onComplete);
      image.src = trackingUrl(TRUE) + "?" + body;
    }

    function sendBeacon(body) {
      if (!body || !navigator.sendBeacon) {
        return FALSE;
      }
      return navigator.sendBeacon(
        trackingUrl(FALSE),
        typeof Blob == "function" ? new Blob([body], { type: formContentType }) : body
      );
    }

    var maxBatchSize = 20;
    var batchFlushDelay = 30000;
    var batchQueue = [];
    var batchCallbacks = [];
    var batchTimer;

    function clearBatchTimer() {
      if (batchTimer) {
        clearTimeout(batchTimer);
        batchTimer = undefined;
      }
    }

    // Envelope-only fields. Both builders walk this list so a key can't land on
    // the envelope without also being stripped from items (and vice versa).
    var batchEnvelopeKeys = ["batch", "v", "h", "st", "s", "c"];

    function batchEnvelopePayload() {
      var values = {
        batch: 1,
        v: version,
        h: trackedHostname,
        // Built when the request is about to go out, so the server can measure
        // how long each item waited in the queue from its own `t`.
        st: Date.now(),
      };
      if (sessionId) {
        values.s = sessionId;
      }
      applyConsentFlag(values);
      var payload = {};
      for (var ei = 0; ei < batchEnvelopeKeys.length; ei++) {
        var key = batchEnvelopeKeys[ei];
        if (values[key] != null) {
          payload[key] = values[key];
        }
      }
      return payload;
    }

    function batchItemPayload(payload) {
      var item = payloadWithRuntimeState(payload);
      for (var ei = 0; ei < batchEnvelopeKeys.length; ei++) {
        delete item[batchEnvelopeKeys[ei]];
      }
      return item;
    }

    function batchBody(items) {
      var serializedItems;
      try {
        serializedItems = stringify(items);
      } catch (error) {
        warn(error);
        return;
      }
      return payloadQueryString(mergeObjects(batchEnvelopePayload(), { items: serializedItems }));
    }

    function completeBatchCallbacks(callbacks) {
      for (var ci = 0; ci < callbacks.length; ci++) {
        if (callbacks[ci]) {
          callbacks[ci]();
        }
      }
    }

    function flushTrackingQueue(useBeacon) {
      if (!batchQueue.length) {
        return FALSE;
      }

      clearBatchTimer();

      var queuedPayloads = batchQueue;
      var queuedCallbacks = batchCallbacks;
      var items = [];
      batchQueue = [];
      batchCallbacks = [];

      for (var qi = 0; qi < queuedPayloads.length; qi++) {
        items.push(batchItemPayload(queuedPayloads[qi]));
      }

      if (useBeacon && sendBeacon(batchBody(items))) {
        completeBatchCallbacks(queuedCallbacks);
        return TRUE;
      }

      if (useXhr) {
        sendXhr(batchBody(items), function () {
          completeBatchCallbacks(queuedCallbacks);
        });
        return TRUE;
      }

      for (var fi = 0; fi < queuedPayloads.length; fi++) {
        sendGif(payloadBody(queuedPayloads[fi]), queuedCallbacks[fi]);
      }
      return FALSE;
    }

    function discardTrackingQueue() {
      clearBatchTimer();
      var callbacks = batchCallbacks;
      batchQueue = [];
      batchCallbacks = [];
      completeBatchCallbacks(callbacks);
    }

    function enqueueTrackingRequest(payload, onComplete, flushWithBeacon) {
      if (trackingDisabled) {
        if (onComplete) {
          onComplete();
        }
        return;
      }
      if (!useXhr) {
        if (flushWithBeacon && sendBeacon(payloadBody(payload))) {
          if (onComplete) {
            onComplete();
          }
          return;
        }
        return sendTrackingRequest(payload, onComplete);
      }

      payload.t = Date.now();
      batchQueue.push(payload);
      batchCallbacks.push(onComplete);

      if (flushWithBeacon) {
        flushTrackingQueue(TRUE);
        return;
      }

      if (onComplete) {
        flushTrackingQueue(FALSE);
        return;
      }

      if (batchQueue.length === 1) {
        batchTimer = setTimeout(function () {
          flushTrackingQueue(FALSE);
        }, batchFlushDelay);
      }

      if (batchQueue.length >= maxBatchSize) {
        flushTrackingQueue(FALSE);
      }
    }

    function sendTrackingRequest(payload, onComplete) {
      if (trackingDisabled) {
        if (onComplete) {
          onComplete();
        }
        return;
      }
      var body = payloadBody(payload);
      if (useXhr) {
        sendXhr(body, onComplete);
      } else {
        sendGif(body, onComplete);
      }
    }

    if (reportCrawler) {
      warn("Bot detected, reporting crawler view");
      sendTrackingRequest({
        ty: crawlerType,
        v: version,
        h: location.hostname,
        p: location.pathname || "/",
      });
      trackingDisabled = TRUE;
    }

    // The basic fields we send on every request
    var basePayload = {
      ty: pageviewType, // overwritten as needed for other request types
      v: version,
    };

    var MAX_CONTEXT_SIZE = 4096;

    function stringifyObj(objectValue, contextName) {
      if (!isPlainObject(objectValue)) {
        return;
      }

      var serialized;
      try {
        serialized = stringify(objectValue);
      } catch (error) {
        warn(error);
        return;
      }

      if (!serialized) {
        return;
      }

      if (serialized.length > MAX_CONTEXT_SIZE) {
        warn(contextName + " exceeds max size of " + MAX_CONTEXT_SIZE);
        return;
      }

      return serialized;
    }

    function serializeMetadataPayload(metadataValue) {
      return stringifyObj(metadataValue, "metadata");
    }

    function metadataValuesEqual(left, right) {
      if (left === right) {
        return TRUE;
      }
      try {
        return stringify(left) === stringify(right);
      } catch (error) {
        warn(error);
        return FALSE;
      }
    }

    function diffMetadata(metadataValue, previousMetadata) {
      var diff = {};
      if (!isPlainObject(metadataValue)) {
        return diff;
      }

      for (var key in metadataValue) {
        if (
          hasOwn(metadataValue, key) &&
          (!hasOwn(previousMetadata, key) ||
            !metadataValuesEqual(metadataValue[key], previousMetadata[key]))
        ) {
          diff[key] = metadataValue[key];
        }
      }
      return diff;
    }

    function serializeMetadataDiff(metadataValue, previousMetadata) {
      var diff = diffMetadata(metadataValue, previousMetadata);
      if (!Object.keys(diff).length) {
        return;
      }
      var serialized = serializeMetadataPayload(diff);
      return serialized === "{}" ? undefined : serialized;
    }

    function takeMetadataDiff() {
      var diff = diffMetadata(pageMetadata, lastSentMetadata);
      if (!Object.keys(diff).length) {
        return;
      }
      var serialized = serializeMetadataPayload(diff);
      if (serialized === "{}") {
        return;
      }
      lastSentMetadata = mergeObjects(lastSentMetadata, diff);
      return serialized;
    }

    var reportError =
      config.reportError ||
      function (error) {
        if (!error) {
          return;
        }
        error = error.toString();
        error = error.stack ? error + " " + error.stack : error;
        warn(error);
        sendTrackingRequest(
          mergeObjects(basePayload, {
            ty: errorType,
            e: error,
            p: location.pathname,
            pg: pageId,
          }),
          undefined
        );
      };

    addEventListener(
      errorType,
      function (event) {
        if (event.filename && event.filename.indexOf(trackerHost) !== -1) {
          reportError(event.message);
        }
      },
      FALSE
    );

    var pageEngagementStartedAt = Date.now();
    var maxScrollDepth = 0;
    var trackingMode = config.mode || getDataAttribute(scriptElement, "mode");
    var ignoredPages = parseList(
      config.ignorePages || getDataAttribute(scriptElement, "ignore-pages")
    ).filter(Boolean);
    // One anchored alternation for every ignore pattern, with "*" as a wildcard.
    // Built once because getTrackedPath runs on every pageview.
    var ignoredPagePattern = ignoredPages.length
      ? new RegExp(
          "^(?:" +
            ignoredPages
              .map(function (page) {
                var ignoredPath = page.charAt(0) === "/" ? page : "/" + page;
                return escapeRegExp(ignoredPath).replace(/\\\*/g, ".*");
              })
              .join("|") +
            ")$",
          "i"
        )
      : undefined;
    var includedParameters = parseList(
      config.includeParams || getDataAttribute(scriptElement, "include-params")
    );
    var allowedParameterMap = makeSet(
      includedParameters.concat([
        "ref",
        "source",
        "medium",
        "content",
        "term",
        "campaign",
        "checkout_id",
        "order_id",
        "user_id",
        "user_email",
        "attribution_key",
      ])
    );
    var clickIdParameterPattern = new RegExp(
      "^(gclid|gbraid|wbraid|dclid|gclsrc|fbclid|msclkid|" +
        "li_fat_id|rdt_cid|ttclid|twclid|sccid|epik|tblci|" +
        "ob_click_id|obclid|dicbo|vmcid|yclid|click_id|clickid|" +
        "cid|subid|sub_id|aff_click_id)$",
      "i"
    );

    function extractQueryParams() {
      // One key=value pair. The key runs to the first "=", so any later "=" stays
      // part of the value; pairs without a "=" never match.
      var pairPattern = /([^&=]+)=([^&]*)/g;
      var queryString = location.search.slice(1);
      var queryParams;
      var match;

      while ((match = pairPattern.exec(queryString)) !== null) {
        var rawKey;
        try {
          rawKey = decode(match[1]);
        } catch (_error) {
          rawKey = match[1];
        }
        // utm_/rr_ keys are always kept, under the stripped name.
        var key = rawKey.replace(/^(?:utm_|rr_)/, "");
        if (
          key === rawKey &&
          !hasOwn(allowedParameterMap, key) &&
          !clickIdParameterPattern.test(key)
        ) {
          continue;
        }
        if (!queryParams) {
          queryParams = {};
        }
        try {
          queryParams[key] = decode(match[2]);
        } catch (_error) {
          queryParams[key] = match[2];
        }
      }

      return queryParams;
    }

    var pathRewriter = config.pathRewriter;
    if (pathRewriter !== undefined && typeof pathRewriter != "function") {
      warn("pathRewriter must be a function");
      pathRewriter = undefined;
    }

    // Referrers are reduced to hostname only before being sent. Scheme, mobile,
    // and www-like prefixes are stripped so reports group cleanly.
    var normalizedReferrer = (document.referrer || "").replace(
      /^(?:https?:\/\/)?(?:(?:m|l|w{2,3}(?:[0-9]+)?)\.)?([^/?#]+).*$/,
      "$1"
    );
    // If the referrer is internal, don't keep it, we don't need it
    if (normalizedReferrer.indexOf(locationHost) !== -1) {
      normalizedReferrer = "";
    }

    function isTouchDevice() {
      return (
        navigator.maxTouchPoints > 0 ||
        (window.matchMedia && window.matchMedia("(pointer: coarse)").matches)
      );
    }

    var deviceContextSent = FALSE;

    // Timezone, language, screen size and touch support are read from the
    // visitor's device, which ePrivacy Article 5(3) treats like a cookie. We
    // therefore read nothing until the server has confirmed storage is allowed
    // (or the visitor granted consent), and then send it as its own hit. Rides
    // along with the next batch, which the pagehide handler flushes for visitors
    // who leave right away.
    function sendDeviceContext() {
      if (deviceContextSent || trackingDisabled || !pageId) {
        return;
      }
      deviceContextSent = TRUE;

      var contextPayload = mergeObjects(basePayload, {
        ty: contextType,
        pg: pageId,
        tch: isTouchDevice(),
        vw: viewportWidth() || null,
        vh: viewportHeight() || null,
      });
      if (screen) {
        contextPayload.sw = screen.width;
        contextPayload.sh = screen.height;
      }
      if (navigator[languageKey]) {
        contextPayload.l = navigator[languageKey];
      }
      try {
        contextPayload.tz = window.Intl.DateTimeFormat().resolvedOptions().timeZone;
      } catch (error) {
        warn(error);
      }

      enqueueTrackingRequest(contextPayload);
    }

    var firstRequestPayload = mergeObjects(basePayload, {
      h: trackedHostname,
      r: normalizedReferrer || undefined,
    });
    if (navigator.brave) {
      firstRequestPayload.brave = TRUE;
    }
    if (navigator._duckduckgoloader_) {
      firstRequestPayload.duck = TRUE;
    }
    if (userAgentData) {
      firstRequestPayload.m = !!userAgentData.mobile;
    }

    var currentTrackedPath;
    var pageId;
    var pageMetadata = {};
    var lastSentMetadata = {};
    var hiddenDuration = 0;
    // A tab opened in the background is hidden from the start and fires no hide event.
    var pageHiddenAt = pageEngagementStartedAt;
    var idleThresholdMs = 2 * 60 * 1000;
    var idleDuration = 0;
    var idleStartedAt;
    var lastScrollActivityAt = pageEngagementStartedAt;

    function updateIdleState(now) {
      if (idleStartedAt === undefined && now - lastScrollActivityAt >= idleThresholdMs) {
        idleStartedAt = lastScrollActivityAt;
      }
    }

    function idlePeriodStart() {
      return idleStartedAt < pageEngagementStartedAt ? pageEngagementStartedAt : idleStartedAt;
    }

    function finishIdlePeriod(now) {
      var startedAt;
      updateIdleState(now);
      if (idleStartedAt === undefined) {
        return;
      }

      startedAt = idlePeriodStart();
      if (now > startedAt) {
        idleDuration += now - startedAt;
      }
      idleStartedAt = undefined;
    }

    function calculateIdleDuration(now) {
      var duration = idleDuration;
      var startedAt;
      updateIdleState(now);
      if (idleStartedAt !== undefined) {
        startedAt = idlePeriodStart();
        if (now > startedAt) {
          duration += now - startedAt;
        }
      }
      return duration;
    }

    function recordScrollActivity() {
      var now = Date.now();
      finishIdlePeriod(now);
      lastScrollActivityAt = now;
    }

    function appendEngagementMetrics(forceImageRequest) {
      if (trackingDisabled || !pageId) {
        return;
      }
      var now = Date.now();
      // A navigation or pagehide can land mid-way through a hidden period, whose
      // elapsed time belongs to this hit and the rest to the next one. Moving the
      // activity clock keeps that time from also counting as idle.
      if (document.hidden) {
        hiddenDuration += now - pageHiddenAt;
        pageHiddenAt = now;
        lastScrollActivityAt = now;
      }
      var appendPayload = mergeObjects(basePayload, {
        ty: "engagement",
        pg: pageId,
      });

      appendPayload.d = Math.max(
        0,
        Math.round(
          (now - pageEngagementStartedAt - hiddenDuration - calculateIdleDuration(now)) / 1000
        )
      );

      hiddenDuration = 0;
      idleDuration = 0;
      pageEngagementStartedAt = now;
      if (idleStartedAt !== undefined) {
        idleStartedAt = now;
      }

      appendPayload.sc = Math.max(0, maxScrollDepth, calculateScrollDepth());

      // sendBeacon is preferred during unload/pagehide because image requests
      // may be canceled while the browser is closing or navigating away.
      enqueueTrackingRequest(appendPayload, undefined, !forceImageRequest);
    }

    addEventListener(
      "visibilitychange",
      function () {
        var now = Date.now();
        if (document.hidden) {
          finishIdlePeriod(now);
          pageHiddenAt = now;
          if (!("on" + pagehideEventName in window)) {
            appendEngagementMetrics();
          }
        } else {
          hiddenDuration += now - pageHiddenAt;
          lastScrollActivityAt = now;
          scheduleVisibleCheck();
        }
      },
      FALSE
    );

    addEventListener(
      pagehideEventName,
      function () {
        appendEngagementMetrics();
      },
      FALSE
    );

    var documentBody = document.body || {};

    function calculateScrollDepth() {
      try {
        var viewportHeight = documentElement[clientHeightKey] || 0;
        var keys = [scrollHeightKey, offsetHeightKey, clientHeightKey];
        var documentHeight = keys
          .map(function (key) {
            return documentBody[key];
          })
          .concat(
            keys.map(function (key) {
              return documentElement[key];
            })
          )
          .filter(isFinite)
          .reduce(function (max, value) {
            return Math.max(max, value);
          }, 0);

        var viewportBottom = (documentElement.scrollTop || 0) + viewportHeight;
        var depthPercent = (100 * viewportBottom) / documentHeight;
        // Round to the nearest 5% and cap to 100%
        return Math.min(100, 5 * Math.round(depthPercent / 5));
      } catch (error) {
        warn(error);
        return 0;
      }
    }

    addEventListener("load", function () {
      maxScrollDepth = calculateScrollDepth();

      addEventListener(
        scrollEventName,
        function () {
          if (maxScrollDepth < calculateScrollDepth()) {
            maxScrollDepth = calculateScrollDepth();
          }
        },
        FALSE
      );
    });

    var lastIgnoredPath;
    function getTrackedPath(pathOverride) {
      var path = "";

      try {
        path = pathOverride || decode(location.pathname);
      } catch (error) {
        warn(error);
      }

      if (typeof pathRewriter == "function") {
        try {
          path = pathRewriter.call(window, { path: path }) || path;
        } catch (error) {
          warnUserFunctionError("path", error);
        }
      }

      if (ignoredPagePattern && ignoredPagePattern.test(path)) {
        // This runs on every history call, which some apps make per scroll or keystroke.
        if (lastIgnoredPath != path) {
          warn("Ignoring " + path);
          lastIgnoredPath = path;
        }
        return;
      }
      lastIgnoredPath = undefined;

      if (trackingMode == "hash" && location.hash) {
        path += location.hash.split("?")[0];
      }

      return path;
    }

    if (!document.doctype) {
      warn("Add DOCTYPE for accurate screen/viewport size");
    }

    // Pageviews collect stable page context first, then send a compact request.
    function trackPageview(isHistoryNavigation, pathOverride) {
      if (trackingDisabled) {
        return;
      }
      var path = getTrackedPath(pathOverride);
      var isInitialRequest = pageId === undefined;

      if (path && currentTrackedPath != path) {
        if (isHistoryNavigation && pageId !== undefined) {
          appendEngagementMetrics(TRUE);
        }
        pageId = createId();
        var payload = mergeObjects(isInitialRequest ? firstRequestPayload : basePayload, {
          pg: pageId,
        });

        currentTrackedPath = path;
        payload.p = path;
        applyConsentFlag(payload);

        pageMetadata = mergeObjects(
          extractQueryParams(),
          collectMetadata({
            type: pageviewType,
            path: payload.p,
          })
        );
        var serializedMetadata = takeMetadataDiff();
        if (serializedMetadata) {
          payload.meta = serializedMetadata;
        }

        if (isInitialRequest) {
          sendTrackingRequest(payload);
        } else {
          enqueueTrackingRequest(payload);
        }

        refreshEventTargets();
      }
    }

    function trackPageviewOnNav() {
      trackPageview(1);
    }

    function trackHistoryMethod(eventName) {
      var originalMethod = history[eventName];

      history[eventName] = function () {
        var event;
        var historyArgs = Array.prototype.slice.call(arguments);
        var result = originalMethod.apply(this, historyArgs);

        if (typeof Event == "function") {
          event = new Event(eventName);
        } else {
          event = document.createEvent("Event");
          event.initEvent(eventName, TRUE, TRUE);
        }

        event.arguments = historyArgs;
        dispatchEvent(event);
        return result;
      };

      addEventListener(eventName, trackPageviewOnNav, FALSE);
    }

    if (history && history.pushState && Event && dispatchEvent) {
      trackHistoryMethod("pushState");
      // Client-side redirects use replaceState so Back skips the redirecting URL.
      // Apps also call it for query and state updates. trackPageview skips those
      // unless a pathRewriter puts the query in the tracked path.
      trackHistoryMethod("replaceState");
      addEventListener("popstate", trackPageviewOnNav, FALSE);
    }

    if (trackingMode == "hash" && "onhashchange" in window) {
      addEventListener("hashchange", trackPageviewOnNav, FALSE);
    }

    // Declarative tracking via data-revscope-click, data-revscope-beacon,
    // data-revscope-visible, and data-revscope-metadata. Event attribute values
    // are "<event_name> [key=value ...]". data-revscope-metadata is key=value
    // pairs only (no event name) and applies like the metadata command.
    // data-revscope-beacon is click tracking that always flushes via sendBeacon.
    // <a> tags carrying an id or name but no href act as implicit fragment
    // targets and fire "a_{id|name}" when visible.
    var dataPrefix = "revscope-";
    var revscopeClickAttribute = dataPrefix + "click";
    var revscopeBeaconAttribute = dataPrefix + "beacon";
    var revscopeVisibleAttribute = dataPrefix + "visible";
    var revscopeMetadataAttribute = dataPrefix + "metadata";
    var clickCooldownMs = 100;
    var visibleCheckThrottleMs = 100;
    var domScanThrottleMs = 250;
    var domScanPollMs = 2000;
    var eventTrackingReady = FALSE;
    var domWatchStarted = FALSE;
    var visibleWatchers = [];
    var firedVisibleNames = {};
    var clickFiredAt = {};
    var appliedDeclarativeMetadata = {};
    var pendingSchedules = {};

    // "plan=growth billing='pro annual'" ->
    // { plan: "growth", billing: "pro annual" }.
    function parseDeclarativeMetadata(spec) {
      if (typeof spec != "string") {
        return;
      }

      // One key=value pair: the value may be double quoted, single quoted, or
      // bare, and an unterminated quote runs to the end of the spec. Leading
      // whitespace is part of the match so a gap reveals a malformed pair.
      var pair = /\s*([^\s=]+)=(?:"([^"]*)"?|'([^']*)'?|(\S*))/g;
      var metadata;
      var match;
      var value;
      var next = 0;

      while ((match = pair.exec(spec)) !== null) {
        if (match.index !== next) {
          break;
        }
        next = pair.lastIndex;
        value = match[2];
        if (value == null) {
          value = match[3];
        }
        if (value == null) {
          value = match[4];
        }
        if (!metadata) {
          metadata = {};
        }
        metadata[match[1]] = value;
      }

      return metadata;
    }

    // "buy plan=growth billing='pro annual'" ->
    // { name: "buy", metadata: { plan: "growth", billing: "pro annual" } }.
    function parseDeclarativeValue(spec) {
      if (typeof spec != "string") {
        return;
      }

      var match = /^\s*(\S+)/.exec(spec);
      if (!match || match[1].indexOf("=") !== -1) {
        return;
      }

      return {
        name: match[1],
        metadata: parseDeclarativeMetadata(spec.slice(match[0].length)),
      };
    }

    // Resolve data-revscope-beacon / data-revscope-click on an element into a
    // click spec. data-revscope-beacon wins when both are present.
    function getClickSpec(element) {
      var beaconValue = getDataAttribute(element, revscopeBeaconAttribute);
      var parsed;
      if (beaconValue != null) {
        parsed = parseDeclarativeValue(beaconValue);
        if (parsed) {
          parsed.beacon = TRUE;
          return parsed;
        }
      }
      var clickValue = getDataAttribute(element, revscopeClickAttribute);
      if (clickValue != null) {
        return parseDeclarativeValue(clickValue);
      }
    }

    // True when this click's browser default is a same-tab navigation that may
    // unload the page. SPA routers that preventDefault are skipped so we do not
    // force sendBeacon for soft navigations; explicit data-revscope-beacon still does.
    function looksLikeSameTabNavigation(event, fromNode) {
      if (!event || event.defaultPrevented) {
        return FALSE;
      }
      if (
        event.metaKey ||
        event.ctrlKey ||
        event.shiftKey ||
        event.altKey ||
        (event.button != null && event.button !== 0)
      ) {
        return FALSE;
      }

      var anchor = closestLinkElement(fromNode);
      var href;
      var target;
      if (!anchor) {
        return FALSE;
      }
      if (anchor.getAttribute("download") != null) {
        return FALSE;
      }
      target = (anchor.getAttribute("target") || "").toLowerCase();
      if (target && target !== "_self") {
        return FALSE;
      }
      href = anchor.getAttribute("href");
      if (href == null) {
        return FALSE;
      }
      href = href.replace(/^\s+|\s+$/g, "");
      if (!href || href.charAt(0) === "#") {
        return FALSE;
      }
      if (/^javascript:/i.test(href)) {
        return FALSE;
      }
      return TRUE;
    }

    function viewportWidth() {
      return Math.max(documentElement.clientWidth || 0, window.innerWidth || 0) || 0;
    }

    function viewportHeight() {
      return Math.max(documentElement[clientHeightKey] || 0, window.innerHeight || 0) || 0;
    }

    function computedStyleFor(node) {
      try {
        if (window.getComputedStyle) {
          return window.getComputedStyle(node);
        }
      } catch (error) {
        warn(error);
      }
      return node.currentStyle;
    }

    // CSS-hidden if the target or any ancestor is display:none, visibility
    // hidden/collapse, or carries the boolean hidden attribute.
    function isElementHidden(element) {
      var node = element;
      while (node && node.nodeType === 1) {
        if (node.getAttribute && node.getAttribute("hidden") != null) {
          return TRUE;
        }
        var style = computedStyleFor(node);
        if (
          style &&
          (style.display === "none" ||
            style.visibility === "hidden" ||
            style.visibility === "collapse")
        ) {
          return TRUE;
        }
        node = node.parentNode;
      }
      return FALSE;
    }

    function isElementInDocument(element) {
      return documentElement.contains ? documentElement.contains(element) : TRUE;
    }

    function isElementVisible(element) {
      if (document.hidden || !isElementInDocument(element) || isElementHidden(element)) {
        return FALSE;
      }

      var rect = element.getBoundingClientRect();
      var vw = viewportWidth();
      var vh = viewportHeight();
      var visibleWidth = Math.min(rect.right, vw) - Math.max(rect.left, 0);
      var visibleHeight = Math.min(rect.bottom, vh) - Math.max(rect.top, 0);
      var elementArea = rect.width * rect.height;

      // Zero-area targets (e.g. named anchors) count as visible whenever their
      // box intersects the viewport at all.
      if (!elementArea) {
        return rect.bottom >= 0 && rect.top <= vh && rect.right >= 0 && rect.left <= vw;
      }

      if (visibleWidth <= 0 || visibleHeight <= 0) {
        return FALSE;
      }

      var visibleArea = visibleWidth * visibleHeight;

      // Elements taller than the viewport can never reach 50% on-screen area,
      // so they qualify on a lower area threshold or by filling half the height.
      if (rect.height > vh) {
        return visibleArea >= 0.3 * elementArea || visibleHeight >= 0.5 * vh;
      }

      return visibleArea >= 0.5 * elementArea;
    }

    // Returns TRUE at most once per cooldown window for a given key, recording
    // the fire time. Throttles repeated click-driven events (including the
    // click + auxclick pair) to one send per cooldown.
    function clickCooldownElapsed(key) {
      var now = Date.now();
      var lastFiredAt = clickFiredAt[key];
      if (lastFiredAt && now - lastFiredAt < clickCooldownMs) {
        return FALSE;
      }
      clickFiredAt[key] = now;
      return TRUE;
    }

    // Clicks fire every time, throttled per event name to one per cooldown.
    function fireClickEvent(eventName, options) {
      if (clickCooldownElapsed(eventName)) {
        trackEvent(eventName, options);
      }
    }

    function findAncestor(node, test) {
      while (node && node !== document) {
        if (test(node)) {
          return node;
        }
        node = node.parentNode;
      }
    }

    function closestEventElement(node) {
      return findAncestor(node, function (el) {
        return (
          el.getAttribute &&
          (getDataAttribute(el, revscopeClickAttribute) != null ||
            getDataAttribute(el, revscopeBeaconAttribute) != null)
        );
      });
    }

    function handleTrackedClick(event) {
      var target = event.target || event.srcElement;
      var element = closestEventElement(target);
      var spec;
      var options;
      if (!element) {
        return;
      }
      spec = getClickSpec(element);
      if (!spec) {
        return;
      }
      options = {};
      if (spec.metadata) {
        options.metadata = spec.metadata;
      }
      if (spec.beacon || looksLikeSameTabNavigation(event, target)) {
        options.beacon = TRUE;
      }
      fireClickEvent(spec.name, options);
    }

    function closestLinkElement(node) {
      return findAncestor(node, function (el) {
        return el.nodeName === "A" && el.getAttribute && el.getAttribute("href") != null;
      });
    }

    // Resolve the exit destination for a clicked link, or undefined when the
    // link stays on this site / is not an exit we track. http(s) links to a
    // different hostname return that hostname; mailto: links return
    // "mailto:" + the email domain (the prefix is preserved on purpose).
    function outboundDestination(anchor) {
      var protocol = anchor.protocol;
      var host;
      var match;

      if (protocol === "http:" || protocol === "https:") {
        host = (anchor.hostname || "").toLowerCase();
        if (!host || host === (location.hostname || "").toLowerCase()) {
          return;
        }
        return host;
      }

      if (protocol === "mailto:") {
        // Greedy [^?,]*@ picks the last @ before ?/, — same as lastIndexOf.
        match = /^mailto:[^?,]*@([^?,]*)/i.exec(anchor.getAttribute("href") || "");
        return "mailto:" + ((match && match[1]) || "").toLowerCase();
      }
    }

    // Best-effort outbound link tracking, on by default. A click on a link that
    // leaves the site fires an "revscope-outbound-click" event whose metadata records the
    // destination. Delivered via sendBeacon at click time so it lands before
    // the page unloads. Shared with auxclick to catch middle/new-tab opens.
    function handleOutboundClick(event) {
      if (event.defaultPrevented) {
        return;
      }
      var anchor = closestLinkElement(event.target || event.srcElement);
      if (!anchor) {
        return;
      }
      var to = outboundDestination(anchor);
      if (to && clickCooldownElapsed(outboundClickEventName + ":" + to)) {
        trackEvent(outboundClickEventName, { metadata: { to: to }, beacon: TRUE });
      }
    }

    function collectVisibleWatchers() {
      visibleWatchers = [];
      if (!document.querySelectorAll) {
        // No standard modern browser supports data-* attributes but lacks querySelectorAll.
        return;
      }

      var marked = document.querySelectorAll("[data-" + revscopeVisibleAttribute + "]");
      for (var mi = 0; mi < marked.length; mi++) {
        var spec = parseDeclarativeValue(getDataAttribute(marked[mi], revscopeVisibleAttribute));
        if (spec) {
          visibleWatchers.push({
            el: marked[mi],
            name: spec.name,
            metadata: spec.metadata,
          });
        }
      }

      // Named anchors without an href are implicit visibility targets.
      var anchors = document.getElementsByTagName("a");
      for (var ai = 0; ai < anchors.length; ai++) {
        var anchor = anchors[ai];
        if (anchor.getAttribute("href") == null) {
          var anchorRef = anchor.id || anchor.getAttribute("name");
          if (anchorRef) {
            visibleWatchers.push({ el: anchor, name: "a_" + anchorRef });
          }
        }
      }
    }

    // Scan data-revscope-metadata elements and apply them like the metadata
    // command: merge into page metadata and enqueue only changed keys.
    function applyDeclarativeMetadata() {
      if (!document.querySelectorAll) {
        return;
      }

      var marked = document.querySelectorAll("[data-" + revscopeMetadataAttribute + "]");
      var merged;
      for (var mi = 0; mi < marked.length; mi++) {
        var parsed = parseDeclarativeMetadata(
          getDataAttribute(marked[mi], revscopeMetadataAttribute)
        );
        if (parsed) {
          merged = mergeObjects(merged, parsed);
        }
      }
      // Rescans re-read the whole document, so only keys whose declarative
      // value changed since the last scan are applied. Otherwise a rescan would
      // revert keys the site has since set through the metadata command.
      var changed = diffMetadata(merged, appliedDeclarativeMetadata);
      if (Object.keys(changed).length) {
        appliedDeclarativeMetadata = mergeObjects(appliedDeclarativeMetadata, changed);
        updateMetadata(changed);
      }
    }

    function checkVisibleWatchers() {
      if (!visibleWatchers.length || document.hidden) {
        return;
      }

      var remaining = [];
      for (var wi = 0; wi < visibleWatchers.length; wi++) {
        var watcher = visibleWatchers[wi];
        if (firedVisibleNames[watcher.name]) {
          continue;
        }
        if (isElementVisible(watcher.el)) {
          firedVisibleNames[watcher.name] = TRUE;
          trackEvent(watcher.name, watcher.metadata ? { metadata: watcher.metadata } : undefined);
        } else {
          remaining.push(watcher);
        }
      }
      visibleWatchers = remaining;
    }

    // A full pass over the declarative markup. Both halves are idempotent, so
    // re-running only picks up elements that appeared since the last pass.
    function scanDeclarativeMarkup() {
      collectVisibleWatchers();
      checkVisibleWatchers();
      applyDeclarativeMetadata();
    }

    // Run task on the trailing edge of a delay window, coalescing calls made
    // while a run is already pending.
    function scheduleThrottled(key, delayMs, task) {
      if (pendingSchedules[key]) {
        return;
      }
      pendingSchedules[key] = TRUE;
      setTimeout(function () {
        pendingSchedules[key] = FALSE;
        task();
      }, delayMs);
    }

    function scheduleVisibleCheck() {
      scheduleThrottled("visible", visibleCheckThrottleMs, checkVisibleWatchers);
    }

    function scheduleDeclarativeScan() {
      scheduleThrottled("scan", domScanThrottleMs, scanDeclarativeMarkup);
    }

    // Attribute records already passed attributeFilter, so they always matter.
    // Text churn (counters, clocks) arrives as childList records carrying no
    // elements, and is not worth a scan.
    function mutationsNeedScan(records) {
      for (var ri = 0; ri < records.length; ri++) {
        if (records[ri].type !== "childList") {
          return TRUE;
        }
        var added = records[ri].addedNodes;
        for (var ni = 0; ni < added.length; ni++) {
          if (added[ni].nodeType === 1) {
            return TRUE;
          }
        }
      }
      return FALSE;
    }

    // Declarative markup can appear long after the first scan (lazy loading,
    // client-rendered views, modals), so keep watching the document for it.
    // MutationObserver only wakes us on relevant changes; browsers without it
    // (IE9/IE10) get a periodic full scan instead.
    function watchDeclarativeMarkup() {
      if (domWatchStarted) {
        return;
      }
      domWatchStarted = TRUE;

      var Observer = window.MutationObserver || window.WebKitMutationObserver;
      if (Observer) {
        new Observer(function (records) {
          if (mutationsNeedScan(records)) {
            scheduleDeclarativeScan();
          }
        }).observe(document, {
          childList: TRUE,
          subtree: TRUE,
          attributeFilter: [
            "data-" + revscopeVisibleAttribute,
            "data-" + revscopeMetadataAttribute,
          ],
        });
        return;
      }

      pollDeclarativeMarkup();
    }

    function pollDeclarativeMarkup() {
      setTimeout(function () {
        scanDeclarativeMarkup();
        pollDeclarativeMarkup();
      }, domScanPollMs);
    }

    function handleTrackedScroll() {
      recordScrollActivity();
      scheduleVisibleCheck();
    }

    function whenDomReady(callback) {
      if (/^(?:interactive|complete|loaded)$/.test(document.readyState)) {
        callback();
      } else {
        addEventListener("DOMContentLoaded", callback, FALSE);
      }
    }

    function setupEventTracking() {
      if (eventTrackingReady) {
        return;
      }
      eventTrackingReady = TRUE;
      addEventListener(clickEventName, handleTrackedClick, FALSE);
      addEventListener(clickEventName, handleOutboundClick, FALSE);
      addEventListener("auxclick", handleOutboundClick, FALSE);
      addEventListener(scrollEventName, handleTrackedScroll, FALSE);
      addEventListener("resize", scheduleVisibleCheck, FALSE);
      addEventListener("load", scheduleVisibleCheck, FALSE);
    }

    // Re-scan marked elements when a new page id begins. Visible events fire at
    // most once per page id, so a fresh scan re-arms them after SPA navigations.
    // Declarative metadata is re-applied the same way as the metadata command;
    // the new page id starts with none of it applied.
    function refreshEventTargets() {
      setupEventTracking();
      firedVisibleNames = {};
      appliedDeclarativeMetadata = {};
      whenDomReady(function () {
        scanDeclarativeMarkup();
        watchDeclarativeMarkup();
      });
    }

    var actionHandlers = {};
    actionHandlers[eventType] = trackEvent;
    actionHandlers[metadataType] = updateMetadata;
    actionHandlers[setConsentType] = setConsent;
    actionHandlers[consentStatusType] = consentStatus;
    actionHandlers.get_session = getSession;

    function dispatchCommand(command) {
      if (!Array.isArray(command) || !command.length) {
        warn("command must be a non-empty array");
        return;
      }

      var action = command[0];
      var actionHandler = actionHandlers[action];
      if (actionHandler) {
        actionHandler.apply(null, command.slice(1));
        return;
      }

      warn("unknown action: " + action);
    }

    function dispatch(action) {
      var command = [action];
      for (var argumentIndex = 1; argumentIndex < arguments.length; argumentIndex++) {
        command.push(arguments[argumentIndex]);
      }
      dispatchCommand(command);
    }

    dispatch.q = [];
    dispatch.a = dispatch.q;
    dispatch.c = config;
    dispatch.projectName = projectName;
    window[dispatchProperty] = dispatch;

    var queuedCalls = rrQueue.slice(0);
    var initialPageviewStarted = FALSE;
    for (var qci = 0; qci < queuedCalls.length; qci++) {
      var queuedConsentCall = queuedCalls[qci];
      if (Array.isArray(queuedConsentCall) && queuedConsentCall[0] === setConsentType) {
        setConsent(queuedConsentCall[1]);
      }
    }
    initialPageviewStarted = TRUE;
    trackPageview();

    // Replay every pre-load command except the consent calls already applied
    // before the initial pageview.
    for (var qi = 0; qi < queuedCalls.length; qi++) {
      var queuedCall = queuedCalls[qi];
      if (Array.isArray(queuedCall) && queuedCall[0] !== setConsentType) {
        dispatchCommand(queuedCall);
      }
    }

    // Public event tracking mirrors pageview privacy rules: events on the first
    // page can inherit attribution context, later SPA events get less context.
    function trackEvent(eventName, options) {
      options = options || {};
      var callback = options.callback;

      if (trackingDisabled) {
        if (typeof callback === "function") {
          callback();
        }
        return;
      }
      if (!eventName || typeof eventName !== "string") {
        warn("event names must be strings, not: " + eventName);
        if (typeof callback === "function") {
          callback();
        }
        return;
      }
      // Replace runs of non-alphanumeric chars with a single dash
      // and remove leading and trailing dashes
      eventName = eventName.replace(/[^a-z0-9]+/gi, "-").replace(/(^-|-$)/g, "");
      if (!eventName) {
        warn("invalid event name");
        if (typeof callback === "function") {
          callback();
        }
        return;
      }
      if (eventName.length > 100) {
        warn("event name too long: " + eventName);
        eventName = eventName.slice(0, 100);
      }

      var metadata = serializeMetadataPayload(options.metadata);
      if (callback != null && typeof callback != "function") {
        warn("callback must be a function, not: " + callback);
        callback = undefined;
      }

      var eventPayload = mergeObjects(basePayload, {
        ty: eventType,
        ev: eventName,
        pg: pageId,
        meta: metadata,
      });
      // options.beacon flushes the queue synchronously via sendBeacon, used for
      // events (e.g. exit links) that may immediately precede a navigation.
      enqueueTrackingRequest(eventPayload, callback, options.beacon);
    }

    function updateMetadata(metadataValue) {
      if (trackingDisabled) {
        return;
      }
      if (!isPlainObject(metadataValue)) {
        warn("metadata must be an object");
        return;
      }
      if (!pageId) {
        warn("metadata requires a tracked page");
        return;
      }

      pageMetadata = mergeObjects(pageMetadata, metadataValue);
      var serializedMetadata = takeMetadataDiff();
      if (!serializedMetadata) {
        return;
      }

      enqueueTrackingRequest(
        mergeObjects(basePayload, {
          ty: metadataType,
          pg: pageId,
          meta: serializedMetadata,
        })
      );
    }

    function setConsent(nextConsent) {
      if (consentValues.indexOf(nextConsent) < 0) {
        warn("consent must be granted, denied, or disabled, not: " + nextConsent);
        return;
      }

      var wasTrackingDisabled = trackingDisabled;
      if (nextConsent === consentDisabled) {
        if (!wasTrackingDisabled) {
          // Drop pending analytics, then forget this visitor while the session
          // id is still available — before cookies and in-memory identity clear.
          discardTrackingQueue();
          if (sessionId) {
            enqueueTrackingRequest(
              mergeObjects(basePayload, {
                ty: forgetType,
                pg: pageId,
                s: sessionId,
              })
            );
            flushTrackingQueue(FALSE);
          }
        }
        trackingDisabled = TRUE;
      } else {
        if (!wasTrackingDisabled) {
          flushTrackingQueue(FALSE);
        }
        trackingDisabled = FALSE;
      }

      consent = nextConsent;
      setCookie(consentCookie, consentValues.indexOf(consent), cookieMaxAge);
      notifyConsentStatus(consent);

      if (consent !== consentGranted) {
        setCookie(landingCookie, "", 0);
        sessionId = undefined;
      }
      if (trackingDisabled) {
        pageId = undefined;
        currentTrackedPath = undefined;
        pageMetadata = {};
        lastSentMetadata = {};
      } else if (wasTrackingDisabled && initialPageviewStarted) {
        // The pageview response carries storage_allowed, so this path does not
        // need to write the cookie or ask for device context itself.
        trackPageview();
      } else if (consent === consentGranted) {
        // Explicit consent is a lawful basis on its own: refresh the session
        // cookie if we already have an id, and read the device if we skipped it.
        if (sessionId) {
          setCookie(landingCookie, sessionId, cookieMaxAge);
        }
        sendDeviceContext();
      }
    }

    function consentStatus(callback) {
      if (typeof callback != "function") {
        warn("callback must be a function, not: " + callback);
        return;
      }
      if (consentRequirementKnown) {
        callback(consentStatusResult);
        return;
      }
      if (hadSessionCookie || hadConsentCookie) {
        resolveConsentStatus(TRUE);
        callback(consentStatusResult);
        return;
      }
      consentStatusCallbacks.push(callback);
    }

    /* RR_TEST_START */
    // Test-only exports. Production builds strip this block before minify
    // (see scripts/build.mjs) so it adds zero bytes to tracker-min.js.
    if (typeof RR_TEST !== "undefined" && RR_TEST) {
      window.__rrTest = {
        version: version,
        hasOwn: hasOwn,
        makeSet: makeSet,
        escapeRegExp: escapeRegExp,
        getDataAttribute: getDataAttribute,
        isReportableHostname: isReportableHostname,
        parseList: parseList,
        isPlainObject: isPlainObject,
        mergeObjects: mergeObjects,
        isFinite: isFinite,
        createId: createId,
        getCookie: getCookie,
        getCookieDomain: getCookieDomain,
        setCookie: setCookie,
        updateSessionId: updateSessionId,
        getSession: getSession,
        collectMetadata: collectMetadata,
        payloadQueryString: payloadQueryString,
        payloadWithRuntimeState: payloadWithRuntimeState,
        applyIngestResponse: applyIngestResponse,
        stringifyObj: stringifyObj,
        serializeMetadataPayload: serializeMetadataPayload,
        diffMetadata: diffMetadata,
        serializeMetadataDiff: serializeMetadataDiff,
        extractQueryParams: extractQueryParams,
        parseDeclarativeValue: parseDeclarativeValue,
        parseDeclarativeMetadata: parseDeclarativeMetadata,
        applyDeclarativeMetadata: applyDeclarativeMetadata,
        scanDeclarativeMarkup: scanDeclarativeMarkup,
        mutationsNeedScan: mutationsNeedScan,
        looksLikeSameTabNavigation: looksLikeSameTabNavigation,
        outboundDestination: outboundDestination,
        getTrackedPath: getTrackedPath,
        calculateIdleDuration: calculateIdleDuration,
        finishIdlePeriod: finishIdlePeriod,
        recordScrollActivity: recordScrollActivity,
        updateIdleState: updateIdleState,
        isElementHidden: isElementHidden,
        isElementVisible: isElementVisible,
        clickCooldownElapsed: clickCooldownElapsed,
        trackEvent: trackEvent,
        sendDeviceContext: sendDeviceContext,
        updateMetadata: updateMetadata,
        trackPageview: trackPageview,
        setConsent: setConsent,
        consentStatus: consentStatus,
        flushTrackingQueue: flushTrackingQueue,
        enqueueTrackingRequest: enqueueTrackingRequest,
        batchItemPayload: batchItemPayload,
        batchEnvelopePayload: batchEnvelopePayload,
        getSessionId: function () {
          return sessionId;
        },
        getConsent: function () {
          return consent;
        },
        getPageId: function () {
          return pageId;
        },
        getPageMetadata: function () {
          return pageMetadata;
        },
        getLastSentMetadata: function () {
          return lastSentMetadata;
        },
        getProjectName: function () {
          return projectName;
        },
        getTrackedHostname: function () {
          return trackedHostname;
        },
        getMaxScrollDepth: function () {
          return maxScrollDepth;
        },
        setMaxScrollDepth: function (value) {
          maxScrollDepth = value;
        },
        getIdleThresholdMs: function () {
          return idleThresholdMs;
        },
        setLastScrollActivityAt: function (value) {
          lastScrollActivityAt = value;
        },
        setPageEngagementStartedAt: function (value) {
          pageEngagementStartedAt = value;
        },
        setIdleStartedAt: function (value) {
          idleStartedAt = value;
        },
        getIdleStartedAt: function () {
          return idleStartedAt;
        },
        getIdleDuration: function () {
          return idleDuration;
        },
        setIdleDuration: function (value) {
          idleDuration = value;
        },
        getAllowedParameterMap: function () {
          return allowedParameterMap;
        },
        getClickIdParameterPattern: function () {
          return clickIdParameterPattern;
        },
        getBasePayload: function () {
          return basePayload;
        },
        getBatchQueue: function () {
          return batchQueue;
        },
        dispatchCommand: dispatchCommand,
      };
    }
    /* RR_TEST_END */
  } catch (error) {
    // Catch any errors during tracker load and try to report them to the server
    if (typeof reportError == "function") {
      reportError(error);
    }
  }
})(window);
