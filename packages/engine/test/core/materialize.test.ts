import { createHash } from 'crypto'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { BtEngine } from '../../src/core/bt-engine'
import { InMemoryFileSystem, MemorySessionStore } from '../../src/adapters/memory'
import { ISocketFactory } from '../../src/interfaces/socket'
import { StorageRootManager } from '../../src/storage/storage-root-manager'
import { Bencode } from '../../src/utils/bencode'

// Mock dependencies
const mockSocketFactory: ISocketFactory = {
  createTcpSocket: vi.fn(),
  createUdpSocket: vi.fn().mockResolvedValue({
    send: vi.fn(),
    onMessage: vi.fn(),
    close: vi.fn(),
  }),
  createTcpServer: vi.fn().mockReturnValue({
    on: vi.fn(),
    listen: vi.fn(),
    address: vi.fn().mockReturnValue({ port: 0 }),
  }),
  wrapTcpSocket: vi.fn(),
}

function sha1(data: Uint8Array): Uint8Array {
  return new Uint8Array(createHash('sha1').update(data).digest())
}

function createEngine(fileSystem: InMemoryFileSystem, sessionStore?: MemorySessionStore): BtEngine {
  const storageRootManager = new StorageRootManager(() => fileSystem)
  storageRootManager.addRoot({ key: 'default', label: 'Default', path: '/downloads' })
  storageRootManager.setDefaultRoot('default')

  return new BtEngine({
    socketFactory: mockSocketFactory,
    storageRootManager,
    sessionStore,
    startSuspended: true,
  })
}

/**
 * Create a multi-file torrent buffer.
 */
function createMultiFileTorrent(opts: {
  name: string
  files: { path: string; length: number }[]
  pieceLength: number
}): Uint8Array {
  const totalSize = opts.files.reduce((sum, f) => sum + f.length, 0)
  const piecesCount = Math.ceil(totalSize / opts.pieceLength)
  const pieces = new Uint8Array(piecesCount * 20)

  return Bencode.encode({
    announce: 'http://tracker.example.com',
    info: {
      name: opts.name,
      'piece length': opts.pieceLength,
      pieces,
      files: opts.files.map((f) => ({
        length: f.length,
        path: f.path.split('/'),
      })),
    },
  })
}

function createMultiFileTorrentWithData(opts: {
  name: string
  files: { path: string; data: Uint8Array }[]
  pieceLength: number
}): Uint8Array {
  const totalSize = opts.files.reduce((sum, file) => sum + file.data.length, 0)
  const concat = new Uint8Array(totalSize)
  let offset = 0

  for (const file of opts.files) {
    concat.set(file.data, offset)
    offset += file.data.length
  }

  const piecesCount = Math.ceil(totalSize / opts.pieceLength)
  const pieces = new Uint8Array(piecesCount * 20)
  for (let i = 0; i < piecesCount; i++) {
    const start = i * opts.pieceLength
    const end = Math.min(start + opts.pieceLength, totalSize)
    pieces.set(sha1(concat.subarray(start, end)), i * 20)
  }

  return Bencode.encode({
    announce: 'http://tracker.example.com',
    info: {
      name: opts.name,
      'piece length': opts.pieceLength,
      pieces,
      files: opts.files.map((file) => ({
        length: file.data.length,
        path: file.path.split('/'),
      })),
    },
  })
}

describe('Materialization', () => {
  let fileSystem: InMemoryFileSystem
  let engine: BtEngine

  beforeEach(() => {
    fileSystem = new InMemoryFileSystem()
    engine = createEngine(fileSystem)
  })

  describe('materializeEligiblePieces()', () => {
    it('materializes piece when file is un-skipped', async () => {
      // Create torrent with boundary piece scenario
      // File A: 0-50000, File B: 50000-100000
      // Piece 3 (49152-65536) spans both files
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'fileA.txt', length: 50000 },
          { path: 'fileB.txt', length: 50000 },
        ],
        pieceLength: 16384,
      })

      // Create parent directory for engine writes during materialization
      await fileSystem.mkdir('test-folder')

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Skip file A - piece 3 becomes boundary
      torrent.setFilePriority(0, 1)
      expect(torrent.pieceClassification[3]).toBe('boundary')

      // Simulate having piece 3 verified and in .parts
      // @ts-expect-error - accessing private member for testing
      torrent._partsFilePieces.add(3)
      torrent.bitfield!.set(3, true)

      // Create a mock PartsFile with the piece data
      const pieceData = new Uint8Array(16384).fill(0xab)
      // @ts-expect-error - accessing private member for testing
      if (torrent._partsFile) {
        // @ts-expect-error - accessing private member for testing
        torrent._partsFile.addPiece(3, pieceData)
      }

      // Un-skip file A - should trigger materialization
      torrent.setFilePriority(0, 0)

      // Wait a tick for async materialization
      await new Promise((resolve) => setTimeout(resolve, 50))

      // Piece should no longer be in partsFilePieces
      expect(torrent.partsFilePieces.has(3)).toBe(false)
      // Piece should still be in bitfield (verified)
      expect(torrent.bitfield!.get(3)).toBe(true)
      // Classification should now be 'wanted'
      expect(torrent.pieceClassification[3]).toBe('wanted')
    })

    it('does not materialize if piece classification is still boundary', async () => {
      // File A, B, C scenario where un-skipping one still leaves boundary
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'fileA.txt', length: 10000 },
          { path: 'fileB.txt', length: 10000 },
          { path: 'fileC.txt', length: 10000 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Skip files A and C
      torrent.setFilePriority(0, 1)
      torrent.setFilePriority(2, 1)

      // Piece 0 spans A and B - boundary
      // Piece 1 spans B and C - boundary
      expect(torrent.pieceClassification[0]).toBe('boundary')

      // Simulate piece 0 in .parts
      // @ts-expect-error - accessing private member for testing
      torrent._partsFilePieces.add(0)
      torrent.bitfield!.set(0, true)

      // Un-skip only file C (piece 0 still touches skipped A)
      torrent.setFilePriority(2, 0)

      await new Promise((resolve) => setTimeout(resolve, 50))

      // Piece 0 should still be in .parts (still touches skipped A)
      expect(torrent.partsFilePieces.has(0)).toBe(true)
    })

    it('returns 0 when no pieces to materialize', async () => {
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'fileA.txt', length: 50000 },
          { path: 'fileB.txt', length: 50000 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // No pieces in .parts
      const count = await torrent.materializeEligiblePieces()
      expect(count).toBe(0)
    })
  })

  describe('canServePiece during materialization', () => {
    it('piece in .parts cannot be served', async () => {
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'fileA.txt', length: 50000 },
          { path: 'fileB.txt', length: 50000 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Piece 3 is verified and in .parts
      torrent.bitfield!.set(3, true)
      // @ts-expect-error - accessing private member for testing
      torrent._partsFilePieces.add(3)

      expect(torrent.canServePiece(3)).toBe(false)
    })

    it('piece becomes serveable after materialization', async () => {
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'fileA.txt', length: 50000 },
          { path: 'fileB.txt', length: 50000 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Initially in .parts
      torrent.bitfield!.set(3, true)
      // @ts-expect-error - accessing private member for testing
      torrent._partsFilePieces.add(3)
      expect(torrent.canServePiece(3)).toBe(false)

      // Simulate materialization (remove from partsFilePieces)
      // @ts-expect-error - accessing private member for testing
      torrent._partsFilePieces.delete(3)
      expect(torrent.canServePiece(3)).toBe(true)
    })
  })

  describe('advertised bitfield after materialization', () => {
    it('advertised bitfield gains the bit after materialization', async () => {
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'fileA.txt', length: 50000 },
          { path: 'fileB.txt', length: 50000 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Piece 3 verified but in .parts
      torrent.bitfield!.set(3, true)
      // @ts-expect-error - accessing private member for testing
      torrent._partsFilePieces.add(3)

      // Advertised should NOT have piece 3
      let advertised = torrent.getAdvertisedBitfield()
      expect(advertised!.get(3)).toBe(false)

      // Materialize (remove from partsFilePieces)
      // @ts-expect-error - accessing private member for testing
      torrent._partsFilePieces.delete(3)

      // Advertised should now have piece 3
      advertised = torrent.getAdvertisedBitfield()
      expect(advertised!.get(3)).toBe(true)
    })
  })

  describe('.parts file cleanup', () => {
    it('piece is removed from .parts tracking after materialization', async () => {
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'fileA.txt', length: 50000 },
          { path: 'fileB.txt', length: 50000 },
        ],
        pieceLength: 16384,
      })

      // Create parent directory for engine writes during materialization
      await fileSystem.mkdir('test-folder')

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Setup: piece 3 is boundary and in .parts
      torrent.setFilePriority(0, 1)
      expect(torrent.pieceClassification[3]).toBe('boundary')

      // @ts-expect-error - accessing private member for testing
      torrent._partsFilePieces.add(3)
      torrent.bitfield!.set(3, true)

      // Setup mock partsFile data
      // @ts-expect-error - accessing private member for testing
      if (torrent._partsFile) {
        // @ts-expect-error - accessing private member for testing
        torrent._partsFile.addPiece(3, new Uint8Array(16384))
      }

      expect(torrent.partsFilePieces.size).toBe(1)

      // Un-skip triggers materialization
      torrent.setFilePriority(0, 0)
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(torrent.partsFilePieces.size).toBe(0)
    })
  })

  describe('edge cases', () => {
    it('handles torrent with no partsFile gracefully', async () => {
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [{ path: 'single.txt', length: 50000 }],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Clear partsFile reference
      // @ts-expect-error - accessing private member for testing
      torrent._partsFile = undefined

      const count = await torrent.materializeEligiblePieces()
      expect(count).toBe(0)
    })

    it('handles empty partsFilePieces set', async () => {
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [{ path: 'single.txt', length: 50000 }],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      expect(torrent.partsFilePieces.size).toBe(0)

      const count = await torrent.materializeEligiblePieces()
      expect(count).toBe(0)
    })
  })

  describe('boundary piece immediate write', () => {
    it('writePieceFilteredByPriority writes only to non-skipped files', async () => {
      // Setup: [large file (skipped), small file (wanted)]
      // File A: 0-50000 (will be skipped), File B: 50000-51000 (wanted)
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'large.dat', length: 50000 },
          { path: 'small.txt', length: 1000 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Create files in the mock filesystem (direct map access)
      // Note: TorrentFile paths are relative (e.g., 'test-folder/large.dat'), not absolute
      const fileAPath = 'test-folder/large.dat'
      const fileBPath = 'test-folder/small.txt'
      fileSystem.files.set(fileAPath, new Uint8Array(50000).fill(0))
      fileSystem.files.set(fileBPath, new Uint8Array(1000).fill(0))

      // Skip file A - piece 3 becomes boundary
      torrent.setFilePriority(0, 1)
      expect(torrent.pieceClassification[3]).toBe('boundary')

      // Verify contentStorage has updated priorities
      const contentStorage = torrent.contentStorage
      if (!contentStorage) throw new Error('contentStorage is undefined')

      // Write piece 3 data using filtered write
      // Piece 3: bytes 49152-65536 in torrent
      // - File A gets bytes 49152-50000 (848 bytes) - should be SKIPPED
      // - File B gets bytes 50000-51000 (1000 bytes) - should be WRITTEN
      const pieceData = new Uint8Array(16384)
      // Fill with pattern so we can verify what was written
      for (let i = 0; i < pieceData.length; i++) {
        pieceData[i] = (i % 256) as number
      }

      await contentStorage.writePieceFilteredByPriority(3, pieceData)

      // Verify file A was NOT modified (should still be all zeros)
      const fileAData = await fileSystem.readFile(fileAPath)
      // Check the last 848 bytes of file A (where piece 3 would overlap)
      const fileAOverlap = fileAData.slice(49152 - 0) // offset 49152 relative to file A start (0)
      expect(fileAOverlap.every((b) => b === 0)).toBe(true)

      // Verify file B WAS modified with the correct data
      const fileBData = await fileSystem.readFile(fileBPath)
      // File B starts at torrent offset 50000, piece 3 starts at 49152
      // So piece data offset for file B start is 50000 - 49152 = 848
      const expectedPattern = pieceData.slice(848, 848 + 1000)
      expect(fileBData).toEqual(expectedPattern)
    })

    it('handles case where only last files are wanted', async () => {
      // This is the user's exact scenario: skip all but last two small files
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'big1.dat', length: 100000 },
          { path: 'big2.dat', length: 100000 },
          { path: 'readme1.txt', length: 500 },
          { path: 'readme2.txt', length: 500 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Create files in mock filesystem (direct map access)
      // Note: TorrentFile paths are relative (e.g., 'test-folder/big1.dat'), not absolute
      fileSystem.files.set('test-folder/big1.dat', new Uint8Array(100000).fill(0))
      fileSystem.files.set('test-folder/big2.dat', new Uint8Array(100000).fill(0))
      fileSystem.files.set('test-folder/readme1.txt', new Uint8Array(500).fill(0))
      fileSystem.files.set('test-folder/readme2.txt', new Uint8Array(500).fill(0))

      // Skip first two large files
      torrent.setFilePriority(0, 1)
      torrent.setFilePriority(1, 1)

      // Find the boundary piece that spans from big2.dat to readme1.txt
      // big1.dat: 0-100000, big2.dat: 100000-200000, readme1.txt: 200000-200500
      // Piece at offset 196608 (piece 12) spans from big2 into readme1
      const boundaryPieceIndex = Math.floor(200000 / 16384) // piece that contains offset 200000
      expect(torrent.pieceClassification[boundaryPieceIndex]).toBe('boundary')

      // Write boundary piece using filtered write
      const contentStorage = torrent.contentStorage
      if (!contentStorage) throw new Error('contentStorage is undefined')
      const pieceData = new Uint8Array(16384).fill(0xab)
      await contentStorage.writePieceFilteredByPriority(boundaryPieceIndex, pieceData)

      // Verify readme1.txt was written to
      const readme1Data = await fileSystem.readFile('test-folder/readme1.txt')
      // At least some bytes should be 0xab now (the part from the boundary piece)
      const hasExpectedData = readme1Data.some((b) => b === 0xab)
      expect(hasExpectedData).toBe(true)

      // Verify big2.dat was NOT modified (should still be all zeros)
      const big2Data = await fileSystem.readFile('test-folder/big2.dat')
      expect(big2Data.every((b) => b === 0)).toBe(true)
    })

    it('handles small file in middle of piece with surrounding files skipped', async () => {
      // Edge case: small file entirely within one piece, surrounded by skipped files
      // Layout:
      //   big1.dat: 0-30000 (skipped)
      //   tiny.txt: 30000-30100 (100 bytes, WANTED)
      //   big2.dat: 30100-60000 (skipped)
      // With pieceLength 16384:
      //   Piece 0: 0-16384 (entirely in big1) -> blacklisted
      //   Piece 1: 16384-32768 (spans big1, tiny, big2) -> boundary
      //   Piece 2: 32768-48768 (spans big2) -> blacklisted
      //   Piece 3: 48768-60000 (in big2) -> blacklisted
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'big1.dat', length: 30000 },
          { path: 'tiny.txt', length: 100 },
          { path: 'big2.dat', length: 29900 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Create files in mock filesystem
      fileSystem.files.set('test-folder/big1.dat', new Uint8Array(30000).fill(0))
      fileSystem.files.set('test-folder/tiny.txt', new Uint8Array(100).fill(0))
      fileSystem.files.set('test-folder/big2.dat', new Uint8Array(29900).fill(0))

      // Skip big1 and big2, keep only tiny.txt
      torrent.setFilePriority(0, 1) // big1 skipped
      torrent.setFilePriority(2, 1) // big2 skipped

      // Piece 1 (16384-32768) should be boundary since it spans:
      // - big1 (skipped): 16384-30000
      // - tiny.txt (wanted): 30000-30100
      // - big2 (skipped): 30100-32768
      expect(torrent.pieceClassification[1]).toBe('boundary')

      // Piece 0 should be blacklisted (entirely in skipped big1)
      expect(torrent.pieceClassification[0]).toBe('blacklisted')

      const contentStorage = torrent.contentStorage
      if (!contentStorage) throw new Error('contentStorage is undefined')

      // Write boundary piece 1 using filtered write
      const pieceData = new Uint8Array(16384).fill(0xcd)
      await contentStorage.writePieceFilteredByPriority(1, pieceData)

      // Verify tiny.txt WAS written to
      // tiny.txt is at torrent offset 30000-30100
      // Piece 1 starts at 16384, so tiny.txt starts at piece offset 30000-16384 = 13616
      const tinyData = await fileSystem.readFile('test-folder/tiny.txt')
      expect(tinyData.every((b) => b === 0xcd)).toBe(true)

      // Verify big1.dat was NOT modified (should still be all zeros)
      const big1Data = await fileSystem.readFile('test-folder/big1.dat')
      expect(big1Data.every((b) => b === 0)).toBe(true)

      // Verify big2.dat was NOT modified (should still be all zeros)
      const big2Data = await fileSystem.readFile('test-folder/big2.dat')
      expect(big2Data.every((b) => b === 0)).toBe(true)
    })

    it('handles skipped file in middle of piece with wanted files on both ends', async () => {
      // Edge case: piece spans [wanted file] -> [skipped file] -> [wanted file]
      // This tests that the middle portion is correctly skipped while both ends are written
      // Layout:
      //   part1.dat: 0-10000 (WANTED)
      //   skip.dat:  10000-20000 (SKIPPED)
      //   part2.dat: 20000-30000 (WANTED)
      // With pieceLength 16384:
      //   Piece 0: 0-16384 (spans part1, skip) -> boundary
      //   Piece 1: 16384-30000 (spans skip, part2) -> boundary
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'part1.dat', length: 10000 },
          { path: 'skip.dat', length: 10000 },
          { path: 'part2.dat', length: 10000 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      // Create files in mock filesystem
      fileSystem.files.set('test-folder/part1.dat', new Uint8Array(10000).fill(0))
      fileSystem.files.set('test-folder/skip.dat', new Uint8Array(10000).fill(0))
      fileSystem.files.set('test-folder/part2.dat', new Uint8Array(10000).fill(0))

      // Skip only the middle file
      torrent.setFilePriority(1, 1) // skip.dat skipped

      // Piece 0 (0-16384) should be boundary since it spans:
      // - part1.dat (wanted): 0-10000
      // - skip.dat (skipped): 10000-16384
      expect(torrent.pieceClassification[0]).toBe('boundary')

      // Piece 1 (16384-30000) should be boundary since it spans:
      // - skip.dat (skipped): 16384-20000
      // - part2.dat (wanted): 20000-30000
      expect(torrent.pieceClassification[1]).toBe('boundary')

      const contentStorage = torrent.contentStorage
      if (!contentStorage) throw new Error('contentStorage is undefined')

      // Write boundary piece 0 using filtered write
      const piece0Data = new Uint8Array(16384).fill(0xaa)
      await contentStorage.writePieceFilteredByPriority(0, piece0Data)

      // Write boundary piece 1 using filtered write
      const piece1Data = new Uint8Array(16384).fill(0xbb)
      await contentStorage.writePieceFilteredByPriority(1, piece1Data)

      // Verify part1.dat WAS written to (should be all 0xaa)
      const part1Data = await fileSystem.readFile('test-folder/part1.dat')
      expect(part1Data.every((b) => b === 0xaa)).toBe(true)

      // Verify skip.dat was NOT modified (should still be all zeros)
      const skipData = await fileSystem.readFile('test-folder/skip.dat')
      expect(skipData.every((b) => b === 0)).toBe(true)

      // Verify part2.dat WAS written to (should be all 0xbb)
      const part2Data = await fileSystem.readFile('test-folder/part2.dat')
      expect(part2Data.every((b) => b === 0xbb)).toBe(true)
    })

    it('setFilePriorities propagates to contentStorage', async () => {
      const buffer = createMultiFileTorrent({
        name: 'test-folder',
        files: [
          { path: 'fileA.txt', length: 50000 },
          { path: 'fileB.txt', length: 50000 },
        ],
        pieceLength: 16384,
      })

      const { torrent } = await engine.addTorrent(buffer)
      if (!torrent) throw new Error('Torrent is null')

      const contentStorage = torrent.contentStorage
      if (!contentStorage) throw new Error('contentStorage is undefined')

      // Initially all files should have priority 0 (wanted)
      // @ts-expect-error - accessing private member for testing
      expect(contentStorage.filePriorities).toEqual([0, 0])

      // Skip file A
      torrent.setFilePriority(0, 1)

      // Verify contentStorage was updated
      // @ts-expect-error - accessing private member for testing
      expect(contentStorage.filePriorities).toEqual([1, 0])
    })
  })

  describe('boundary piece regression coverage', () => {
    it('stress tests one-by-one unskip with mixed file sizes and verifies persisted bytes', async () => {
      function buildFiles(seed: number, count: number): { path: string; data: Uint8Array }[] {
        return Array.from({ length: count }, (_, fileIndex) => {
          const length =
            17 +
            (((seed * 97 + fileIndex * 193 + (fileIndex % 5) * 389) % 2100) as number) +
            ((fileIndex * 11 + seed) % 37)
          const data = new Uint8Array(length)
          for (let i = 0; i < length; i++) {
            data[i] = (seed * 29 + fileIndex * 53 + i * 17 + (i >> 2)) & 0xff
          }
          return {
            path: `file-${fileIndex.toString().padStart(2, '0')}.bin`,
            data,
          }
        })
      }

      function alternatingOrder(count: number): number[] {
        const evens: number[] = []
        const odds: number[] = []
        for (let i = 0; i < count; i++) {
          if (i % 2 === 0) evens.push(i)
          else odds.push(i)
        }
        return [...evens, ...odds]
      }

      function centerOutOrder(count: number): number[] {
        const order: number[] = []
        const used = new Set<number>()
        const center = Math.floor((count - 1) / 2)
        for (let distance = 0; order.length < count; distance++) {
          const left = center - distance
          const right = center + distance + (count % 2 === 0 ? 1 : 0)
          if (left >= 0 && !used.has(left)) {
            order.push(left)
            used.add(left)
          }
          if (right < count && !used.has(right)) {
            order.push(right)
            used.add(right)
          }
        }
        return order
      }

      async function runScenario(scenario: {
        name: string
        pieceLength: number
        files: { path: string; data: Uint8Array }[]
        unskipOrder: number[]
      }): Promise<void> {
        const localFileSystem = new InMemoryFileSystem()
        const localEngine = createEngine(localFileSystem)
        const torrentBuffer = createMultiFileTorrentWithData({
          name: scenario.name,
          files: scenario.files,
          pieceLength: scenario.pieceLength,
        })

        const totalSize = scenario.files.reduce((sum, file) => sum + file.data.length, 0)
        const allData = new Uint8Array(totalSize)
        let globalOffset = 0
        for (const file of scenario.files) {
          allData.set(file.data, globalOffset)
          globalOffset += file.data.length
        }

        await localFileSystem.mkdir(scenario.name)

        const { torrent } = await localEngine.addTorrent(torrentBuffer)
        if (!torrent) throw new Error(`Torrent is null for scenario ${scenario.name}`)

        const allSkipped = new Map<number, number>()
        scenario.files.forEach((_, index) => allSkipped.set(index, 1))
        await torrent.setFilePrioritiesAsync(allSkipped)

        async function writeVerifiedPieceForCurrentPriorities(pieceIndex: number): Promise<void> {
          if (torrent.hasPiece(pieceIndex)) return
          const start = pieceIndex * scenario.pieceLength
          const end = Math.min(start + torrent.getPieceLength(pieceIndex), allData.length)
          const pieceData = allData.slice(start, end)
          const classification = torrent.pieceClassification[pieceIndex]

          if (classification === 'boundary') {
            // @ts-expect-error - accessing private member for targeted boundary-piece test coverage
            const result = await torrent.verifyAndWriteBoundaryPiece(
              pieceIndex,
              pieceData,
              torrent.getPieceHash(pieceIndex),
              [],
            )
            expect(result).toEqual({ success: true, bytesWritten: pieceData.length })
          } else if (classification === 'wanted') {
            if (!torrent.contentStorage) throw new Error('contentStorage is undefined')
            await torrent.contentStorage.writePiece(pieceIndex, pieceData)
          } else {
            throw new Error(
              `Scenario ${scenario.name}: piece ${pieceIndex} unexpectedly blacklisted while downloading file`,
            )
          }

          torrent.markPieceVerified(pieceIndex)
        }

        function getFilePieceRange(fileIndex: number): { startPiece: number; endPiece: number } {
          const file = torrent.files[fileIndex]
          if (!file) throw new Error(`Missing file ${fileIndex} in scenario ${scenario.name}`)
          const startPiece = Math.floor(file.offset / scenario.pieceLength)
          const endPiece = Math.floor((file.offset + file.length - 1) / scenario.pieceLength)
          return { startPiece, endPiece }
        }

        const completedFiles = new Set<number>()

        for (const fileIndex of scenario.unskipOrder) {
          await torrent.setFilePriorityAsync(fileIndex, 0)

          const { startPiece, endPiece } = getFilePieceRange(fileIndex)
          for (let pieceIndex = startPiece; pieceIndex <= endPiece; pieceIndex++) {
            await writeVerifiedPieceForCurrentPriorities(pieceIndex)
          }

          completedFiles.add(fileIndex)
          expect(torrent.isFileComplete(fileIndex)).toBe(true)

          for (const completedFileIndex of completedFiles) {
            const expected = scenario.files[completedFileIndex].data
            const persisted = await localFileSystem.readFile(
              `${scenario.name}/${scenario.files[completedFileIndex].path}`,
            )
            expect(persisted).toEqual(expected)
          }
        }

        expect(torrent.partsFilePieces.size).toBe(0)
        expect(await localFileSystem.exists(`${torrent.infoHashStr}.parts`)).toBe(false)

        await torrent.recheckData()
        expect(torrent.completedPiecesCount).toBe(torrent.piecesCount)

        for (const file of scenario.files) {
          const persisted = await localFileSystem.readFile(`${scenario.name}/${file.path}`)
          expect(persisted).toEqual(file.data)
        }
      }

      const scenarioAFileCount = 30
      const scenarioBFileCount = 26
      const scenarioCFileCount = 34
      const scenarioDFileCount = 22

      const scenarios = [
        {
          name: 'StressBoundarySeq257',
          pieceLength: 257,
          files: buildFiles(11, scenarioAFileCount),
          unskipOrder: Array.from({ length: scenarioAFileCount }, (_, index) => index),
        },
        {
          name: 'StressBoundaryReverse97',
          pieceLength: 97,
          files: buildFiles(23, scenarioBFileCount),
          unskipOrder: Array.from(
            { length: scenarioBFileCount },
            (_, index) => scenarioBFileCount - 1 - index,
          ),
        },
        {
          name: 'StressBoundaryAlt509',
          pieceLength: 509,
          files: buildFiles(37, scenarioCFileCount),
          unskipOrder: alternatingOrder(scenarioCFileCount),
        },
        {
          name: 'StressBoundaryCenter1021',
          pieceLength: 1021,
          files: buildFiles(41, scenarioDFileCount),
          unskipOrder: centerOutOrder(scenarioDFileCount),
        },
      ]

      for (const scenario of scenarios) {
        await runScenario(scenario)
      }
    }, 30_000)

    it('exports newly unskipped file spans while piece remains in .parts', async () => {
      const fileAData = new Uint8Array(100)
      const skippedData = new Uint8Array(10)
      const fileCData = new Uint8Array(120)
      for (let i = 0; i < fileAData.length; i++) fileAData[i] = i & 0xff
      for (let i = 0; i < skippedData.length; i++) skippedData[i] = (i + 100) & 0xff
      for (let i = 0; i < fileCData.length; i++) fileCData[i] = (i + 110) & 0xff

      const torrentBuffer = createMultiFileTorrentWithData({
        name: 'PartialExportTorrent',
        files: [
          { path: 'partA.bin', data: fileAData },
          { path: 'skip.txt', data: skippedData },
          { path: 'partC.bin', data: fileCData },
        ],
        pieceLength: 256,
      })

      const { torrent } = await engine.addTorrent(torrentBuffer)
      if (!torrent) throw new Error('Torrent is null')

      await fileSystem.mkdir('PartialExportTorrent')

      torrent.setFilePriority(1, 1)
      torrent.setFilePriority(2, 1)
      expect(torrent.pieceClassification[0]).toBe('boundary')

      const pieceData = new Uint8Array([...fileAData, ...skippedData, ...fileCData])
      // @ts-expect-error - accessing private member for testing
      const result = await torrent.verifyAndWriteBoundaryPiece(
        0,
        pieceData,
        torrent.getPieceHash(0),
        [],
      )
      expect(result).toEqual({ success: true, bytesWritten: pieceData.length })
      torrent.bitfield!.set(0, true)

      expect(await fileSystem.exists('PartialExportTorrent/partC.bin')).toBe(false)

      torrent.setFilePriority(2, 0)
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(torrent.pieceClassification[0]).toBe('boundary')
      expect(torrent.partsFilePieces.has(0)).toBe(true)
      expect(torrent.canServePiece(0)).toBe(false)
      expect(torrent.getAdvertisedBitfield()!.get(0)).toBe(false)

      const writtenFileC = await fileSystem.readFile('PartialExportTorrent/partC.bin')
      expect(writtenFileC).toEqual(fileCData)
    })

    it('round-trips skipped boundary data from .parts to disk after unskip', async () => {
      const fileAData = new Uint8Array(150)
      const fileBData = new Uint8Array(250)
      for (let i = 0; i < fileAData.length; i++) fileAData[i] = i & 0xff
      for (let i = 0; i < fileBData.length; i++) fileBData[i] = (i + fileAData.length) & 0xff

      const torrentBuffer = createMultiFileTorrentWithData({
        name: 'BoundaryTorrent',
        files: [
          { path: 'fileA.bin', data: fileAData },
          { path: 'fileB.bin', data: fileBData },
        ],
        pieceLength: 100,
      })

      const { torrent } = await engine.addTorrent(torrentBuffer)
      if (!torrent) throw new Error('Torrent is null')

      await fileSystem.mkdir('BoundaryTorrent')

      torrent.setFilePriority(1, 1)
      expect(torrent.pieceClassification[1]).toBe('boundary')

      const pieceData = new Uint8Array([...fileAData.slice(100), ...fileBData.slice(0, 50)])
      // @ts-expect-error - accessing private member for testing
      const result = await torrent.verifyAndWriteBoundaryPiece(
        1,
        pieceData,
        torrent.getPieceHash(1),
        [],
      )
      expect(result).toEqual({ success: true, bytesWritten: 100 })
      torrent.bitfield!.set(1, true)

      const writtenFileA = await fileSystem.readFile('BoundaryTorrent/fileA.bin')
      expect(writtenFileA.slice(100)).toEqual(fileAData.slice(100))
      expect(await fileSystem.exists('BoundaryTorrent/fileB.bin')).toBe(false)

      torrent.setFilePriority(1, 0)
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(torrent.partsFilePieces.has(1)).toBe(false)
      expect(torrent.pieceClassification[1]).toBe('wanted')

      const writtenFileB = await fileSystem.readFile('BoundaryTorrent/fileB.bin')
      expect(writtenFileB.slice(0, 50)).toEqual(fileBData.slice(0, 50))

      const roundTrip = await torrent.readFileBytes(1, 0, 50)
      expect(roundTrip).toEqual(fileBData.slice(0, 50))
    })

    it('setFilePriorityAsync waits for boundary materialization before resolving', async () => {
      const fileAData = new Uint8Array(150)
      const fileBData = new Uint8Array(250)
      for (let i = 0; i < fileAData.length; i++) fileAData[i] = i & 0xff
      for (let i = 0; i < fileBData.length; i++) fileBData[i] = (i + fileAData.length) & 0xff

      const torrentBuffer = createMultiFileTorrentWithData({
        name: 'BoundaryAsyncTorrent',
        files: [
          { path: 'fileA.bin', data: fileAData },
          { path: 'fileB.bin', data: fileBData },
        ],
        pieceLength: 100,
      })

      const { torrent } = await engine.addTorrent(torrentBuffer)
      if (!torrent) throw new Error('Torrent is null')

      await fileSystem.mkdir('BoundaryAsyncTorrent')

      torrent.setFilePriority(1, 1)
      const pieceData = new Uint8Array([...fileAData.slice(100), ...fileBData.slice(0, 50)])
      // @ts-expect-error - accessing private member for testing
      await torrent.verifyAndWriteBoundaryPiece(1, pieceData, torrent.getPieceHash(1), [])
      torrent.bitfield!.set(1, true)

      await torrent.setFilePriorityAsync(1, 0)

      expect(torrent.partsFilePieces.has(1)).toBe(false)
      const writtenFileB = await fileSystem.readFile('BoundaryAsyncTorrent/fileB.bin')
      expect(writtenFileB.slice(0, 50)).toEqual(fileBData.slice(0, 50))
    })

    it('materializes wanted .parts pieces on session restore', async () => {
      const sessionStore = new MemorySessionStore()
      const sharedFs = new InMemoryFileSystem()
      const engine1 = createEngine(sharedFs, sessionStore)

      const fileAData = new Uint8Array(150)
      const fileBData = new Uint8Array(250)
      for (let i = 0; i < fileAData.length; i++) fileAData[i] = i & 0xff
      for (let i = 0; i < fileBData.length; i++) fileBData[i] = (i + fileAData.length) & 0xff

      const torrentBuffer = createMultiFileTorrentWithData({
        name: 'RestoreTorrent',
        files: [
          { path: 'fileA.bin', data: fileAData },
          { path: 'fileB.bin', data: fileBData },
        ],
        pieceLength: 100,
      })

      const { torrent: torrent1 } = await engine1.addTorrent(torrentBuffer)
      if (!torrent1) throw new Error('Torrent is null')

      await sharedFs.mkdir('RestoreTorrent')

      torrent1.setFilePriority(1, 1)
      const pieceData = new Uint8Array([...fileAData.slice(100), ...fileBData.slice(0, 50)])
      // @ts-expect-error - accessing private member for testing
      await torrent1.verifyAndWriteBoundaryPiece(1, pieceData, torrent1.getPieceHash(1), [])
      torrent1.bitfield!.set(1, true)

      const infoHash = Buffer.from(torrent1.infoHash).toString('hex')
      await engine1.sessionPersistence.saveTorrentList()
      await engine1.sessionPersistence.saveTorrentFile(infoHash, torrentBuffer)
      await engine1.sessionPersistence.saveTorrentState(torrent1)

      const stateKey = `torrent:${infoHash}:state`
      const state = await sessionStore.getJson<{
        bitfield?: string
        downloaded: number
        filePriorities?: number[]
        forceActive?: boolean
        pieceCount?: number
        queuePosition?: number
        storageKey?: string
        updatedAt: number
        uploaded: number
        userState: string
      }>(stateKey)
      if (!state) throw new Error('Expected persisted state')
      await sessionStore.setJson(stateKey, {
        ...state,
        filePriorities: [0, 0],
      })

      const engine2 = createEngine(sharedFs, sessionStore)
      await engine2.restoreSession()

      const torrent2 = engine2.torrents[0]
      expect(torrent2.partsFilePieces.has(1)).toBe(false)
      expect(torrent2.pieceClassification[1]).toBe('wanted')

      const restoredFileB = await sharedFs.readFile('RestoreTorrent/fileB.bin')
      expect(restoredFileB.slice(0, 50)).toEqual(fileBData.slice(0, 50))
    })

    it('exports wanted spans from existing boundary .parts pieces on session restore', async () => {
      const sessionStore = new MemorySessionStore()
      const sharedFs = new InMemoryFileSystem()
      const engine1 = createEngine(sharedFs, sessionStore)

      const fileAData = new Uint8Array(100)
      const skippedData = new Uint8Array(10)
      const fileCData = new Uint8Array(120)
      for (let i = 0; i < fileAData.length; i++) fileAData[i] = i & 0xff
      for (let i = 0; i < skippedData.length; i++) skippedData[i] = (i + 100) & 0xff
      for (let i = 0; i < fileCData.length; i++) fileCData[i] = (i + 110) & 0xff

      const torrentBuffer = createMultiFileTorrentWithData({
        name: 'RestoreBoundaryTorrent',
        files: [
          { path: 'partA.bin', data: fileAData },
          { path: 'skip.txt', data: skippedData },
          { path: 'partC.bin', data: fileCData },
        ],
        pieceLength: 256,
      })

      const { torrent: torrent1 } = await engine1.addTorrent(torrentBuffer)
      if (!torrent1) throw new Error('Torrent is null')

      await sharedFs.mkdir('RestoreBoundaryTorrent')

      torrent1.setFilePriority(1, 1)
      torrent1.setFilePriority(2, 1)
      const pieceData = new Uint8Array([...fileAData, ...skippedData, ...fileCData])
      // @ts-expect-error - accessing private member for testing
      await torrent1.verifyAndWriteBoundaryPiece(0, pieceData, torrent1.getPieceHash(0), [])
      torrent1.bitfield!.set(0, true)

      const infoHash = Buffer.from(torrent1.infoHash).toString('hex')
      await engine1.sessionPersistence.saveTorrentList()
      await engine1.sessionPersistence.saveTorrentFile(infoHash, torrentBuffer)
      await engine1.sessionPersistence.saveTorrentState(torrent1)

      const stateKey = `torrent:${infoHash}:state`
      const state = await sessionStore.getJson<{
        bitfield?: string
        downloaded: number
        filePriorities?: number[]
        forceActive?: boolean
        pieceCount?: number
        queuePosition?: number
        storageKey?: string
        updatedAt: number
        uploaded: number
        userState: string
      }>(stateKey)
      if (!state) throw new Error('Expected persisted state')
      await sessionStore.setJson(stateKey, {
        ...state,
        filePriorities: [0, 1, 0],
      })

      const engine2 = createEngine(sharedFs, sessionStore)
      await engine2.restoreSession()

      const torrent2 = engine2.torrents[0]
      expect(torrent2.partsFilePieces.has(0)).toBe(true)
      expect(torrent2.pieceClassification[0]).toBe('boundary')
      expect(torrent2.canServePiece(0)).toBe(false)

      const restoredFileC = await sharedFs.readFile('RestoreBoundaryTorrent/partC.bin')
      expect(restoredFileC).toEqual(fileCData)
    })

    it('readFileBytes prefers .parts data for boundary pieces over sparse on-disk bytes', async () => {
      const fileAData = new Uint8Array(100)
      const skippedData = new Uint8Array(10)
      const fileCData = new Uint8Array(120)
      for (let i = 0; i < fileAData.length; i++) fileAData[i] = i & 0xff
      for (let i = 0; i < skippedData.length; i++) skippedData[i] = (i + 100) & 0xff
      for (let i = 0; i < fileCData.length; i++) fileCData[i] = (i + 110) & 0xff

      const torrentBuffer = createMultiFileTorrentWithData({
        name: 'ReadFromPartsTorrent',
        files: [
          { path: 'partA.bin', data: fileAData },
          { path: 'skip.txt', data: skippedData },
          { path: 'partC.bin', data: fileCData },
        ],
        pieceLength: 256,
      })

      const { torrent } = await engine.addTorrent(torrentBuffer)
      if (!torrent) throw new Error('Torrent is null')

      await fileSystem.mkdir('ReadFromPartsTorrent')

      torrent.setFilePriority(1, 1)
      torrent.setFilePriority(2, 1)

      const pieceData = new Uint8Array([...fileAData, ...skippedData, ...fileCData])
      // @ts-expect-error - accessing private member for testing
      await torrent.verifyAndWriteBoundaryPiece(0, pieceData, torrent.getPieceHash(0), [])
      torrent.bitfield!.set(0, true)

      torrent.setFilePriority(2, 0)
      await new Promise((resolve) => setTimeout(resolve, 50))

      const fileCPath = 'ReadFromPartsTorrent/partC.bin'
      const sparseBytes = new Uint8Array(fileCData.length)
      fileSystem.files.set(fileCPath, sparseBytes)

      const readBack = await torrent.readFileBytes(2, 0, fileCData.length)
      expect(readBack).toEqual(fileCData)
      expect(torrent.partsFilePieces.has(0)).toBe(true)
    })

    it('recheckData detects corruption in a materialized boundary region', async () => {
      const fileAData = new Uint8Array(150)
      const fileBData = new Uint8Array(250)
      for (let i = 0; i < fileAData.length; i++) fileAData[i] = i & 0xff
      for (let i = 0; i < fileBData.length; i++) fileBData[i] = (i + fileAData.length) & 0xff

      const torrentBuffer = createMultiFileTorrentWithData({
        name: 'RecheckTorrent',
        files: [
          { path: 'fileA.bin', data: fileAData },
          { path: 'fileB.bin', data: fileBData },
        ],
        pieceLength: 100,
      })

      const { torrent } = await engine.addTorrent(torrentBuffer)
      if (!torrent) throw new Error('Torrent is null')

      await fileSystem.mkdir('RecheckTorrent')

      torrent.setFilePriority(1, 1)
      const pieceData = new Uint8Array([...fileAData.slice(100), ...fileBData.slice(0, 50)])
      // @ts-expect-error - accessing private member for testing
      await torrent.verifyAndWriteBoundaryPiece(1, pieceData, torrent.getPieceHash(1), [])
      torrent.bitfield!.set(1, true)

      torrent.setFilePriority(1, 0)
      await new Promise((resolve) => setTimeout(resolve, 50))

      const fileBPath = 'RecheckTorrent/fileB.bin'
      const corruptFileB = await fileSystem.readFile(fileBPath)
      corruptFileB.fill(0, 0, 50)
      fileSystem.files.set(fileBPath, corruptFileB)

      await torrent.recheckData()

      expect(torrent.hasPiece(1)).toBe(false)
      expect(torrent.bitfield?.cardinality()).toBe(0)
    })

    it('recheckData validates boundary pieces against exported wanted spans', async () => {
      const fileAData = new Uint8Array(100)
      const skippedData = new Uint8Array(10)
      const fileCData = new Uint8Array(120)
      for (let i = 0; i < fileAData.length; i++) fileAData[i] = i & 0xff
      for (let i = 0; i < skippedData.length; i++) skippedData[i] = (i + 100) & 0xff
      for (let i = 0; i < fileCData.length; i++) fileCData[i] = (i + 110) & 0xff

      const torrentBuffer = createMultiFileTorrentWithData({
        name: 'RecheckBoundaryTorrent',
        files: [
          { path: 'partA.bin', data: fileAData },
          { path: 'skip.txt', data: skippedData },
          { path: 'partC.bin', data: fileCData },
        ],
        pieceLength: 256,
      })

      const { torrent } = await engine.addTorrent(torrentBuffer)
      if (!torrent) throw new Error('Torrent is null')

      await fileSystem.mkdir('RecheckBoundaryTorrent')

      torrent.setFilePriority(1, 1)
      torrent.setFilePriority(2, 1)
      const pieceData = new Uint8Array([...fileAData, ...skippedData, ...fileCData])
      // @ts-expect-error - accessing private member for testing
      await torrent.verifyAndWriteBoundaryPiece(0, pieceData, torrent.getPieceHash(0), [])
      torrent.bitfield!.set(0, true)

      torrent.setFilePriority(2, 0)
      await new Promise((resolve) => setTimeout(resolve, 50))

      const fileCPath = 'RecheckBoundaryTorrent/partC.bin'
      const corruptFileC = await fileSystem.readFile(fileCPath)
      corruptFileC.fill(0, 0, corruptFileC.length)
      fileSystem.files.set(fileCPath, corruptFileC)

      await torrent.recheckData()

      expect(torrent.partsFilePieces.has(0)).toBe(false)
      expect(torrent.hasPiece(0)).toBe(false)
      expect(torrent.bitfield?.cardinality()).toBe(0)
    })
  })
})
