# Search Plugins

Topic: `search-plugins`

Status: **living plugin authoring and runtime reference; last reconciled on
2026-03-13**.

JSTorrent search plugins are small JavaScript modules that provide torrent search results from a specific site or API. A plugin is installed from a URL, loaded in a sandbox, and asked to export a manifest plus a `search(ctx, input)` function.

The current reference implementation is
[`search-plugins/internet-archive.js`](../../search-plugins/internet-archive.js).

For platform isolation and policy implications, see
[`sandbox-and-search-plugin-trust-boundaries.md`](sandbox-and-search-plugin-trust-boundaries.md).

## Plugin file structure

Each plugin is a single ES module that exports:

- `manifest`: metadata that describes the plugin and declares which hosts it may access
- `search(ctx, input)`: the function JSTorrent calls to perform a search

Minimal template:

```js
export const manifest = {
  id: 'com.example.search',
  name: 'Example Search',
  version: '0.1.0',
  description: 'Search example.com',
  homepage: 'https://example.com',
  hosts: ['example.com'],
  categories: ['all', 'movies'],
}

export async function search(ctx, input) {
  const url =
    'https://example.com/search?q=' +
    ctx.encode(input.query) +
    '&category=' +
    ctx.encode(input.category || 'all')

  const html = await ctx.fetchText({ url, method: 'GET' })
  const document = ctx.parseHtml(html)

  for (const row of document.queryAll('.result')) {
    const title = row.query('.title')?.text().trim()
    const torrentUrl = row.query('a.torrent')?.attr('href')
    if (!title || !torrentUrl) continue

    ctx.emitResult({
      name: title,
      source: 'Example Search',
      torrentUrl,
    })
  }
}
```

## Manifest reference

`manifest` must export a non-empty `name` and at least one declared host in `hosts`.

| Field | Required | Notes |
|------|----------|-------|
| `id` | No | Stable plugin identifier. If omitted, JSTorrent generates one from the plugin name and source hash. |
| `name` | Yes | User-facing plugin name. |
| `version` | No | Plugin version string. |
| `description` | No | Short description shown in plugin UI. |
| `homepage` | No | Project or provider homepage. |
| `source` | No | Source URL for the plugin. This is typically filled automatically when installed from a URL. |
| `hosts` | Yes | Allowed network hosts for plugin fetches. Use hostnames only, for example `['archive.org']`. |
| `categories` | No | Supported category filters exposed by the plugin, for example `['all', 'movies', 'music']`. |

Host rules:

- Hosts are normalized to lowercase.
- `http` and `https` requests are allowed.
- Requests must target a declared host or one of its subdomains.
- Wildcards such as `*.example.com` are not supported.

## `search(ctx, input)`

JSTorrent calls `search(ctx, input)` for each search run.

`input` shape:

```ts
{
  query: string
  category?: string
}
```

The function can return synchronously or asynchronously. It should call `ctx.emitResult(...)` once for each result it finds.

## Runtime functions

The plugin runtime exposes these helpers on `ctx`:

| Function | Purpose |
|----------|---------|
| `ctx.encode(value)` | URL-encodes a string with `encodeURIComponent`. |
| `ctx.fetchText(input)` | Fetches text from an allowed URL. |
| `ctx.fetchJson(input)` | Fetches text from an allowed URL and parses it as JSON. |
| `ctx.parseHtml(html)` | Parses HTML and returns a queryable document wrapper. |
| `ctx.emitResult(result)` | Adds one search result to the output list. |
| `ctx.log(level, message)` | Emits a debug/info/warn/error log entry for the run trace. |

Fetch input shape:

```ts
{
  url: string
  method?: 'GET' | 'POST'
  headers?: Record<string, string>
  body?: string
}
```

HTML parser helpers available on the parsed document and any queried node:

- `text()`
- `html()`
- `attr(name)`
- `query(selector)`
- `queryAll(selector)`

## Result shape

Each emitted result should look like this:

```ts
{
  name: string
  source: string
  size?: number
  seeds?: number
  leeches?: number
  magnetUrl?: string
  torrentUrl?: string
  infoHash?: string
  detailsUrl?: string
  publishedAt?: number
}
```

At least one of `magnetUrl` or `torrentUrl` should be provided so the result can be added to JSTorrent.

## Internet Archive reference

The bundled Internet Archive example shows the intended structure and a realistic API-backed implementation:

- File:
  [`search-plugins/internet-archive.js`](../../search-plugins/internet-archive.js)
- Manifest declares `archive.org` in `hosts` and exposes categories such as `movies`, `music`, `books`, and `software`
- `search(ctx, input)` builds an `advancedsearch.php` query, calls `ctx.fetchJson(...)`, and emits torrent results from the API response
- It uses `ctx.encode(...)` for URL construction and `ctx.log(...)` for run diagnostics
- It emits `torrentUrl`, `detailsUrl`, `size`, `seeds`, and `publishedAt`

Key excerpt:

```js
export const manifest = {
  id: 'org.archive.search',
  name: 'Internet Archive',
  version: '0.1.0',
  homepage: 'https://archive.org',
  hosts: ['archive.org'],
  categories: ['all', 'movies', 'music', 'books', 'software'],
}

export async function search(ctx, input) {
  const payload = await ctx.fetchJson({
    url: buildSearchUrl(ctx, input),
    method: 'GET',
  })

  for (const doc of payload.response.docs || []) {
    if (!doc?.identifier) continue

    ctx.emitResult({
      name: doc.title || doc.identifier,
      source: 'Internet Archive',
      torrentUrl: buildTorrentUrl(doc.identifier),
      detailsUrl: 'https://archive.org/details/' + doc.identifier,
    })
  }
}
```

## Current limitations

- Plugins must export with `export const ...` and `export function ...` or `export async function ...`
- `export default` is not supported
- Fetch access is restricted to hosts declared in the manifest
- Non-HTTP(S) protocols are rejected
