var legacyMigrationNotificationCreating = false
var legacyMigrationWindowCreating = false

function readLegacyMigrationState(callback) {
    chrome.storage.local.get(LEGACY_MIGRATION_STATE_KEY, function(data) {
        callback(data[LEGACY_MIGRATION_STATE_KEY])
    })
}

function writeLegacyMigrationState(state, callback) {
    var update = {}
    update[LEGACY_MIGRATION_STATE_KEY] = state
    chrome.storage.local.set(update, callback || function() {})
}

function activateLegacyMigrationCampaign(callback) {
    readLegacyMigrationState(function(state) {
        writeLegacyMigrationState(
            activateLegacyMigrationState(state, Date.now()),
            callback
        )
    })
}

function updateLegacyMigrationCampaign(mutator, callback) {
    readLegacyMigrationState(function(state) {
        var now = Date.now()
        var current = activateLegacyMigrationState(state, now)
        writeLegacyMigrationState(mutator(current, now), callback)
    })
}

function getCurrentLegacyMigrationContext(callback) {
    chrome.runtime.getPlatformInfo(function(info) {
        callback({
            platform: getLegacyMigrationPlatform(info && info.os),
            variant: getLegacyMigrationVariant(chrome.runtime.id)
        })
    })
}

function setLegacyMigrationUninstallUrl() {
    if (!chrome.runtime.setUninstallURL) return
    var variant = getLegacyMigrationVariant(chrome.runtime.id)
    chrome.runtime.setUninstallURL(
        getLegacyMigrationUrl('legacy-app-uninstall', variant),
        function() {
            if (chrome.runtime.lastError) {
                console.warn('Unable to set migration uninstall URL', chrome.runtime.lastError.message)
            }
        }
    )
}

function openLegacyMigrationUrl(url) {
    if (chrome.browser && typeof chrome.browser.openTab === 'function') {
        try {
            chrome.browser.openTab({url:url})
            return
        } catch (error) {
            console.warn('chrome.browser.openTab failed; using window.open', error)
        }
    }
    window.open(url)
}

function openLegacyMigrationPage(ref) {
    getCurrentLegacyMigrationContext(function(context) {
        openLegacyMigrationUrl(
            getLegacyMigrationUrl(ref, context.variant, context.platform)
        )
    })
}

function acknowledgeLegacyMigrationCampaign(callback) {
    updateLegacyMigrationCampaign(function(state, now) {
        return acknowledgeLegacyMigrationState(state, now)
    }, function() {
        chrome.notifications.clear(LEGACY_MIGRATION_NOTIFICATION_ID)
        if (callback) callback()
    })
}

function acknowledgeAndOpenLegacyMigrationPage(ref) {
    openLegacyMigrationPage(ref)
    acknowledgeLegacyMigrationCampaign()
}

function snoozeLegacyMigrationCampaign(callback) {
    updateLegacyMigrationCampaign(function(state, now) {
        return snoozeLegacyMigrationState(state, now)
    }, function() {
        chrome.notifications.clear(LEGACY_MIGRATION_NOTIFICATION_ID)
        if (callback) callback()
    })
}

function stopLegacyMigrationReminders(callback) {
    updateLegacyMigrationCampaign(function(state, now) {
        return disableLegacyMigrationState(state, now)
    }, function() {
        chrome.notifications.clear(LEGACY_MIGRATION_NOTIFICATION_ID)
        var migrateWindow = chrome.app.window.get('legacy-migrate')
        if (migrateWindow) migrateWindow.close()
        if (callback) callback()
    })
}

function showLegacyMigrationWindow() {
    if (legacyMigrationWindowCreating) return
    var existing = chrome.app.window.get('legacy-migrate')
    if (existing) {
        existing.show()
        existing.focus()
        return
    }
    legacyMigrationWindowCreating = true
    chrome.app.window.create('migrate.html', {
        id: 'legacy-migrate',
        outerBounds: {width:460, height:620}
    }, function() {
        legacyMigrationWindowCreating = false
    })
}

function maybeShowLegacyMigrationNotification() {
    if (legacyMigrationNotificationCreating) return
    legacyMigrationNotificationCreating = true
    readLegacyMigrationState(function(state) {
        var now = Date.now()
        if (!isLegacyMigrationReminderDue(state, now)) {
            legacyMigrationNotificationCreating = false
            return
        }
        getCurrentLegacyMigrationContext(function(context) {
            var promptedState = markLegacyMigrationPrompted(state, now)
            var copy = getLegacyMigrationNotificationCopy(
                context.platform,
                context.variant
            )
            chrome.notifications.create(LEGACY_MIGRATION_NOTIFICATION_ID, {
                type: 'basic',
                title: copy.title,
                message: copy.message,
                iconUrl: 'js-128.png',
                priority: 2,
                requireInteraction: true,
                buttons: [
                    {title: 'See migration options'},
                    {title: 'Remind me in 7 days'}
                ]
            }, function() {
                if (chrome.runtime.lastError) {
                    legacyMigrationNotificationCreating = false
                    console.warn('Unable to create migration notification', chrome.runtime.lastError.message)
                    return
                }
                writeLegacyMigrationState(promptedState, function() {
                    legacyMigrationNotificationCreating = false
                })
            })
        })
    })
}

chrome.runtime.onInstalled.addListener(function(details) {
    setLegacyMigrationUninstallUrl()
    if (details.reason === 'install' || details.reason === 'update') {
        activateLegacyMigrationCampaign()
    }
})

chrome.runtime.onStartup.addListener(function() {
    maybeShowLegacyMigrationNotification()
})

chrome.app.runtime.onLaunched.addListener(function() {
    showLegacyMigrationWindow()
})

chrome.notifications.onClicked.addListener(function(id) {
    if (id !== LEGACY_MIGRATION_NOTIFICATION_ID) return
    acknowledgeAndOpenLegacyMigrationPage('legacy-app-notification')
})

chrome.notifications.onButtonClicked.addListener(function(id, buttonIndex) {
    if (id !== LEGACY_MIGRATION_NOTIFICATION_ID) return
    if (buttonIndex === 0) {
        acknowledgeAndOpenLegacyMigrationPage('legacy-app-notification')
    } else if (buttonIndex === 1) {
        snoozeLegacyMigrationCampaign()
    }
})

setLegacyMigrationUninstallUrl()
