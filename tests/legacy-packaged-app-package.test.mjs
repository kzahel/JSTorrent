import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { execFile } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'

import { LEGACY_APP_VARIANTS, extensionIdFromPublicKey } from '../scripts/legacy-app-variants.mjs'

const execFileAsync = promisify(execFile)
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const packageScript = path.join(repoRoot, 'scripts', 'package-legacy-app.mjs')

async function sha256(file) {
  return createHash('sha256')
    .update(await readFile(file))
    .digest('hex')
}

test('legacy package builder is deterministic and leaves source manifests untouched', async () => {
  const temporaryRoot = await mkdtemp(path.join(os.tmpdir(), 'jstorrent-package-test-'))
  const firstOutput = path.join(temporaryRoot, 'first')
  const secondOutput = path.join(temporaryRoot, 'second')
  const fixtures = path.join(firstOutput, 'fixtures')
  const sourceManifestPath = path.join(repoRoot, 'archive', 'legacy-app', 'manifest.json')
  const sourceMessagesPath = path.join(
    repoRoot,
    'archive',
    'legacy-app',
    '_locales',
    'en',
    'messages.json',
  )
  const sourceBefore = [await sha256(sourceManifestPath), await sha256(sourceMessagesPath)]

  try {
    await execFileAsync(process.execPath, [
      packageScript,
      '--output-dir',
      firstOutput,
      '--fixtures-dir',
      fixtures,
    ])
    await execFileAsync(process.execPath, [packageScript, '--output-dir', secondOutput])

    for (const variant of LEGACY_APP_VARIANTS) {
      assert.equal(extensionIdFromPublicKey(variant.publicKey), variant.extensionId)
      assert.equal(
        await sha256(path.join(firstOutput, variant.archiveName)),
        await sha256(path.join(secondOutput, variant.archiveName)),
      )

      const fixtureManifest = JSON.parse(
        await readFile(path.join(fixtures, `${variant.slug}-candidate`, 'manifest.json'), 'utf8'),
      )
      assert.equal(fixtureManifest.version, variant.candidateVersion)
      assert.equal(fixtureManifest.key, variant.publicKey)
    }

    assert.deepEqual(
      [await sha256(sourceManifestPath), await sha256(sourceMessagesPath)],
      sourceBefore,
    )
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true })
  }
})
