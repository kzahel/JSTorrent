import { createHash, randomUUID } from 'node:crypto'
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
  utimes,
  writeFile,
} from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

import { LEGACY_APP_VARIANTS, extensionIdFromPublicKey } from './legacy-app-variants.mjs'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoot = path.join(repoRoot, 'archive', 'legacy-app')
const defaultOutputDir = path.join(repoRoot, 'dist', 'legacy-app')
const fixedDate = new Date(Date.UTC(2026, 7, 26, 0, 0, 0))
const excludedFiles = new Set(['LICENSE', 'README.md', 'TODO.txt', 'package.sh'])
const requiredFiles = [
  'background.js',
  'manifest.json',
  '_locales/en/messages.json',
  'migrate.html',
  'migrate.js',
  'migration.js',
  'migration-runtime.js',
]

function parseArgs(argv) {
  const options = { outputDir: defaultOutputDir, fixturesDir: '', baselineRoot: '' }
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === '--output-dir') {
      const value = argv[++index]
      if (!value || value.startsWith('--')) throw new Error('--output-dir requires a path')
      options.outputDir = path.resolve(value)
    } else if (argument === '--fixtures-dir') {
      const value = argv[++index]
      if (!value || value.startsWith('--')) throw new Error('--fixtures-dir requires a path')
      options.fixturesDir = path.resolve(value)
    } else if (argument === '--baseline-root') {
      const value = argv[++index]
      if (!value || value.startsWith('--')) throw new Error('--baseline-root requires a path')
      options.baselineRoot = path.resolve(value)
    } else {
      throw new Error(`Unknown argument: ${argument}`)
    }
  }
  if (options.fixturesDir) {
    const relativeFixturePath = path.relative(options.outputDir, options.fixturesDir)
    if (
      !relativeFixturePath ||
      relativeFixturePath === '..' ||
      relativeFixturePath.startsWith(`..${path.sep}`) ||
      path.isAbsolute(relativeFixturePath)
    ) {
      throw new Error('--fixtures-dir must be a child of --output-dir')
    }
  }
  if (options.baselineRoot && !options.fixturesDir) {
    throw new Error('--baseline-root requires --fixtures-dir')
  }
  return options
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    ...options,
  })
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(' ')} failed:\n${result.stdout || ''}${result.stderr || ''}`,
    )
  }
  return result.stdout
}

function trackedPackageFiles() {
  const prefix = 'archive/legacy-app/'
  const files = run('git', ['ls-files', '--', 'archive/legacy-app'])
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((file) => {
      if (!file.startsWith(prefix)) throw new Error(`Unexpected tracked path: ${file}`)
      return file.slice(prefix.length)
    })
    .filter((file) => {
      if (excludedFiles.has(file)) return false
      if (file.startsWith('_metadata/')) return false
      if (file.endsWith('.bak') || file.endsWith('.zip')) return false
      return !file.split('/').some((part) => part.startsWith('.'))
    })
    .sort()

  for (const requiredFile of requiredFiles) {
    if (!files.includes(requiredFile))
      throw new Error(`Required package file missing: ${requiredFile}`)
  }
  return files
}

async function normalizeFile(file) {
  await chmod(file, 0o644)
  await utimes(file, fixedDate, fixedDate)
}

async function writeNormalized(file, contents) {
  await writeFile(file, contents)
  await normalizeFile(file)
}

async function buildTree(destination, variant, { includeTestKey = false } = {}) {
  const files = trackedPackageFiles()
  for (const relativeFile of files) {
    const source = path.join(sourceRoot, relativeFile)
    const target = path.join(destination, relativeFile)
    await mkdir(path.dirname(target), { recursive: true })
    await copyFile(source, target)
    await normalizeFile(target)
  }

  const manifestPath = path.join(destination, 'manifest.json')
  let manifest = await readFile(manifestPath, 'utf8')
  const sourceVersion = '"version": "2.4.4"'
  if (!manifest.includes(sourceVersion)) throw new Error('Source manifest version changed')
  manifest = manifest.replace(sourceVersion, `"version": "${variant.candidateVersion}"`)
  manifest = manifest.replace(/^\s*"key":.*\n/gm, '')
  if (includeTestKey) manifest = manifest.replace('{\n', `{\n"key": "${variant.publicKey}",\n`)
  await writeNormalized(manifestPath, manifest)

  const messagesPath = path.join(destination, '_locales', 'en', 'messages.json')
  let messages = await readFile(messagesPath, 'utf8')
  const sourceName = '"message": "JSTorrent"'
  if (!messages.includes(sourceName)) throw new Error('Source localized product name changed')
  messages = messages.replace(sourceName, `"message": "${variant.productName}"`)
  await writeNormalized(messagesPath, messages)

  return files
}

async function listBaselineFiles(root, prefix = '') {
  const files = []
  const entries = await readdir(path.join(root, prefix), { withFileTypes: true })
  for (const entry of entries) {
    const relativePath = path.join(prefix, entry.name)
    if (entry.isDirectory()) files.push(...(await listBaselineFiles(root, relativePath)))
    else if (entry.isFile()) files.push(relativePath)
  }
  return files.sort()
}

async function buildBaselineFixture(destination, baselineRoot, variant) {
  const variantRoot = path.join(baselineRoot, variant.slug)
  const files = (await listBaselineFiles(variantRoot)).filter(
    (file) => !file.startsWith(`_metadata${path.sep}`) && !file.endsWith('.bak'),
  )
  for (const relativeFile of files) {
    const target = path.join(destination, relativeFile)
    await mkdir(path.dirname(target), { recursive: true })
    await copyFile(path.join(variantRoot, relativeFile), target)
  }

  const manifestPath = path.join(destination, 'manifest.json')
  let manifest = await readFile(manifestPath, 'utf8')
  const parsedManifest = JSON.parse(manifest)
  if (parsedManifest.version !== variant.baselineVersion) {
    throw new Error(`${variant.slug} baseline version is ${parsedManifest.version}`)
  }
  manifest = manifest.replace(/^\s*"key":.*\n/gm, '')
  manifest = manifest.replace('{\n', `{\n"key": "${variant.publicKey}",\n`)
  await writeFile(manifestPath, manifest)

  const messages = JSON.parse(
    await readFile(path.join(destination, '_locales', 'en', 'messages.json'), 'utf8'),
  )
  if (messages.extName.message !== variant.productName) {
    throw new Error(`${variant.slug} baseline product name is ${messages.extName.message}`)
  }
}

async function createZip(stagingDir, relativeFiles, outputZip) {
  await mkdir(path.dirname(outputZip), { recursive: true })
  const temporaryZip = path.join(
    path.dirname(outputZip),
    `.${path.basename(outputZip)}.${randomUUID()}.tmp`,
  )
  const result = spawnSync('zip', ['-X', '-q', '-D', temporaryZip, '-@'], {
    cwd: stagingDir,
    encoding: 'utf8',
    env: { ...process.env, TZ: 'UTC' },
    input: `${relativeFiles.join('\n')}\n`,
  })
  if (result.status !== 0) {
    await rm(temporaryZip, { force: true })
    throw new Error(`zip failed:\n${result.stdout || ''}${result.stderr || ''}`)
  }
  await rename(temporaryZip, outputZip)
}

async function sha256(file) {
  return createHash('sha256')
    .update(await readFile(file))
    .digest('hex')
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  for (const variant of LEGACY_APP_VARIANTS) {
    const derivedId = extensionIdFromPublicKey(variant.publicKey)
    if (derivedId !== variant.extensionId) {
      throw new Error(`${variant.slug} public key derives ${derivedId}, not ${variant.extensionId}`)
    }
  }

  const workingRoot = await mkdtemp(path.join(os.tmpdir(), 'jstorrent-legacy-package-'))
  const fixtureWorking = path.join(workingRoot, 'fixtures')
  const results = []
  try {
    for (const variant of LEGACY_APP_VARIANTS) {
      const stagingDir = path.join(workingRoot, `zip-${variant.slug}`)
      const relativeFiles = await buildTree(stagingDir, variant)
      const outputZip = path.join(options.outputDir, variant.archiveName)
      await createZip(stagingDir, relativeFiles, outputZip)
      results.push({ variant, outputZip, hash: await sha256(outputZip) })

      if (options.fixturesDir) {
        await buildTree(path.join(fixtureWorking, `${variant.slug}-candidate`), variant, {
          includeTestKey: true,
        })
        if (options.baselineRoot) {
          await buildBaselineFixture(
            path.join(fixtureWorking, `${variant.slug}-baseline`),
            options.baselineRoot,
            variant,
          )
        }
      }
    }

    await mkdir(options.outputDir, { recursive: true })
    const sums = results
      .map(({ outputZip, hash }) => `${hash}  ${path.basename(outputZip)}`)
      .sort()
      .join('\n')
    await writeFile(path.join(options.outputDir, 'SHA256SUMS'), `${sums}\n`)

    if (options.fixturesDir) {
      await mkdir(path.dirname(options.fixturesDir), { recursive: true })
      await rm(options.fixturesDir, { recursive: true, force: true })
      await rename(fixtureWorking, options.fixturesDir)
    }
  } finally {
    await rm(workingRoot, { recursive: true, force: true })
  }

  const validationOutput = run(process.execPath, [
    path.join(repoRoot, 'scripts', 'validate-legacy-app-packages.mjs'),
    '--dir',
    options.outputDir,
    ...(options.baselineRoot ? ['--baseline-root', options.baselineRoot] : []),
  ])
  process.stdout.write(validationOutput)

  for (const { variant, outputZip, hash } of results) {
    console.log(`${variant.productName} ${variant.candidateVersion}`)
    console.log(`  ${outputZip}`)
    console.log(`  SHA-256 ${hash}`)
  }
  if (options.fixturesDir) console.log(`Test fixtures: ${options.fixturesDir}`)
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error)
  process.exitCode = 1
})
