# JSTorrent Website

The public website is an Astro application deployed to GitHub Pages. It
contains the product site, downloads, blog, legacy compatibility routes, and a
browser-hosted client page that reuses JSTorrent's React/Solid packages.

## Development

From the repository root:

```bash
pnpm --filter jstorrent-website dev
pnpm --filter jstorrent-website build
pnpm --filter jstorrent-website preview
```

The development server uses <http://localhost:3000>.

## Source Map

- `src/pages/`: Astro routes, client page, blog, and RSS
- `src/components/`: product, download, FAQ, and client components
- `src/content/`: blog content
- `public/`: static files, AltStore source JSON, and retained legacy URLs
- `astro.config.mjs`: site URL, integrations, aliases, and development port

The build aliases `@jstorrent/engine`, `@jstorrent/client`, and
`@jstorrent/ui` to workspace source.

## Deployment

[`deploy-website.yml`](../.github/workflows/deploy-website.yml) builds and
deploys the site on relevant `main` changes and `website-v*` tags. It resolves
the latest Tauri release so download links track current desktop artifacts.

See the [release topic](../docs/topics/releases.md) for the optional website tag
workflow.
