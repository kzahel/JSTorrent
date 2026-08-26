import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const legacyRoot = path.join(repoRoot, 'archive', 'legacy-app')

async function readLegacy(relativePath) {
  return readFile(path.join(legacyRoot, relativePath), 'utf8')
}

function plain(value) {
  return JSON.parse(JSON.stringify(value))
}

async function loadHelpers() {
  const context = { encodeURIComponent }
  vm.createContext(context)
  vm.runInContext(await readLegacy('migration.js'), context)
  return context
}

function createEvent() {
  const listeners = []
  return {
    addListener(listener) {
      listeners.push(listener)
    },
    emit(...args) {
      for (const listener of listeners) listener(...args)
    },
  }
}

async function loadRuntime({
  os = 'cros',
  runtimeId,
  initialStorage = {},
  notificationCreateError = '',
  withBrowser = true,
} = {}) {
  let now = Date.UTC(2026, 7, 26, 8)
  class FakeDate extends Date {
    static now() {
      return now
    }
  }

  const storage = structuredClone(initialStorage)
  const notifications = []
  const clearedNotifications = []
  const openedTabs = []
  const openedWindows = []
  const uninstallUrls = []
  const warnings = []
  const appWindows = new Map()
  const onInstalled = createEvent()
  const onStartup = createEvent()
  const onLaunched = createEvent()
  const onClicked = createEvent()
  const onButtonClicked = createEvent()

  const chrome = {
    runtime: {
      id: runtimeId || 'anhdpjpojoipgpmfanmedjghaligalgb',
      lastError: null,
      getPlatformInfo(callback) {
        callback({ os })
      },
      setUninstallURL(url, callback) {
        uninstallUrls.push(url)
        if (callback) callback()
      },
      onInstalled,
      onStartup,
    },
    storage: {
      local: {
        get(key, callback) {
          if (typeof key === 'string') callback({ [key]: structuredClone(storage[key]) })
          else callback(structuredClone(storage))
        },
        set(update, callback) {
          Object.assign(storage, structuredClone(update))
          if (callback) callback()
        },
      },
    },
    notifications: {
      create(id, options, callback) {
        notifications.push({ id, options: structuredClone(options) })
        if (notificationCreateError) chrome.runtime.lastError = { message: notificationCreateError }
        if (callback) callback(id)
        chrome.runtime.lastError = null
      },
      clear(id, callback) {
        clearedNotifications.push(id)
        if (callback) callback(true)
      },
      onClicked,
      onButtonClicked,
    },
    app: {
      runtime: { onLaunched },
      window: {
        get(id) {
          return appWindows.get(id) || null
        },
        create(page, options, callback) {
          const appWindow = {
            close() {},
            focus() {},
            show() {},
          }
          appWindows.set(options.id, appWindow)
          openedWindows.push({ page, options: structuredClone(options) })
          if (callback) callback(appWindow)
        },
      },
    },
  }

  if (withBrowser) {
    chrome.browser = {
      openTab({ url }) {
        openedTabs.push(url)
      },
    }
  }

  const context = {
    Date: FakeDate,
    chrome,
    console: {
      error: console.error,
      log() {},
      warn(...args) {
        warnings.push(args)
      },
    },
    encodeURIComponent,
    window: {
      open(url) {
        openedTabs.push(url)
      },
    },
  }
  vm.createContext(context)
  vm.runInContext(await readLegacy('migration.js'), context)
  vm.runInContext(await readLegacy('migration-runtime.js'), context)

  return {
    advance(milliseconds) {
      now += milliseconds
    },
    chrome,
    clearedNotifications,
    context,
    notifications,
    openedTabs,
    openedWindows,
    storage,
    uninstallUrls,
    warnings,
  }
}

test('migration helpers distinguish variants and supported platforms', async () => {
  const helpers = await loadHelpers()
  assert.equal(helpers.getLegacyMigrationVariant('anhdpjpojoipgpmfanmedjghaligalgb'), 'paid')
  assert.equal(helpers.getLegacyMigrationVariant('abmohcnlldaiaodkpacnldcdnjjgldfh'), 'lite')
  assert.equal(helpers.getLegacyMigrationPlatform('cros'), 'chromeos')
  assert.equal(helpers.getLegacyMigrationPlatform('linux'), 'linux')
  assert.equal(helpers.getLegacyMigrationPlatform('win'), 'windows')
  assert.equal(helpers.getLegacyMigrationPlatform('mac'), 'macos')
})

test('migration URL is stable, bounded, and architecture-neutral', async () => {
  const helpers = await loadHelpers()
  assert.equal(
    helpers.getLegacyMigrationUrl('legacy-app-notification', 'lite', 'chromeos'),
    'https://jstorrent.com/migrate?ref=legacy-app-notification&variant=lite&campaign=available-2026&platform=chromeos',
  )
  const copy = helpers.getLegacyMigrationNotificationCopy('chromeos', 'paid')
  assert.match(copy.message, /jstorrent\.com\/migrate/)
  assert.doesNotMatch(copy.message, /waitlist|same features|extension/i)
})

test('campaign state enforces seven days and preserves same-campaign acknowledgment', async () => {
  const helpers = await loadHelpers()
  const now = Date.UTC(2026, 7, 26)
  const day = 24 * 60 * 60 * 1000
  const initial = helpers.activateLegacyMigrationState({ migrationNoticeDismissed: true }, now)
  assert.equal(initial.campaignId, 'available-2026')
  assert.equal(helpers.isLegacyMigrationReminderDue(initial, now), true)

  const prompted = helpers.markLegacyMigrationPrompted(initial, now)
  assert.equal(helpers.isLegacyMigrationReminderDue(prompted, now + 6 * day), false)
  assert.equal(helpers.isLegacyMigrationReminderDue(prompted, now + 7 * day), true)

  const acknowledged = helpers.acknowledgeLegacyMigrationState(prompted, now + day)
  const sameCampaignUpdate = helpers.activateLegacyMigrationState(acknowledged, now + 8 * day)
  assert.deepEqual(plain(sameCampaignUpdate), plain(acknowledged))
  assert.equal(helpers.isLegacyMigrationReminderDue(sameCampaignUpdate, now + 30 * day), false)
})

test('install/update arms campaign silently and first startup prompts once', async () => {
  const runtime = await loadRuntime({
    initialStorage: { migrationNoticeDismissed: true },
  })
  runtime.chrome.runtime.onInstalled.emit({ reason: 'update', previousVersion: '2.4.4' })
  assert.equal(runtime.notifications.length, 0)
  assert.equal(runtime.storage.legacyMigrationCampaignState.campaignId, 'available-2026')

  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.notifications.length, 1)
  assert.equal(runtime.notifications[0].options.requireInteraction, true)
  assert.deepEqual(
    runtime.notifications[0].options.buttons.map((button) => button.title),
    ['See migration options', 'Remind me in 7 days'],
  )

  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.notifications.length, 1)
  assert.match(runtime.uninstallUrls.at(-1), /ref=legacy-app-uninstall/)
})

test('startup prompts again only after seven days', async () => {
  const runtime = await loadRuntime()
  runtime.chrome.runtime.onInstalled.emit({ reason: 'install' })
  runtime.chrome.runtime.onStartup.emit()
  runtime.advance(6 * 24 * 60 * 60 * 1000)
  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.notifications.length, 1)
  runtime.advance(24 * 60 * 60 * 1000)
  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.notifications.length, 2)
})

test('failed notification creation does not consume the startup reminder', async () => {
  const runtime = await loadRuntime({ notificationCreateError: 'notifications unavailable' })
  runtime.chrome.runtime.onInstalled.emit({ reason: 'update' })
  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.storage.legacyMigrationCampaignState.lastPromptedAt, 0)

  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.notifications.length, 2)
  assert.equal(runtime.warnings.length, 2)
})

test('remind action snoozes and primary action acknowledges and opens page', async () => {
  const runtime = await loadRuntime({ os: 'win', withBrowser: false })
  runtime.chrome.runtime.onInstalled.emit({ reason: 'update' })
  runtime.chrome.runtime.onStartup.emit()
  runtime.chrome.notifications.onButtonClicked.emit('legacy-migration-available-2026', 1)
  assert.ok(runtime.storage.legacyMigrationCampaignState.snoozedUntil > 0)
  assert.equal(runtime.clearedNotifications.at(-1), 'legacy-migration-available-2026')

  runtime.advance(8 * 24 * 60 * 60 * 1000)
  runtime.chrome.runtime.onStartup.emit()
  runtime.chrome.notifications.onButtonClicked.emit('legacy-migration-available-2026', 0)
  assert.ok(runtime.storage.legacyMigrationCampaignState.acknowledgedAt > 0)
  assert.match(runtime.openedTabs.at(-1), /platform=windows/)

  runtime.advance(30 * 24 * 60 * 60 * 1000)
  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.notifications.length, 2)
})

test('explicit launch creates the richer window without an automatic startup window', async () => {
  const runtime = await loadRuntime()
  runtime.chrome.runtime.onInstalled.emit({ reason: 'update' })
  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.openedWindows.length, 0)
  runtime.chrome.app.runtime.onLaunched.emit({})
  assert.equal(runtime.openedWindows.length, 1)
  assert.equal(runtime.openedWindows[0].page, 'migrate.html')
})

test('permanent stop survives startup and a later same-campaign update', async () => {
  const runtime = await loadRuntime()
  runtime.chrome.runtime.onInstalled.emit({ reason: 'update' })
  runtime.chrome.runtime.onStartup.emit()
  runtime.context.stopLegacyMigrationReminders()
  assert.ok(runtime.storage.legacyMigrationCampaignState.disabledAt > 0)

  runtime.advance(30 * 24 * 60 * 60 * 1000)
  runtime.chrome.runtime.onInstalled.emit({ reason: 'update' })
  runtime.chrome.runtime.onStartup.emit()
  assert.equal(runtime.notifications.length, 1)
})

test('package source loads migration before the legacy background and omits maximum nags', async () => {
  const manifest = JSON.parse(await readLegacy('manifest.json'))
  assert.deepEqual(manifest.app.background.scripts, [
    'conf.js',
    'migration.js',
    'migration-runtime.js',
    'background.js',
  ])

  const runtime = await readLegacy('migration-runtime.js')
  const background = await readLegacy('background.js')
  const prompt = await readLegacy('migrate.html')
  assert.doesNotMatch(
    runtime,
    /chrome\.alarms|scriptLoad|MIGRATE_ON_SCRIPT_LOAD|MIGRATE_ALARM_MINUTES/,
  )
  assert.doesNotMatch(background, /new\.jstorrent\.com|showMigrationNags|MIGRATE_ON_SCRIPT_LOAD/)
  assert.doesNotMatch(background, /doShowUpdateNotification\(details, resp\)/)
  assert.doesNotMatch(prompt, /same features and more|you're all set|waitlist/i)
  assert.match(prompt, /Stop reminders/)
  assert.match(prompt, /Remove old app/)
})
