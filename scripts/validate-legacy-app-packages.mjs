import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

import { LEGACY_APP_VARIANTS } from './legacy-app-variants.mjs'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const defaultPackageDir = path.join(repoRoot, 'dist', 'legacy-app')
const requiredFiles = [
  '_locales/en/messages.json',
  'background.js',
  'manifest.json',
  'migrate.html',
  'migrate.js',
  'migration.js',
  'migration-runtime.js',
]
const forbiddenPaths = new Set([
  '_metadata/verified_contents.json',
  'LICENSE',
  'README.md',
  'TODO.txt',
  'package.sh',
])
const forbiddenCopy =
  /join (the )?waitlist|coming soon|not yet available|new\.jstorrent\.com|MIGRATE_ON_SCRIPT_LOAD|showMigrationNags|same features and more|you're all set/i

function parseArgs(argv) {
  let packageDir = defaultPackageDir
  let baselineRoot = ''
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === '--dir' || argument === '--baseline-root') {
      const value = argv[++index]
      if (!value || value.startsWith('--')) throw new Error(`${argument} requires a path`)
      if (argument === '--dir') packageDir = path.resolve(value)
      else baselineRoot = path.resolve(value)
    } else throw new Error(`Unknown argument: ${argument}`)
  }
  return { baselineRoot, packageDir }
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', ...options })
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(' ')} failed:\n${result.stdout || ''}${result.stderr || ''}`,
    )
  }
  return result.stdout
}

async function sha256(file) {
  return createHash('sha256')
    .update(await readFile(file))
    .digest('hex')
}

async function extractAndValidate(packageDir, variant, temporaryRoot) {
  const archive = path.join(packageDir, variant.archiveName)
  const archiveEntries = run('unzip', ['-Z1', archive])
    .trim()
    .split('\n')
    .filter((entry) => entry && !entry.endsWith('/'))
    .sort()
  assert.ok(
    archiveEntries.includes('manifest.json'),
    `${variant.slug}: manifest is not at ZIP root`,
  )
  assert.equal(
    new Set(archiveEntries).size,
    archiveEntries.length,
    `${variant.slug}: duplicate paths`,
  )
  for (const requiredFile of requiredFiles) {
    assert.ok(archiveEntries.includes(requiredFile), `${variant.slug}: missing ${requiredFile}`)
  }
  for (const entry of archiveEntries) {
    assert.ok(!entry.startsWith('/'), `${variant.slug}: absolute path ${entry}`)
    assert.ok(!entry.split('/').includes('..'), `${variant.slug}: parent path ${entry}`)
    assert.ok(!forbiddenPaths.has(entry), `${variant.slug}: forbidden path ${entry}`)
    assert.ok(!entry.endsWith('.bak'), `${variant.slug}: backup path ${entry}`)
    assert.ok(!entry.startsWith('_metadata/'), `${variant.slug}: Store metadata path ${entry}`)
    assert.ok(
      !entry.split('/').some((part) => part.startsWith('.')),
      `${variant.slug}: dot path ${entry}`,
    )
  }

  const extracted = path.join(temporaryRoot, variant.slug)
  run('unzip', ['-qq', archive, '-d', extracted])
  const manifest = JSON.parse(await readFile(path.join(extracted, 'manifest.json'), 'utf8'))
  const messages = JSON.parse(
    await readFile(path.join(extracted, '_locales', 'en', 'messages.json'), 'utf8'),
  )
  const sourceManifest = JSON.parse(
    await readFile(path.join(repoRoot, 'archive', 'legacy-app', 'manifest.json'), 'utf8'),
  )

  assert.equal(manifest.version, variant.candidateVersion)
  assert.equal(messages.extName.message, variant.productName)
  assert.equal(manifest.manifest_version, 2)
  assert.ok(!Object.hasOwn(manifest, 'key'), `${variant.slug}: shipping manifest contains test key`)
  assert.deepEqual(manifest.permissions, sourceManifest.permissions)
  assert.deepEqual(manifest.optional_permissions, sourceManifest.optional_permissions)
  assert.deepEqual(manifest.app.background.scripts, [
    'conf.js',
    'migration.js',
    'migration-runtime.js',
    'background.js',
  ])

  for (const entry of archiveEntries) {
    if (!/\.(html|js|json|txt)$/i.test(entry)) continue
    const contents = await readFile(path.join(extracted, entry), 'utf8')
    assert.doesNotMatch(
      contents,
      forbiddenCopy,
      `${variant.slug}: stale migration copy in ${entry}`,
    )
  }
  const migrationSource = await readFile(path.join(extracted, 'migration.js'), 'utf8')
  assert.match(migrationSource, /https:\/\/jstorrent\.com\/migrate/)
  assert.match(migrationSource, /LEGACY_MIGRATION_REMINDER_DAYS = 7/)

  return {
    archive,
    archiveEntries,
    extracted,
    manifest,
    messages,
    hash: await sha256(archive),
    variant,
  }
}

async function compareVariants(paid, lite) {
  assert.deepEqual(paid.archiveEntries, lite.archiveEntries, 'paid/Lite ZIP file lists differ')
  for (const entry of paid.archiveEntries) {
    if (entry === 'manifest.json' || entry === '_locales/en/messages.json') continue
    assert.equal(
      await sha256(path.join(paid.extracted, entry)),
      await sha256(path.join(lite.extracted, entry)),
      `paid/Lite content differs at ${entry}`,
    )
  }

  const paidManifest = structuredClone(paid.manifest)
  const liteManifest = structuredClone(lite.manifest)
  delete paidManifest.version
  delete liteManifest.version
  assert.deepEqual(paidManifest, liteManifest, 'paid/Lite manifests differ beyond version')

  const paidMessages = structuredClone(paid.messages)
  const liteMessages = structuredClone(lite.messages)
  delete paidMessages.extName.message
  delete liteMessages.extName.message
  assert.deepEqual(paidMessages, liteMessages, 'paid/Lite messages differ beyond product name')
}

async function listFiles(root, prefix = '') {
  const files = []
  const entries = await readdir(path.join(root, prefix), { withFileTypes: true })
  for (const entry of entries) {
    const relativePath = path.posix.join(prefix.split(path.sep).join(path.posix.sep), entry.name)
    if (entry.isDirectory()) files.push(...(await listFiles(root, relativePath)))
    else if (entry.isFile()) files.push(relativePath)
  }
  return files.sort()
}

async function compareWithStoreBaselines(validated, baselineRoot) {
  const expectedRemoved = [
    'LICENSE',
    'README.md',
    'TODO.txt',
    '_locales/en/messages.json.bak',
    '_metadata/verified_contents.json',
  ].sort()
  const expectedAdded = [
    'migrate.html',
    'migrate.js',
    'migration-runtime.js',
    'migration.js',
  ].sort()
  const expectedChanged = ['background.js', 'manifest.json'].sort()

  for (const result of validated) {
    const baselineDir = path.join(baselineRoot, result.variant.slug)
    const baselineFiles = await listFiles(baselineDir)
    const baselineSet = new Set(baselineFiles)
    const candidateSet = new Set(result.archiveEntries)
    const removed = baselineFiles.filter((file) => !candidateSet.has(file))
    const added = result.archiveEntries.filter((file) => !baselineSet.has(file))
    const changed = []
    for (const file of baselineFiles) {
      if (!candidateSet.has(file)) continue
      if (
        (await sha256(path.join(baselineDir, file))) !==
        (await sha256(path.join(result.extracted, file)))
      ) {
        changed.push(file)
      }
    }

    assert.deepEqual(removed, expectedRemoved, `${result.variant.slug}: unexpected removed paths`)
    assert.deepEqual(added, expectedAdded, `${result.variant.slug}: unexpected added paths`)
    assert.deepEqual(changed, expectedChanged, `${result.variant.slug}: unexpected changed paths`)

    const baselineManifest = JSON.parse(
      await readFile(path.join(baselineDir, 'manifest.json'), 'utf8'),
    )
    const baselineMessages = JSON.parse(
      await readFile(path.join(baselineDir, '_locales', 'en', 'messages.json'), 'utf8'),
    )
    assert.equal(baselineManifest.version, result.variant.baselineVersion)
    assert.equal(baselineMessages.extName.message, result.variant.productName)
    console.log(
      `${result.variant.slug} baseline diff: ${removed.length} removed, ${added.length} added, ${changed.length} changed`,
    )
  }
}

async function main() {
  const { baselineRoot, packageDir } = parseArgs(process.argv.slice(2))
  const temporaryRoot = await mkdtemp(path.join(os.tmpdir(), 'jstorrent-legacy-validate-'))
  try {
    const validated = []
    for (const variant of LEGACY_APP_VARIANTS) {
      validated.push(await extractAndValidate(packageDir, variant, temporaryRoot))
    }
    await compareVariants(validated[0], validated[1])
    if (baselineRoot) await compareWithStoreBaselines(validated, baselineRoot)

    const sumsPath = path.join(packageDir, 'SHA256SUMS')
    const recordedSums = await readFile(sumsPath, 'utf8')
    for (const result of validated) {
      assert.match(recordedSums, new RegExp(`${result.hash}  ${path.basename(result.archive)}`))
      console.log(
        `${path.basename(result.archive)}: ${result.archiveEntries.length} files, SHA-256 ${result.hash}`,
      )
    }

    const directoryEntries = await readdir(packageDir)
    assert.ok(directoryEntries.includes('SHA256SUMS'))
    console.log('Legacy paid/Lite package validation passed.')
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error)
  process.exitCode = 1
})
