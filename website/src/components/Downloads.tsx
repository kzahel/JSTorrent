import { useState, useEffect } from 'react'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const chromeApi = (globalThis as any).chrome as any

const EXTENSION_ID = 'dbokmlpefliilbjldladbimlcfgbolhk'
const WEBSTORE_URL = `https://chromewebstore.google.com/detail/jstorrent/${EXTENSION_ID}`
const PLAY_STORE_URL = 'https://play.google.com/store/apps/details?id=com.jstorrent.app'
const GITHUB_RELEASES_URL = 'https://api.github.com/repos/kzahel/jstorrent/releases?per_page=100'

const FALLBACK_TAURI_TAG = 'v0.1.25'

interface TauriReleaseInfo {
  tag: string
  windowsUrl: string
  macosPkgArmUrl: string
  macosPkgIntelUrl: string
  macosArmUrl: string
  macosIntelUrl: string
  linuxDebUrl: string
  linuxAppImageUrl: string
  linuxArm64DebUrl: string
  linuxArm64AppImageUrl: string
}

function makeTauriReleaseInfo(tag: string): TauriReleaseInfo {
  const version = tag.replace(/^v/, '')
  return {
    tag,
    windowsUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_x64-setup.exe`,
    macosPkgArmUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_aarch64.pkg`,
    macosPkgIntelUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_x64.pkg`,
    macosArmUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_aarch64.dmg`,
    macosIntelUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_x64.dmg`,
    linuxDebUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_amd64.deb`,
    linuxAppImageUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_amd64.AppImage`,
    linuxArm64DebUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_arm64.deb`,
    linuxArm64AppImageUrl: `https://github.com/kzahel/jstorrent/releases/download/tauri-app-${tag}/JSTorrent_${version}_aarch64.AppImage`,
  }
}

interface GitHubRelease {
  tag_name: string
  prerelease: boolean
  assets: Array<{ name: string; browser_download_url: string }>
}

function parseVersion(tag: string): number[] {
  return tag
    .replace(/^v/, '')
    .split('.')
    .map((n) => parseInt(n, 10) || 0)
}

function compareVersions(a: string, b: string): number {
  const pa = parseVersion(a)
  const pb = parseVersion(b)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const diff = (pa[i] ?? 0) - (pb[i] ?? 0)
    if (diff !== 0) return diff
  }
  return 0
}

function useGitHubReleases(fallbackTag: string): TauriReleaseInfo {
  const [tauri, setTauri] = useState<TauriReleaseInfo>(() => makeTauriReleaseInfo(fallbackTag))

  useEffect(() => {
    let cancelled = false
    fetch(GITHUB_RELEASES_URL)
      .then((res) => res.json())
      .then((releases: GitHubRelease[]) => {
        if (cancelled) return

        const latestTauri = releases
          .filter((r) => {
            if (!r.tag_name.startsWith('tauri-app-v') || r.prerelease) return false
            const a = r.assets
            return (
              a.some((x) => x.name.endsWith('-setup.exe')) &&
              a.some((x) => x.name.includes('aarch64') && x.name.endsWith('.dmg')) &&
              a.some((x) => x.name.endsWith('.deb')) &&
              a.some((x) => x.name.endsWith('.AppImage'))
            )
          })
          .sort((a, b) =>
            compareVersions(
              b.tag_name.replace('tauri-app-', ''),
              a.tag_name.replace('tauri-app-', ''),
            ),
          )[0]
        if (latestTauri) {
          const tag = latestTauri.tag_name.replace('tauri-app-', '')
          const windowsExe = latestTauri.assets.find((a) => a.name.endsWith('-setup.exe'))!
          const macosPkgArm = latestTauri.assets.find(
            (a) => a.name.includes('aarch64') && a.name.endsWith('.pkg'),
          )
          const macosPkgIntel = latestTauri.assets.find(
            (a) => a.name.includes('x64') && a.name.endsWith('.pkg'),
          )
          const macosArm = latestTauri.assets.find(
            (a) => a.name.includes('aarch64') && a.name.endsWith('.dmg'),
          )!
          const macosIntel = latestTauri.assets.find(
            (a) => a.name.includes('x64') && a.name.endsWith('.dmg'),
          )!
          const linuxDeb = latestTauri.assets.find(
            (a) => a.name.endsWith('.deb') && a.name.includes('amd64'),
          )!
          const linuxAppImage = latestTauri.assets.find(
            (a) => a.name.endsWith('.AppImage') && a.name.includes('amd64'),
          )!
          const linuxArm64Deb = latestTauri.assets.find(
            (a) => a.name.endsWith('.deb') && a.name.includes('arm64'),
          )
          const linuxArm64AppImage = latestTauri.assets.find(
            (a) => a.name.endsWith('.AppImage') && a.name.includes('aarch64'),
          )
          const info = makeTauriReleaseInfo(tag)
          setTauri({
            tag,
            windowsUrl: windowsExe.browser_download_url,
            macosPkgArmUrl: macosPkgArm?.browser_download_url ?? info.macosPkgArmUrl,
            macosPkgIntelUrl: macosPkgIntel?.browser_download_url ?? info.macosPkgIntelUrl,
            macosArmUrl: macosArm.browser_download_url,
            macosIntelUrl: macosIntel.browser_download_url,
            linuxDebUrl: linuxDeb.browser_download_url,
            linuxAppImageUrl: linuxAppImage.browser_download_url,
            linuxArm64DebUrl: linuxArm64Deb?.browser_download_url ?? info.linuxArm64DebUrl,
            linuxArm64AppImageUrl:
              linuxArm64AppImage?.browser_download_url ?? info.linuxArm64AppImageUrl,
          })
        }
      })
      .catch(() => {
        // Keep fallback on error
      })
    return () => {
      cancelled = true
    }
  }, [])

  return tauri
}

type Platform = 'windows' | 'mac' | 'linux'

interface StatusResponse {
  ok: true
  installed: true
  extensionVersion: string
  platform: 'desktop' | 'chromeos'
  nativeHostConnected: boolean
  nativeHostVersion?: string
  desktopVersion?: string
  hasEverConnected: boolean
  lastConnectedTime?: number
  installId: string
}

function detectPlatform(): Platform {
  const ua = navigator.userAgent.toLowerCase()
  if (ua.includes('win')) return 'windows'
  if (ua.includes('mac')) return 'mac'
  return 'linux'
}

function detectArm64(): boolean {
  const ua = navigator.userAgent.toLowerCase()
  return ua.includes('aarch64') || ua.includes('arm64')
}

interface DownloadsProps {
  tauriAppTag?: string
}

const iconStyle = { width: 18, height: 18, fill: 'currentColor', flexShrink: 0 } as const

function GooglePlayIcon() {
  return (
    <svg viewBox="0 0 24 24" style={iconStyle}>
      <path d="M3 20.5V3.5c0-.59.34-1.11.84-1.35L13.69 12 3.84 21.85C3.34 21.6 3 21.09 3 20.5zm13.81-5.38L6.05 21.34l8.49-8.49 2.27 2.27zm3.35-4.31c.34.27.59.69.59 1.19 0 .5-.25.92-.59 1.19l-2.27 1.31-2.5-2.5 2.5-2.5 2.27 1.31zM6.05 2.66l10.76 6.22-2.27 2.27L6.05 2.66z" />
    </svg>
  )
}

function ChromeIcon() {
  return (
    <svg viewBox="0 0 24 24" style={iconStyle}>
      <path d="M12 0C8.21 0 4.831 1.757 2.632 4.501l3.953 6.848A5.454 5.454 0 0 1 12 6.545h10.691A12 12 0 0 0 12 0zM1.931 5.47A11.943 11.943 0 0 0 0 12c0 6.012 4.42 10.991 10.189 11.864l3.953-6.847a5.45 5.45 0 0 1-6.865-2.29zm13.342 2.166a5.446 5.446 0 0 1 1.45 7.09l.002.001h-.002l-5.344 9.257c.206.01.413.016.621.016 6.627 0 12-5.373 12-12 0-1.54-.29-3.011-.818-4.364zM12 16.364a4.364 4.364 0 1 1 0-8.728 4.364 4.364 0 0 1 0 8.728Z" />
    </svg>
  )
}

function WindowsIcon() {
  return (
    <svg viewBox="0 0 24 24" style={iconStyle}>
      <path d="M0 3.449L9.75 2.1v9.451H0m10.949-9.602L24 0v11.4H10.949M0 12.6h9.75v9.451L0 20.699M10.949 12.6H24V24l-12.9-1.801" />
    </svg>
  )
}

function AppleIcon() {
  return (
    <svg viewBox="0 0 24 24" style={iconStyle}>
      <path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.8-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z" />
    </svg>
  )
}

function CopyIcon() {
  return (
    <svg viewBox="0 0 16 16" version="1.1" style={{ width: 16, height: 16, fill: 'currentColor' }}>
      <path d="M0 6.75C0 5.784.784 5 1.75 5h1.5a.75.75 0 0 1 0 1.5h-1.5a.25.25 0 0 0-.25.25v7.5c0 .138.112.25.25.25h7.5a.25.25 0 0 0 .25-.25v-1.5a.75.75 0 0 1 1.5 0v1.5A1.75 1.75 0 0 1 9.25 16h-7.5A1.75 1.75 0 0 1 0 14.25Z"></path>
      <path d="M5 1.75C5 .784 5.784 0 6.75 0h7.5C15.216 0 16 .784 16 1.75v7.5A1.75 1.75 0 0 1 14.25 11h-7.5A1.75 1.75 0 0 1 5 9.25Zm1.75-.25a.25.25 0 0 0-.25.25v7.5c0 .138.112.25.25.25h7.5a.25.25 0 0 0 .25-.25v-7.5a.25.25 0 0 0-.25-.25Z"></path>
    </svg>
  )
}

export default function Downloads({ tauriAppTag }: DownloadsProps) {
  const [copied, setCopied] = useState(false)
  const [extensionInstalled, setExtensionInstalled] = useState<boolean | null>(null)
  const [status, setStatus] = useState<StatusResponse | null>(null)
  const [selectedPlatform, setSelectedPlatform] = useState<Platform>(detectPlatform)
  const isArm64 = detectArm64()
  const tauri = useGitHubReleases(tauriAppTag || FALLBACK_TAURI_TAG)

  useEffect(() => {
    const checkExtension = () => {
      try {
        if (chromeApi && chromeApi.runtime) {
          chromeApi.runtime.sendMessage(
            EXTENSION_ID,
            { type: 'status' },
            (response: StatusResponse | undefined) => {
              if (chromeApi.runtime.lastError || !response) {
                setExtensionInstalled(false)
                setStatus(null)
              } else {
                setExtensionInstalled(true)
                setStatus(response)
              }
            },
          )
        } else {
          setExtensionInstalled(false)
        }
      } catch {
        setExtensionInstalled(false)
      }
    }
    checkExtension()
  }, [])

  const copyToClipboard = () => {
    const command = 'curl -fsSL https://jstorrent.com/install.sh | bash'
    navigator.clipboard.writeText(command).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    })
  }

  const handleLaunch = async () => {
    try {
      if (chromeApi && chromeApi.runtime) {
        chromeApi.runtime.sendMessage(
          EXTENSION_ID,
          { type: 'launch-ping' },
          (response: unknown) => {
            console.log('Extension response:', response)
          },
        )
      }
    } catch (e) {
      console.error('Failed to message extension:', e)
    }
  }

  return (
    <section id="download" className="section section-alt">
      <div className="container">
        <h2>Download</h2>

        {/* Desktop App */}
        <div className="download-subsection">
          <h3>Desktop App</h3>
          <p>
            Prefer a standalone app, or don&apos;t use a Chromium-based browser? Built with Rust —
            not Electron, no bloat. No browser required.
          </p>
          {status?.desktopVersion && (
            <div
              className={`status-row ${compareVersions(status.desktopVersion, tauri.tag) >= 0 ? 'success' : ''}`}
            >
              <span
                className={`status-indicator ${compareVersions(status.desktopVersion, tauri.tag) >= 0 ? 'success' : ''}`}
              />
              <span>Installed v{status.desktopVersion}</span>
              {compareVersions(tauri.tag, status.desktopVersion) > 0 && (
                <span className="text-muted">({tauri.tag} available)</span>
              )}
            </div>
          )}
          <div className="tabs">
            <button
              className={`tab ${selectedPlatform === 'windows' ? 'active' : ''}`}
              onClick={() => setSelectedPlatform('windows')}
            >
              <WindowsIcon /> Windows
            </button>
            <button
              className={`tab ${selectedPlatform === 'mac' ? 'active' : ''}`}
              onClick={() => setSelectedPlatform('mac')}
            >
              <AppleIcon /> Mac
            </button>
            <button
              className={`tab ${selectedPlatform === 'linux' ? 'active' : ''}`}
              onClick={() => setSelectedPlatform('linux')}
            >
              Linux
            </button>
          </div>

          <div className="tab-content">
            {selectedPlatform === 'windows' && (
              <a href={tauri.windowsUrl} className="btn btn-primary">
                Download for Windows ({tauri.tag})
              </a>
            )}

            {selectedPlatform === 'mac' && (
              <div className="btn-group">
                <a href={tauri.macosPkgArmUrl} className="btn btn-primary">
                  Download for Mac — Apple Silicon ({tauri.tag})
                </a>
                <a href={tauri.macosPkgIntelUrl} className="btn btn-secondary">
                  Intel Mac
                </a>
              </div>
            )}

            {selectedPlatform === 'linux' && (
              <>
                <p>Install via terminal (auto-detects .deb / .rpm / AppImage):</p>
                <div className="command-box">
                  <code>curl -fsSL https://jstorrent.com/install.sh | bash</code>
                  <button
                    className="copy-btn"
                    onClick={copyToClipboard}
                    aria-label="Copy to clipboard"
                  >
                    <CopyIcon />
                  </button>
                  {copied && <div className="tooltip show">Copied!</div>}
                </div>
                <p style={{ marginTop: '1rem' }}>Or download directly:</p>
                <div className="btn-group">
                  <a
                    href={isArm64 ? tauri.linuxArm64DebUrl : tauri.linuxDebUrl}
                    className="btn btn-secondary"
                  >
                    .deb — {isArm64 ? 'ARM64' : 'x86_64'} ({tauri.tag})
                  </a>
                  <a
                    href={isArm64 ? tauri.linuxArm64AppImageUrl : tauri.linuxAppImageUrl}
                    className="btn btn-secondary"
                  >
                    AppImage — {isArm64 ? 'ARM64' : 'x86_64'}
                  </a>
                  <a
                    href={isArm64 ? tauri.linuxDebUrl : tauri.linuxArm64DebUrl}
                    className="btn btn-secondary"
                  >
                    .deb — {isArm64 ? 'x86_64' : 'ARM64'}
                  </a>
                  <a
                    href={isArm64 ? tauri.linuxAppImageUrl : tauri.linuxArm64AppImageUrl}
                    className="btn btn-secondary"
                  >
                    AppImage — {isArm64 ? 'x86_64' : 'ARM64'}
                  </a>
                </div>
              </>
            )}
          </div>
        </div>

        {/* Android & ChromeOS */}
        <div id="download-android" className="download-subsection">
          <h3>Android & ChromeOS</h3>
          <p>
            Native Android app powered by a QuickJS engine. Also works as a companion app on
            ChromeOS.
          </p>
          <a
            href={PLAY_STORE_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="btn btn-primary"
          >
            <GooglePlayIcon /> Get it on Google Play
          </a>
        </div>

        {/* Extension */}
        <div className="download-subsection">
          <h3>Chrome Extension</h3>
          {extensionInstalled === true ? (
            <>
              <div className="status-row success">
                <span className="status-indicator success" />
                <span>Installed</span>
                {status && <span className="text-muted">v{status.extensionVersion}</span>}
              </div>
              <button className="btn btn-primary btn-large" onClick={handleLaunch}>
                Launch JSTorrent
              </button>
            </>
          ) : extensionInstalled === false ? (
            <>
              <p>
                The easiest way to use JSTorrent on desktop. Runs right in Chrome with magnet link
                handling, right-click downloads, and the full JSTorrent UI.
              </p>
              <a
                href={WEBSTORE_URL}
                target="_blank"
                rel="noopener noreferrer"
                className="btn btn-primary"
              >
                <ChromeIcon /> Install from Chrome Web Store
              </a>
            </>
          ) : (
            <p className="text-muted">Checking...</p>
          )}
        </div>
      </div>
    </section>
  )
}
