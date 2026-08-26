var LEGACY_MIGRATION_CAMPAIGN_ID = 'available-2026'
var LEGACY_MIGRATION_STATE_KEY = 'legacyMigrationCampaignState'
var LEGACY_MIGRATION_NOTIFICATION_ID = 'legacy-migration-available-2026'
var LEGACY_MIGRATION_REMINDER_DAYS = 7
var LEGACY_MIGRATION_REMINDER_MS = LEGACY_MIGRATION_REMINDER_DAYS * 24 * 60 * 60 * 1000
var LEGACY_MIGRATION_URL_BASE = 'https://jstorrent.com/migrate'
var LEGACY_MIGRATION_PAID_ID = 'anhdpjpojoipgpmfanmedjghaligalgb'
var LEGACY_MIGRATION_LITE_ID = 'abmohcnlldaiaodkpacnldcdnjjgldfh'
var LEGACY_MIGRATION_NEW_EXTENSION_ID = 'dbokmlpefliilbjldladbimlcfgbolhk'

function getLegacyMigrationVariant(runtimeId) {
    return runtimeId === LEGACY_MIGRATION_LITE_ID ? 'lite' : 'paid'
}

function getLegacyMigrationProductName(variant) {
    return variant === 'lite' ? 'JSTorrent Lite' : 'JSTorrent'
}

function getLegacyMigrationPlatform(os) {
    if (os === 'cros') return 'chromeos'
    if (os === 'win') return 'windows'
    if (os === 'mac') return 'macos'
    if (os === 'linux') return 'linux'
    if (os === 'android') return 'android'
    return 'other'
}

function getLegacyMigrationUrl(ref, variant, platform) {
    var values = [
        ['ref', ref || 'legacy-app'],
        ['variant', variant || 'paid'],
        ['campaign', LEGACY_MIGRATION_CAMPAIGN_ID]
    ]
    if (platform) values.push(['platform', platform])
    return LEGACY_MIGRATION_URL_BASE + '?' + values.map(function(pair) {
        return encodeURIComponent(pair[0]) + '=' + encodeURIComponent(pair[1])
    }).join('&')
}

function getLegacyMigrationNotificationCopy(platform, variant) {
    var productName = getLegacyMigrationProductName(variant)
    if (platform === 'chromeos') {
        return {
            title: productName + ' has a new version',
            message: 'This old Chrome App no longer launches. JSTorrent is available again. See setup options: jstorrent.com/migrate'
        }
    }
    if (platform === 'windows' || platform === 'macos' || platform === 'linux') {
        return {
            title: productName + ' has a new version',
            message: 'Chrome no longer runs this old app. JSTorrent is available for desktop. See migration options: jstorrent.com/migrate'
        }
    }
    return {
        title: productName + ' has a new version',
        message: 'This old Chrome App has ended. See current JSTorrent setup options: jstorrent.com/migrate'
    }
}

function createLegacyMigrationState(now) {
    return {
        campaignId: LEGACY_MIGRATION_CAMPAIGN_ID,
        activatedAt: now,
        lastPromptedAt: 0,
        snoozedUntil: 0,
        acknowledgedAt: 0,
        disabledAt: 0
    }
}

function copyLegacyMigrationState(state) {
    return {
        campaignId: state.campaignId,
        activatedAt: state.activatedAt || 0,
        lastPromptedAt: state.lastPromptedAt || 0,
        snoozedUntil: state.snoozedUntil || 0,
        acknowledgedAt: state.acknowledgedAt || 0,
        disabledAt: state.disabledAt || 0
    }
}

function activateLegacyMigrationState(state, now) {
    if (!state || state.campaignId !== LEGACY_MIGRATION_CAMPAIGN_ID) {
        return createLegacyMigrationState(now)
    }
    var current = copyLegacyMigrationState(state)
    if (!current.activatedAt) current.activatedAt = now
    return current
}

function isLegacyMigrationReminderDue(state, now) {
    if (!state || state.campaignId !== LEGACY_MIGRATION_CAMPAIGN_ID) return false
    if (!state.activatedAt || state.acknowledgedAt || state.disabledAt) return false
    if (state.snoozedUntil && now < state.snoozedUntil) return false
    if (!state.lastPromptedAt) return true
    if (now < state.lastPromptedAt) return false
    return now - state.lastPromptedAt >= LEGACY_MIGRATION_REMINDER_MS
}

function markLegacyMigrationPrompted(state, now) {
    var current = copyLegacyMigrationState(state)
    current.lastPromptedAt = now
    current.snoozedUntil = 0
    return current
}

function snoozeLegacyMigrationState(state, now) {
    var current = copyLegacyMigrationState(state)
    current.snoozedUntil = now + LEGACY_MIGRATION_REMINDER_MS
    return current
}

function acknowledgeLegacyMigrationState(state, now) {
    var current = copyLegacyMigrationState(state)
    current.acknowledgedAt = now
    current.snoozedUntil = 0
    return current
}

function disableLegacyMigrationState(state, now) {
    var current = copyLegacyMigrationState(state)
    current.disabledAt = now
    current.snoozedUntil = 0
    return current
}
