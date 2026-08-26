import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const migrationPagePath = path.join(repoRoot, 'website', 'src', 'pages', 'migrate.astro')

test('migration page presents every supported replacement composition', async () => {
  const source = await readFile(migrationPagePath, 'utf8')
  assert.match(source, /Keep using JSTorrent/i)
  assert.match(source, /complete JSTorrent app for your computer/i)
  assert.match(source, /complete JSTorrent app for Android/i)
  assert.match(source, /install-crostini\.sh/)
  assert.match(source, /install the JSTorrent Linux service and\s+connect the Chrome extension/is)
  assert.match(source, /not transferred\s+automatically/i)
  assert.doesNotMatch(
    source,
    /same features and more|you're all set|join (the )?waitlist|Legacy Chrome App migration/i,
  )
})

test('migration page bounds and propagates only campaign attribution', async () => {
  const source = await readFile(migrationPagePath, 'utf8')
  for (const key of ['ref', 'variant', 'platform', 'campaign']) {
    assert.match(source, new RegExp(`${key}: new Set`))
  }
  assert.match(source, /available-2026/)
  assert.match(source, /data-migration-link/g)
  assert.match(source, /destination\.searchParams\.set\(key, value\)/)
  assert.doesNotMatch(source, /installId|clientId|userId|email/i)
})

test('migration page reports only positive extension detection inside the Linux choice', async () => {
  const source = await readFile(migrationPagePath, 'utf8')
  assert.match(source, /sendMessage\(extensionId, \{ type: 'ping' \}/)
  assert.match(source, /id="extension-status"[^>]*hidden/is)
  assert.match(source, /extension already installed/i)
  assert.match(source, /extensionStatus\.hidden = false/)
  assert.doesNotMatch(source, /extension not detected|checking whether.*extension/is)
})

test('migration page recommends and orders choices without narrating device detection', async () => {
  const source = await readFile(migrationPagePath, 'utf8')
  assert.match(source, /android: 'play'/)
  assert.match(source, /windows: 'desktop'/)
  assert.match(source, /android: \['play', 'desktop', 'crostini'\]/)
  assert.match(source, /recommendedCard\.classList\.add\('recommended'\)/)
  assert.match(source, /data-recommendation hidden/)
  assert.match(source, /Other setup options/)
  assert.match(source, /insertAdjacentElement\('afterend', otherOptionsHeading\)/)
  assert.doesNotMatch(source, /Opened from the old|Choices for .* shown first|platformNames/i)
})
