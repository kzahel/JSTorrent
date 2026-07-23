/**
 * @jstorrent/client/core
 *
 * Chrome-free exports for standalone usage.
 * Import from '@jstorrent/client/core' to avoid Chrome type dependencies.
 */

// Adapters
export { DirectEngineAdapter } from './adapters/types'
export type { EngineAdapter } from './adapters/types'

// Types (Chrome-free)
export type { DaemonInfo, DownloadRoot } from './types'
export type {
  SearchPluginCategory,
  SearchPluginManifest,
  SearchPluginSearchInput,
  SearchPluginFetchInput,
  SearchPluginFetchPolicy,
  SearchPluginFetchResponse,
  SearchResult,
  SearchRunSummary,
  SearchDisplayResult,
  SearchPluginLogLevel,
  SearchPluginLogEntry,
  SearchPluginRequestTrace,
  SearchPluginRunError,
  SearchPluginRunTrace,
  SearchPluginDraftRunResult,
  SearchPluginSourceInspection,
  SearchPluginHtmlNode,
  SearchPluginHtmlDocument,
  SearchPluginContext,
  SearchPluginModule,
  InstalledPluginRecord,
  SearchPluginHost,
} from './search/types'

// Engine Manager types (implementations are platform-specific)
export type { IEngineManager, StorageRoot, FileOperationResult } from './engine-manager/types'

// React contexts
export { EngineProvider, useAdapter, useEngine } from './context/EngineContext'
export type { EngineProviderProps } from './context/EngineContext'
export {
  ConfigProvider,
  useConfig,
  useConfigValue,
  useConfigSubscription,
} from './context/ConfigContext'
export {
  EngineManagerProvider,
  useEngineManager,
  useFileOperations,
} from './context/EngineManagerContext'
export type { FileOperations } from './context/EngineManagerContext'
export {
  SearchPluginServiceProvider,
  useSearchPluginService,
} from './context/SearchPluginServiceContext'
export { SearchPluginService } from './search/search-plugin-service'
export { INTERNET_ARCHIVE_SAMPLE_PLUGIN_SOURCE } from './search/samples/internet-archive-plugin-source'

// Hooks
export { useEngineState, useTorrentState } from './hooks/useEngineState'
export { useConfigInit } from './hooks/useConfigInit'

// UI Components
export { AppShell } from './components/AppShell'
export { AppHeader } from './components/AppHeader'
export { VideoPopupPage } from './components/VideoPopupPage'

// App content (Chrome-free, callbacks optional)
export { AppContent } from './AppContent'
export type { AppContentProps, FileInfo } from './AppContent'

// Settings overlay (platform-agnostic, uses EngineManagerContext)
export { SettingsOverlay } from './components/SettingsOverlay'
export { SearchPluginsOverlay } from './components/SearchPluginsOverlay'

// Video popup session helpers
export {
  createVideoPopupSessionHost,
  createRemoteByteRangeStreamingSession,
} from './utils/video-popup-session'
