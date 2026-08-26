var statusEl = document.getElementById('status')
var headingEl = document.getElementById('heading')
var migrationCopyEl = document.getElementById('migration-copy')
var migrateBtn = document.getElementById('migrate-btn')
var variant = getLegacyMigrationVariant(chrome.runtime.id)
var productName = getLegacyMigrationProductName(variant)
var platform = 'other'

headingEl.textContent = productName + ' has a new version'

function withBackgroundPage(callback) {
  chrome.runtime.getBackgroundPage(function(backgroundPage) {
    if (backgroundPage) callback(backgroundPage)
  })
}

chrome.runtime.getPlatformInfo(function(info) {
  platform = getLegacyMigrationPlatform(info && info.os)
  migrateBtn.href = getLegacyMigrationUrl('legacy-app-window', variant, platform)

  if (platform === 'chromeos') {
    migrationCopyEl.textContent = 'This old Chrome App no longer launches on current ChromeOS. JSTorrent is available again. Choose the Android, extension plus companion, or Crostini setup that fits this Chromebook.'
  } else if (platform === 'windows' || platform === 'macos' || platform === 'linux') {
    migrationCopyEl.textContent = 'Chrome no longer runs this old app. Install the current standalone JSTorrent desktop app; the Chrome extension is optional browser integration.'
  }
})

chrome.runtime.sendMessage(LEGACY_MIGRATION_NEW_EXTENSION_ID, {type:'ping'}, function(response) {
  if (!chrome.runtime.lastError && response && response.installed) {
    statusEl.textContent = 'JSTorrent extension detected. Finish the required desktop, Android, or Crostini setup before relying on it.'
    statusEl.className = 'status installed'
  } else {
    statusEl.textContent = 'JSTorrent extension not detected. It is optional on desktop; use the setup guide to choose what this device needs.'
    statusEl.className = 'status not-installed'
  }
})

migrateBtn.addEventListener('click', function() {
  withBackgroundPage(function(backgroundPage) {
    backgroundPage.acknowledgeLegacyMigrationCampaign()
  })
})

document.getElementById('remind-btn').addEventListener('click', function() {
  withBackgroundPage(function(backgroundPage) {
    backgroundPage.snoozeLegacyMigrationCampaign(function() {
      window.close()
    })
  })
})

document.getElementById('stop-btn').addEventListener('click', function() {
  withBackgroundPage(function(backgroundPage) {
    backgroundPage.stopLegacyMigrationReminders()
  })
})

document.getElementById('uninstall-btn').addEventListener('click', function() {
  chrome.management.uninstallSelf({showConfirmDialog:true}, function() {
    if (chrome.runtime.lastError) {
      statusEl.textContent = 'Chrome could not remove this app: ' + chrome.runtime.lastError.message
      statusEl.className = 'status not-installed'
    }
  })
})
