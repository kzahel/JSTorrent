import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const migrationPagePath = path.join(repoRoot, 'website', 'src', 'pages', 'migrate.astro')

test('migration page presents every supported replacement composition', async () => {
  const source = await readFile(migrationPagePath, 'utf8')
  assert.match(source, /standalone desktop app/i)
  assert.match(source, /Android app works as a standalone client/i)
  assert.match(source, /install-crostini\.sh/)
  assert.match(source, /extension is browser integration, not a\s+complete torrent client/is)
  assert.match(source, /not transferred\s+automatically/i)
  assert.doesNotMatch(source, /same features and more|you're all set|join (the )?waitlist/i)
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

test('migration page progressively detects the extension without claiming readiness', async () => {
  const source = await readFile(migrationPagePath, 'utf8')
  assert.match(source, /sendMessage\(extensionId, \{ type: 'ping' \}/)
  assert.match(source, /extension detected.*still needs/is)
  assert.match(source, /all supported choices remain available/i)
})
