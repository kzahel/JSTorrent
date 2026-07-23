/**
 * TauriChannel — HostChannel implementation for the Tauri desktop app (context 4).
 *
 * Communicates with the native host via the Tauri Rust backend, which relays
 * messages over stdin/stdout using the native messaging protocol.
 *
 * KV storage is routed through the native host to SQLite on disk,
 * shared with the Chrome extension when installed.
 *
 * Accesses the Tauri runtime directly via window.__TAURI_INTERNALS__ to avoid
 * an npm dependency on @tauri-apps/api in the shared client package.
 */

import type { HostChannel } from './host-channel'
import type {
  HostState,
  HostCapabilities,
  KVOpts,
  HostNotification,
  NativeEvent,
  Unsubscribe,
  DaemonStats,
  DaemonInfo,
  DownloadRoot,
  UpdateCheckResult,
  ProfileListEntry,
  UsageMetrics,
  VideoPopupLaunchOptions,
} from './types'

// --- Tauri IPC helpers ---

interface TauriInternals {
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>
  transformCallback: (callback: (...args: unknown[]) => void, once?: boolean) => number
}

interface NetworkInterfaceInfo {
  name: string
  address: string
  prefixLength: number
}

interface GatewayInfo {
  ip: string
  interfaceName?: string
}

interface DaemonCapabilityPayload {
  roots_manageable?: boolean
  lan_share_urls?: boolean
  free_space?: boolean
  write_atomic?: boolean
}

function createOpaqueStreamToken(): string {
  const bytes = new Uint8Array(24)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
}

function isPrivateLanAddress(address: string): boolean {
  if (address.startsWith('10.')) return true
  if (address.startsWith('192.168.')) return true
  const match = /^172\.(\d{1,3})\./.exec(address)
  if (!match) return false
  const octet = Number(match[1])
  return octet >= 16 && octet <= 31
}

function isShareableIpv4Address(address: string): boolean {
  if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(address)) return false
  if (address.startsWith('127.')) return false
  if (address.startsWith('169.254.')) return false
  if (address === '0.0.0.0') return false
  return true
}

function pickLanAddress(
  interfaces: NetworkInterfaceInfo[],
  gateway: GatewayInfo | null,
): string | null {
  const candidates = interfaces.filter((iface) => isShareableIpv4Address(iface.address))

  if (gateway?.interfaceName) {
    const gatewayCandidates = candidates.filter((iface) => iface.name === gateway.interfaceName)
    const privateGateway = gatewayCandidates.find((iface) => isPrivateLanAddress(iface.address))
    if (privateGateway) return privateGateway.address
    if (gatewayCandidates[0]) return gatewayCandidates[0].address
  }

  const preferred = candidates.find((iface) => isPrivateLanAddress(iface.address))
  return preferred?.address ?? candidates[0]?.address ?? null
}

function getTauriInternals(): TauriInternals {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const internals = (window as any).__TAURI_INTERNALS__ as TauriInternals | undefined
  if (!internals) throw new Error('Not running in Tauri context')
  return internals
}

function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return getTauriInternals().invoke<T>(cmd, args)
}

/** Fire-and-forget log to Rust stderr via js_log command. */
export function jsLog(msg: string): void {
  tauriInvoke('js_log', { msg }).catch(() => {})
}

/**
 * Listen for Tauri events using the internal IPC mechanism.
 * Equivalent to `listen()` from @tauri-apps/api/event.
 */
function tauriListen<T>(
  event: string,
  handler: (event: { payload: T }) => void,
): Promise<() => void> {
  const internals = getTauriInternals()
  const callbackId = internals.transformCallback((rawEvent: unknown) => {
    handler(rawEvent as { payload: T })
  })
  return internals
    .invoke<number>('plugin:event|listen', {
      event,
      target: { kind: 'Any' },
      handler: callbackId,
    })
    .then((eventId) => {
      return () => {
        internals.invoke('plugin:event|unlisten', { event, eventId }).catch(() => {})
      }
    })
}

/**
 * Emit a Tauri event. Equivalent to `emit()` from @tauri-apps/api/event.
 */
function tauriEmit(event: string, payload?: unknown): Promise<void> {
  return tauriInvoke('plugin:event|emit', { event, payload })
}

// --- Host message helper ---

interface HostResponse {
  ok: boolean
  type?: string
  payload?: Record<string, unknown>
  error?: string
}

async function hostMessage(message: Record<string, unknown>): Promise<HostResponse> {
  return tauriInvoke<HostResponse>('host_message', { message })
}

// --- Desktop activation marker ---

let desktopActivationMarked = false

/**
 * Mark the current desktop profile as having been used for torrents.
 * Called when the user adds a torrent via the desktop UI.
 * No-op outside of Tauri context. Only calls the Tauri command once per session.
 */
export function markDesktopActivated(): void {
  if (desktopActivationMarked) return
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return
  desktopActivationMarked = true
  tauriInvoke('mark_desktop_activated').catch(() => {
    desktopActivationMarked = false
  })
}

/** @internal Reset for testing only. */
export function _resetDesktopActivation(): void {
  desktopActivationMarked = false
}

// --- TauriChannel ---

export class TauriChannel implements HostChannel {
  private currentState: HostState = {
    status: 'connecting',
    platform: 'tauri',
    daemonInfo: null,
    roots: [],
    lastError: null,
  }
  private stateListeners = new Set<(state: HostState) => void>()
  private eventListeners = new Set<(event: NativeEvent) => void>()
  private eventUnlisten: (() => void) | null = null
  private daemonInfo: { port: number; token: string } | null = null

  // --- Power management state ---
  private keepAwakeEnabled = false
  private activeDownloadCount = 0
  private isBlocking = false

  // --- Lifecycle ---

  async connect(): Promise<void> {
    // No synchronization needed: Tauri's setup() completes before the event
    // loop starts, so app.manage(bridge) always happens before any tauriInvoke
    // can be dispatched. The webview JS loads after setup returns.
    try {
      await this.doConnect()
    } catch (e) {
      this.updateState({
        ...this.currentState,
        status: 'disconnected',
        lastError: String(e),
      })
    }
  }

  private async doConnect(): Promise<void> {
    let storedProfileId: string | null = null
    try {
      storedProfileId = localStorage.getItem('jstorrent:profileId')
    } catch {
      // localStorage may not be available
    }
    const response = await tauriInvoke<{
      ok: boolean
      payload?: {
        port: number
        token: string
        version?: string
        roots?: DownloadRoot[]
        capabilities?: DaemonCapabilityPayload
        profileId?: string
        clientType?: string
        clientVersion?: string
        browserName?: string
        pid?: number
        started?: number
      }
      error?: string
    }>('host_handshake', { profileId: storedProfileId })

    // Note: response.type ('DaemonInfo') is missing due to serde #[serde(flatten)]
    // dropping the tag from adjacently-tagged enums. Check payload fields instead.
    if (response.ok && response.payload?.port) {
      const { port, token, version, roots, capabilities, profileId } = response.payload
      this.daemonInfo = { port, token }
      if (profileId) {
        try {
          localStorage.setItem('jstorrent:profileId', profileId)
        } catch {
          // localStorage may not be available
        }
      }
      this.updateState({
        status: 'connected',
        platform: 'tauri',
        daemonInfo: {
          port,
          token,
          version,
          roots: roots ?? [],
          host: '127.0.0.1',
          capabilities:
            capabilities == null
              ? undefined
              : {
                  roots_manageable: capabilities.roots_manageable !== false,
                  lan_share_urls: capabilities.lan_share_urls === true,
                  free_space: capabilities.free_space === true,
                  write_atomic: capabilities.write_atomic === true,
                },
          profileId,
        },
        roots: roots ?? [],
        lastError: null,
      })
    } else if (response.error === 'profile_in_use' && response.payload) {
      const { clientType, clientVersion, browserName, pid, started } = response.payload
      this.updateState({
        ...this.currentState,
        status: 'disconnected',
        lastError: 'profile_in_use',
        profileInUseInfo: { clientType, clientVersion, browserName, pid, started },
      })
    } else {
      const error = response.error ?? 'Handshake failed'
      // If the stored profileId is stale (e.g. config reset, upgrade), clear it
      // and retry once so the host creates a fresh profile.
      if (storedProfileId && error.includes('Invalid profile ID')) {
        console.warn('[TauriChannel] Stale profileId, clearing and retrying')
        try {
          localStorage.removeItem('jstorrent:profileId')
        } catch {
          // localStorage may not be available
        }
        return this.connect()
      }
      this.updateState({
        ...this.currentState,
        status: 'disconnected',
        lastError: error,
      })
    }

    // Listen for events from native host (MagnetAdded, TorrentAdded)
    this.eventUnlisten = await tauriListen<{ event?: string; payload?: unknown }>(
      'host-event',
      (event) => {
        const data = event.payload
        if (data?.event) {
          const nativeEvent: NativeEvent = { event: data.event, payload: data.payload }
          for (const cb of this.eventListeners) {
            cb(nativeEvent)
          }
        }
      },
    )

    // Retrieve any deep link events that arrived before the frontend was ready
    // (e.g., app was launched by clicking a magnet link)
    this.drainPendingDeepLinks()

    // Load keepAwake setting from persisted config
    this.kvGet<boolean>('keepAwake', { keyPrefix: 'config:' })
      .then((enabled) => {
        if (enabled) this.setKeepAwake(true)
      })
      .catch(() => {})
  }

  disconnect(): void {
    if (this.eventUnlisten) {
      this.eventUnlisten()
      this.eventUnlisten = null
    }
  }

  // --- Connection state ---

  getState(): HostState {
    return this.currentState
  }

  onStateChanged(cb: (state: HostState) => void): Unsubscribe {
    this.stateListeners.add(cb)
    return () => {
      this.stateListeners.delete(cb)
    }
  }

  // --- Events ---

  onEvent(cb: (event: NativeEvent) => void): Unsubscribe {
    this.eventListeners.add(cb)
    return () => {
      this.eventListeners.delete(cb)
    }
  }

  // --- Capabilities ---

  get capabilities(): HostCapabilities {
    return {
      rootsManageable: true,
      hasSync: false,
      hasNativeNotifications: true,
      hasBackgroundPersistence: true,
    }
  }

  // --- KV storage (routed through native host → SQLite) ---

  async kvGet<T = unknown>(key: string, opts?: KVOpts): Promise<T | undefined> {
    const prefixed = (opts?.keyPrefix ?? 'session:') + key
    const resp = await hostMessage({ op: 'kvGet', key: prefixed })
    if (resp.ok && resp.payload) {
      const value = resp.payload.value as string | null
      if (value != null) return JSON.parse(value) as T
    }
    return undefined
  }

  async kvGetMulti(keys: string[], opts?: KVOpts): Promise<Record<string, unknown>> {
    if (keys.length === 0) return {}
    const prefix = opts?.keyPrefix ?? 'session:'
    const prefixedKeys = keys.map((k) => prefix + k)
    const resp = await hostMessage({ op: 'kvGetMulti', keys: prefixedKeys })
    const result: Record<string, unknown> = {}
    if (resp.ok && resp.payload) {
      const entries = resp.payload.entries as Record<string, string> | undefined
      if (entries) {
        for (const [k, v] of Object.entries(entries)) {
          result[k.slice(prefix.length)] = JSON.parse(v)
        }
      }
    }
    return result
  }

  async kvSet(key: string, value: unknown, opts?: KVOpts): Promise<void> {
    const prefixed = (opts?.keyPrefix ?? 'session:') + key
    await hostMessage({ op: 'kvSet', key: prefixed, value: JSON.stringify(value) })
  }

  async kvDelete(key: string, opts?: KVOpts): Promise<void> {
    const prefixed = (opts?.keyPrefix ?? 'session:') + key
    await hostMessage({ op: 'kvDelete', key: prefixed })
  }

  async kvKeys(prefix?: string, opts?: KVOpts): Promise<string[]> {
    const keyPrefix = opts?.keyPrefix ?? 'session:'
    const fullPrefix = keyPrefix + (prefix ?? '')
    const resp = await hostMessage({ op: 'kvKeys', prefix: fullPrefix })
    if (resp.ok && resp.payload) {
      const keys = resp.payload.keys as string[] | undefined
      if (keys) return keys.map((k) => k.slice(keyPrefix.length))
    }
    return []
  }

  async kvClear(prefix?: string, opts?: KVOpts): Promise<void> {
    const keyPrefix = opts?.keyPrefix ?? 'session:'
    const fullPrefix = keyPrefix + (prefix ?? '')
    await hostMessage({ op: 'kvClear', prefix: fullPrefix })
  }

  // --- File operations ---

  async pickDownloadFolder(): Promise<DownloadRoot | null> {
    try {
      // Use Tauri's native dialog (properly parented to the app window)
      const startDir =
        this.currentState.roots.length > 0
          ? this.currentState.roots[this.currentState.roots.length - 1].path
          : undefined

      const response = await tauriInvoke<HostResponse>('pick_download_folder', { startDir })

      if (response.ok && response.payload?.root) {
        const root = response.payload.root as DownloadRoot
        const exists = this.currentState.roots.some((r) => r.key === root.key)
        const newRoots = exists ? this.currentState.roots : [...this.currentState.roots, root]
        this.updateState({ ...this.currentState, roots: newRoots })
        return root
      }
      return null
    } catch {
      // User cancelled or dialog error
      return null
    }
  }

  async removeDownloadRoot(key: string): Promise<void> {
    await tauriInvoke('host_message', {
      message: { op: 'deleteDownloadRoot', key },
    })
    const newRoots = this.currentState.roots.filter((r) => r.key !== key)
    this.updateState({ ...this.currentState, roots: newRoots })
  }

  async openFile(rootKey: string, path: string): Promise<void> {
    await tauriInvoke('host_message', {
      message: { op: 'openFile', rootKey, path },
    })
  }

  async revealInFolder(rootKey: string, path: string): Promise<void> {
    await tauriInvoke('host_message', {
      message: { op: 'revealInFolder', rootKey, path },
    })
  }

  async createLanShareUrl(
    torrentId: string,
    fileIndex: number,
    rootKey: string,
    path: string,
    fileSize: number,
    mimeType?: string | null,
  ): Promise<string | null> {
    const daemonInfo = this.currentState.daemonInfo
    if (!daemonInfo?.port || !daemonInfo.token) {
      return null
    }
    if (daemonInfo.capabilities?.lan_share_urls !== true) {
      return null
    }

    const daemonHost = daemonInfo.host ?? '127.0.0.1'
    const streamToken = createOpaqueStreamToken()

    const registerResponse = await fetch(
      `http://${daemonHost}:${daemonInfo.port}/stream/register`,
      {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-JST-Auth': daemonInfo.token,
        },
        body: JSON.stringify({
          streamToken,
          torrentId,
          fileIndex,
          rootKey,
          path,
          fileSize,
          mimeType: mimeType ?? null,
        }),
      },
    )
    if (!registerResponse.ok) {
      throw new Error(`Failed to register HTTP stream: ${registerResponse.status}`)
    }

    const registerData = (await registerResponse.json()) as {
      ok: boolean
      error?: string
      mediaPort?: number
    }
    if (!registerData.ok) {
      throw new Error(registerData.error ?? 'Failed to register HTTP stream')
    }

    const mediaPort =
      typeof registerData.mediaPort === 'number'
        ? registerData.mediaPort
        : Number(registerData.mediaPort)
    if (!Number.isFinite(mediaPort) || mediaPort <= 0) {
      throw new Error('Daemon did not return a media port')
    }

    const authHeaders = { 'X-JST-Auth': daemonInfo.token }
    const [interfaces, gateway] = await Promise.all([
      fetch(`http://${daemonHost}:${daemonInfo.port}/network/interfaces`, {
        headers: authHeaders,
      }).then(async (response) => {
        if (!response.ok) {
          throw new Error(`Failed to query network interfaces: ${response.status}`)
        }
        return (await response.json()) as NetworkInterfaceInfo[]
      }),
      fetch(`http://${daemonHost}:${daemonInfo.port}/network/gateway`, {
        headers: authHeaders,
      })
        .then(async (response) => {
          if (!response.ok) {
            return null
          }
          return (await response.json()) as GatewayInfo | null
        })
        .catch(() => null),
    ])

    const lanAddress = pickLanAddress(interfaces, gateway)
    if (!lanAddress) {
      throw new Error('No LAN IPv4 address available for sharing')
    }

    return `http://${lanAddress}:${mediaPort}/stream/${streamToken}`
  }

  // --- Notifications ---

  notify(notification: HostNotification): void {
    if (notification.type === 'stats') {
      tauriInvoke('update_tray_stats', { stats: notification.stats }).catch(() => {})
      this.activeDownloadCount = notification.stats.activeCount
      this.updateNoSleep()
    } else if (notification.type === 'torrent-complete') {
      this.showNotificationIfEnabled(
        'notifyOnTorrentComplete',
        'Download Complete',
        notification.name,
      )
    } else if (notification.type === 'torrent-error') {
      this.showNotificationIfEnabled(
        'notifyOnError',
        'Download Error',
        `${notification.name}: ${notification.error}`,
      )
    } else if (notification.type === 'duplicate-torrent') {
      tauriInvoke('show_notification', {
        title: 'Already Added',
        body: `"${notification.name}" is already in your torrent list`,
      }).catch(() => {})
    }
  }

  private showNotificationIfEnabled(settingKey: string, title: string, body: string): void {
    this.kvGet<boolean>(settingKey, { keyPrefix: 'config:' })
      .then((enabled) => {
        if (enabled !== false) {
          tauriInvoke('show_notification', { title, body }).catch(() => {})
        }
      })
      .catch(() => {
        // Setting not found, default to enabled
        tauriInvoke('show_notification', { title, body }).catch(() => {})
      })
  }

  // --- Host actions ---

  retryConnection(): void {
    this.connect().catch((e) => {
      console.error('[TauriChannel] Retry failed:', e)
    })
  }

  triggerLaunch(): void {
    // No-op — daemon is always launched by native host
  }

  async openVideoPlayerPopup(_options: VideoPopupLaunchOptions): Promise<boolean> {
    throw new Error('Video popup is not supported in the desktop app')
  }

  takeOver(): void {
    let storedProfileId: string | null = null
    try {
      storedProfileId = localStorage.getItem('jstorrent:profileId')
    } catch {
      // localStorage may not be available
    }
    hostMessage({
      op: 'takeOver',
      extensionId: 'tauri-desktop',
      profileId: storedProfileId,
      clientType: 'tauri',
      clientVersion: this.getVersion() ?? undefined,
    })
      .then((response) => {
        if (response.ok && response.type === 'DaemonInfo' && response.payload) {
          const port = response.payload.port as number
          const token = response.payload.token as string
          const version = response.payload.version as string | undefined
          const roots = (response.payload.roots as DownloadRoot[] | undefined) ?? []
          const capabilities = response.payload.capabilities as DaemonCapabilityPayload | undefined
          const profileId = response.payload.profileId as string | undefined
          this.daemonInfo = { port, token }
          if (profileId) {
            try {
              localStorage.setItem('jstorrent:profileId', profileId)
            } catch {
              // localStorage may not be available
            }
          }
          this.updateState({
            status: 'connected',
            platform: 'tauri',
            daemonInfo: {
              port,
              token,
              version,
              roots,
              host: '127.0.0.1',
              capabilities:
                capabilities == null
                  ? undefined
                  : {
                      roots_manageable: capabilities.roots_manageable !== false,
                      lan_share_urls: capabilities.lan_share_urls === true,
                      free_space: capabilities.free_space === true,
                      write_atomic: capabilities.write_atomic === true,
                    },
              profileId,
            },
            roots,
            lastError: null,
          })
        } else {
          this.updateState({
            ...this.currentState,
            status: 'disconnected',
            lastError: response.error ?? 'Take over failed',
          })
        }
      })
      .catch((e) => {
        console.error('[TauriChannel] Take over failed:', e)
      })
  }

  // --- Debug / admin ---

  async getStats(): Promise<DaemonStats | null> {
    if (!this.daemonInfo) return null
    const { port, token } = this.daemonInfo
    try {
      const response = await fetch(`http://127.0.0.1:${port}/stats`, {
        headers: { 'X-JST-Auth': token },
      })
      return response.ok ? ((await response.json()) as DaemonStats) : null
    } catch {
      return null
    }
  }

  async getDaemonInfo(): Promise<DaemonInfo | null> {
    return this.currentState.daemonInfo
  }

  async getMetrics(): Promise<UsageMetrics | null> {
    return null // Metrics are extension-only (chrome.storage.sync)
  }

  async clearSessionStorage(): Promise<void> {
    try {
      await this.kvClear(undefined, { keyPrefix: 'session:' })
    } catch (e) {
      console.warn('[TauriChannel] Failed to clear session storage:', e)
    }
  }

  notifyClosing(): void {
    // No-op — Tauri handles app lifecycle natively
  }

  // --- App info ---

  getVersion(): string | null {
    // Set in Tauri app's vite.config.ts via define: { 'import.meta.env.PACKAGE_VERSION': ... }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return (import.meta as any).env?.PACKAGE_VERSION ?? null
  }

  isDevMode(): boolean {
    return import.meta.env?.DEV ?? false
  }

  requestPermission(_permission: string): Promise<boolean> {
    return Promise.resolve(true) // Desktop apps have full permissions
  }

  setKeepAwake(enabled: boolean): void {
    this.keepAwakeEnabled = enabled
    this.updateNoSleep()
  }

  async checkForUpdates(): Promise<UpdateCheckResult | null> {
    tauriEmit('check-for-updates')
    return null // Tauri handles updates via its own JS-side dialog
  }

  async installUpdate(): Promise<boolean> {
    // Tauri handles install via its own dialog triggered by checkForUpdates
    return false
  }

  // --- Desktop app ---

  async launchDesktop(): Promise<boolean> {
    return false // Already in the desktop app
  }

  // --- Profile management ---

  async listProfiles(): Promise<ProfileListEntry[]> {
    try {
      const resp = await hostMessage({ op: 'listProfiles' })
      if (resp.ok && resp.type === 'ProfileList' && resp.payload) {
        return (resp.payload as { profiles?: ProfileListEntry[] }).profiles ?? []
      }
      return []
    } catch {
      return []
    }
  }

  async renameProfile(profileId: string, displayName: string): Promise<boolean> {
    try {
      const resp = await hostMessage({ op: 'renameProfile', profileId, displayName })
      return resp.ok
    } catch {
      return false
    }
  }

  async deleteProfile(profileId: string): Promise<boolean> {
    try {
      const resp = await hostMessage({ op: 'deleteProfile', profileId })
      return resp.ok
    } catch {
      return false
    }
  }

  async switchProfile(profileId: string | null): Promise<void> {
    if (profileId != null) {
      try {
        localStorage.setItem('jstorrent:profileId', profileId)
      } catch {
        // localStorage may not be available
      }
    } else {
      try {
        localStorage.removeItem('jstorrent:profileId')
      } catch {
        // localStorage may not be available
      }
    }
    await tauriInvoke('restart_app')
  }

  // --- Private helpers ---

  private drainPendingDeepLinks(): void {
    tauriInvoke<{ event: string; payload: unknown }[]>('get_pending_deep_links')
      .then((events) => {
        for (const evt of events) {
          if (evt.event) {
            const nativeEvent: NativeEvent = { event: evt.event, payload: evt.payload }
            for (const cb of this.eventListeners) {
              cb(nativeEvent)
            }
          }
        }
      })
      .catch((e) => {
        console.warn('[TauriChannel] Failed to get pending deep links:', e)
      })
  }

  private updateNoSleep(): void {
    const shouldBlock = this.keepAwakeEnabled && this.activeDownloadCount > 0
    if (shouldBlock && !this.isBlocking) {
      tauriInvoke('plugin:nosleep|block', {
        noSleepType: 'PreventUserIdleSystemSleep',
      }).catch((e) => console.warn('[TauriChannel] Failed to block sleep:', e))
      this.isBlocking = true
    } else if (!shouldBlock && this.isBlocking) {
      tauriInvoke('plugin:nosleep|unblock').catch((e) =>
        console.warn('[TauriChannel] Failed to unblock sleep:', e),
      )
      this.isBlocking = false
    }
  }

  private updateState(newState: HostState): void {
    this.currentState = newState
    for (const cb of this.stateListeners) {
      cb(newState)
    }
  }
}
