import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, it, expect } from 'vitest'
import { validateSearchResult } from '../../src'
import { loadPlugin, runPlugin } from '../../src/node'
import { normalizeSearchPluginManifest } from '../../src/validation/manifest'

const pluginPath = resolve(__dirname, '../../../../search-plugins/internet-archive.js')
const source = readFileSync(pluginPath, 'utf-8')

describe('Internet Archive plugin', () => {
  describe('manifest', () => {
    it('loads and has a valid manifest', () => {
      const module = loadPlugin(source)
      expect(module.manifest).toBeDefined()
      expect(module.manifest.name).toBe('Internet Archive')
      expect(module.manifest.hosts).toEqual(['archive.org'])
      expect(module.manifest.id).toBe('org.archive.search')
    })

    it('manifest passes normalization', () => {
      const module = loadPlugin(source)
      const normalized = normalizeSearchPluginManifest(module.manifest)
      expect(normalized.name).toBe('Internet Archive')
      expect(normalized.hosts).toEqual(['archive.org'])
    })

    it('has expected categories', () => {
      const module = loadPlugin(source)
      expect(module.manifest.categories).toEqual(['all', 'movies', 'music', 'books', 'software'])
    })

    it('exports a search function', () => {
      const module = loadPlugin(source)
      expect(typeof module.search).toBe('function')
    })
  })

  describe('search (live)', () => {
    it.skip('returns results for a movie search', { timeout: 20000 }, async () => {
      const result = await runPlugin({
        source,
        input: { query: 'night of the living dead', category: 'movies' },
        enforceHosts: true,
        timeoutMs: 15000,
      })

      expect(result.trace.ok).toBe(true)
      expect(result.trace.results.length).toBeGreaterThan(0)
      expect(result.trace.requests.length).toBeGreaterThan(0)

      for (const r of result.trace.results) {
        const errors = validateSearchResult(r)
        expect(errors).toEqual([])
        expect(r.source).toBe('Internet Archive')
        expect(r.torrentUrl).toMatch(/archive\.org/)
      }
    })

    it('handles empty query gracefully', { timeout: 15000 }, async () => {
      const result = await runPlugin({
        source,
        input: { query: '' },
        enforceHosts: true,
        timeoutMs: 10000,
      })

      expect(result.trace.ok).toBe(false)
      expect(result.trace.error?.phase).toBe('search')
    })
  })
})
