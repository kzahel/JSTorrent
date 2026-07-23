#!/usr/bin/env node

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const repositoryRoot = path.resolve(scriptDirectory, '..')
const activeFiles = new Set()
const failures = []

const skippedDirectories = new Set([
  '.git',
  '.venv',
  'archive',
  'build',
  'dist',
  'node_modules',
  'target',
])

function repositoryPath(filePath) {
  return path.relative(repositoryRoot, filePath).split(path.sep).join('/')
}

function addIfPresent(relativePath) {
  const absolutePath = path.join(repositoryRoot, relativePath)
  if (fs.existsSync(absolutePath)) {
    activeFiles.add(absolutePath)
  }
}

function collectReadmes(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const absolutePath = path.join(directory, entry.name)
    const relativePath = repositoryPath(absolutePath)
    const parts = relativePath.split('/')

    if (entry.isDirectory()) {
      if (
        skippedDirectories.has(entry.name) ||
        entry.name === 'docs_archive' ||
        relativePath === 'docs/archive' ||
        relativePath.startsWith('docs/archive/') ||
        relativePath.includes('quickjs-ng')
      ) {
        continue
      }
      collectReadmes(absolutePath)
      continue
    }

    if (entry.isFile() && entry.name === 'README.md' && !parts.includes('archive')) {
      activeFiles.add(absolutePath)
    }
  }
}

function collectMarkdown(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const absolutePath = path.join(directory, entry.name)
    if (entry.isDirectory()) {
      collectMarkdown(absolutePath)
    } else if (entry.isFile() && entry.name.endsWith('.md')) {
      activeFiles.add(absolutePath)
    }
  }
}

for (const file of ['README.md', 'DEVELOPMENT.md', 'CLAUDE.md']) {
  addIfPresent(file)
}

collectReadmes(repositoryRoot)
collectMarkdown(path.join(repositoryRoot, 'docs/topics'))
collectMarkdown(path.join(repositoryRoot, 'docs/contracts'))

for (const file of [
  'docs/README.md',
  'docs/archive/README.md',
  'docs/reference/README.md',
  'docs/reference/beps/README.md',
  'desktop/tauri-app/SIDECARS.md',
  'packages/engine/CLAUDE.md',
]) {
  addIfPresent(file)
}

function lineNumber(text, offset) {
  return text.slice(0, offset).split('\n').length
}

function report(file, text, offset, message) {
  failures.push(`${repositoryPath(file)}:${lineNumber(text, offset)}: ${message}`)
}

function checkLocalLinks(file, text) {
  const linkPattern = /!?\[[^\]]*]\(([^)]+)\)/g

  for (const match of text.matchAll(linkPattern)) {
    let target = match[1].trim()
    if (target.startsWith('<') && target.endsWith('>')) {
      target = target.slice(1, -1)
    }

    target = target.split(/\s+["']/)[0]
    const targetWithoutAnchor = target.split('#')[0]
    if (!targetWithoutAnchor || /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(targetWithoutAnchor)) {
      continue
    }

    let decodedTarget
    try {
      decodedTarget = decodeURIComponent(targetWithoutAnchor)
    } catch {
      report(file, text, match.index, `invalid encoded link: ${target}`)
      continue
    }

    const resolved = decodedTarget.startsWith('/')
      ? path.join(repositoryRoot, decodedTarget.slice(1))
      : path.resolve(path.dirname(file), decodedTarget)

    if (!fs.existsSync(resolved)) {
      report(file, text, match.index, `missing local link target: ${target}`)
    }
  }
}

function checkScriptReferences(file, text) {
  const scriptPattern =
    /(?:^|[\s`("'=])((?:\.\/|\.\.\/)?(?:scripts|android\/scripts|desktop\/scripts|desktop\/tauri-app\/scripts|ios\/scripts|packages\/engine\/scripts)\/[A-Za-z0-9_./-]+\.(?:sh|mjs|cjs|ts|py|ps1|bat))/gm

  for (const match of text.matchAll(scriptPattern)) {
    const reference = match[1]
    const candidates = [
      path.resolve(repositoryRoot, reference),
      path.resolve(path.dirname(file), reference),
    ]

    if (!candidates.some((candidate) => fs.existsSync(candidate))) {
      report(file, text, match.index, `missing referenced script: ${reference}`)
    }
  }
}

function checkPortability(file, text) {
  const homePathPattern = /(?:\/Users\/[^/\s`)]+|\/home\/[^/\s`)]+)\//g

  for (const match of text.matchAll(homePathPattern)) {
    report(file, text, match.index, `machine-specific absolute path: ${match[0]}`)
  }
}

for (const file of [...activeFiles].sort()) {
  const text = fs.readFileSync(file, 'utf8')
  checkLocalLinks(file, text)
  checkScriptReferences(file, text)
  checkPortability(file, text)
}

if (failures.length > 0) {
  console.error('Active documentation check failed:\n')
  for (const failure of failures) {
    console.error(`- ${failure}`)
  }
  process.exit(1)
}

console.log(`Active documentation check passed (${activeFiles.size} files).`)
