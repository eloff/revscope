#!/usr/bin/env node

/**
 * Download published crawler / proxy CIDR feeds into revscope/ipranges/.
 *
 * Usage:
 *   node scripts/update-ip-ranges.mjs
 */

import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const outDir = path.join(scriptDir, "..", "revscope", "ipranges");

const USER_AGENT = "Mozilla/5.0 (compatible; RevScope-ip-ranges/1.0; +https://revscope.co)";

/** @typedef {{ name: string, url: string, operator: string, normalize?: (body: unknown) => PrefixFeed }} Feed */

/** @typedef {{ creationTime?: string, prefixes: Array<{ ipv4Prefix?: string, ipv6Prefix?: string }> }} PrefixFeed */

/** @type {Feed[]} */
const FEEDS = [
  {
    name: "google-common-crawlers.json",
    url: "https://developers.google.com/static/crawling/ipranges/common-crawlers.json",
    operator: "google",
  },
  {
    name: "google-special-crawlers.json",
    url: "https://developers.google.com/static/crawling/ipranges/special-crawlers.json",
    operator: "google",
  },
  {
    name: "bingbot.json",
    url: "https://www.bing.com/toolbox/bingbot.json",
    operator: "microsoft",
  },
  {
    name: "openai-gptbot.json",
    url: "https://openai.com/gptbot.json",
    operator: "openai",
  },
  {
    name: "openai-searchbot.json",
    url: "https://openai.com/searchbot.json",
    operator: "openai",
  },
  {
    name: "openai-chatgpt-user.json",
    url: "https://openai.com/chatgpt-user.json",
    operator: "openai",
  },
  {
    name: "anthropic-bots.json",
    url: "https://claude.com/crawling/bots.json",
    operator: "anthropic",
  },
  {
    name: "perplexitybot.json",
    url: "https://www.perplexity.ai/perplexitybot.json",
    operator: "perplexity",
  },
  {
    name: "perplexity-user.json",
    url: "https://www.perplexity.ai/perplexity-user.json",
    operator: "perplexity",
  },
  {
    name: "duckassistbot.json",
    url: "https://duckduckgo.com/duckassistbot.json",
    operator: "duckduckgo",
  },
  {
    name: "ccbot.json",
    url: "https://index.commoncrawl.org/ccbot.json",
    operator: "commoncrawl",
  },
  {
    name: "applebot.json",
    url: "https://search.developer.apple.com/applebot.json",
    operator: "apple",
  },
  {
    name: "cloudflare.json",
    url: "https://api.cloudflare.com/client/v4/ips",
    operator: "cloudflare",
    normalize: normalizeCloudflare,
  },
];

function isPrefix(value) {
  return (
    value !== null &&
    typeof value === "object" &&
    (typeof value.ipv4Prefix === "string" || typeof value.ipv6Prefix === "string")
  );
}

function asPrefixFeed(body) {
  if (body === null || typeof body !== "object") {
    return null;
  }
  const prefixes = body.prefixes;
  if (!Array.isArray(prefixes) || prefixes.length === 0 || !prefixes.every(isPrefix)) {
    return null;
  }
  return {
    ...(typeof body.creationTime === "string" ? { creationTime: body.creationTime } : {}),
    prefixes,
  };
}

function normalizeCloudflare(body) {
  if (
    body === null ||
    typeof body !== "object" ||
    body.result === null ||
    typeof body.result !== "object"
  ) {
    return { creationTime: new Date().toISOString(), prefixes: [] };
  }
  const result = body.result;
  const ipv4 = Array.isArray(result.ipv4_cidrs) ? result.ipv4_cidrs : [];
  const ipv6 = Array.isArray(result.ipv6_cidrs) ? result.ipv6_cidrs : [];
  return {
    creationTime: new Date().toISOString(),
    prefixes: [
      ...ipv4.filter((value) => typeof value === "string").map((ipv4Prefix) => ({ ipv4Prefix })),
      ...ipv6.filter((value) => typeof value === "string").map((ipv6Prefix) => ({ ipv6Prefix })),
    ],
  };
}

async function downloadFeed(feed) {
  const response = await fetch(feed.url, {
    headers: {
      Accept: "application/json",
      "User-Agent": USER_AGENT,
    },
  });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  const body = await response.json();
  const feedJson = asPrefixFeed(feed.normalize ? feed.normalize(body) : body);
  if (!feedJson) {
    throw new Error("response had no usable prefixes");
  }
  return feedJson;
}

async function existingPrefixCount(filePath) {
  try {
    const existing = asPrefixFeed(JSON.parse(await readFile(filePath, "utf8")));
    return existing?.prefixes.length ?? 0;
  } catch {
    return 0;
  }
}

async function main() {
  await mkdir(outDir, { recursive: true });
  let failed = 0;
  for (const feed of FEEDS) {
    const filePath = path.join(outDir, feed.name);
    try {
      const feedJson = await downloadFeed(feed);
      await writeFile(filePath, `${JSON.stringify(feedJson, null, 2)}\n`);
      console.log(`${feed.name}: wrote ${feedJson.prefixes.length} prefixes (${feed.operator})`);
    } catch (error) {
      failed += 1;
      const kept = await existingPrefixCount(filePath);
      const reason = error instanceof Error ? error.message : String(error);
      console.warn(
        `${feed.name}: ${reason}; keeping existing file${kept > 0 ? ` (${kept} prefixes)` : ""}`,
      );
    }
  }
  if (failed > 0) {
    process.exitCode = 1;
  }
}

await main();
