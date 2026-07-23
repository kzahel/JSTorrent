package com.jstorrent.app.ui.screens

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.runtime.mutableStateOf
import com.jstorrent.app.search.InstalledPluginRecord
import com.jstorrent.app.search.RecommendedSearchPlugin
import com.jstorrent.app.search.SearchDisplayResult
import com.jstorrent.app.search.SearchPluginManifest
import com.jstorrent.app.search.SearchResult
import com.jstorrent.app.ui.theme.JSTorrentTheme
import com.jstorrent.app.viewmodel.SearchResultItemUi
import com.jstorrent.app.viewmodel.SearchUiState
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class SearchScreenTest {

    @get:Rule
    val composeTestRule = createComposeRule()

    @Test
    fun emptyState_showsInstallAndManageActions() {
        composeTestRule.setContent {
            JSTorrentTheme {
                SearchScreenContent(
                    uiState = SearchUiState(
                        recommendedPlugins = listOf(
                            RecommendedSearchPlugin(
                                manifest = SearchPluginManifest(
                                    id = "org.archive.search",
                                    name = "Internet Archive",
                                    hosts = listOf("archive.org")
                                ),
                                sourceUrl = "https://example.com/archive.js"
                            )
                        )
                    ),
                    onNavigateBack = {},
                    onOpenTorrentDetails = {},
                    onManageSearchPlugins = {},
                    onQueryChanged = {},
                    onCategoryChanged = {},
                    onTogglePluginSelection = {},
                    onSelectAllPlugins = {},
                    onClearPluginSelection = {},
                    onSearch = {},
                    onInstallRecommended = {},
                    onAddResult = {}
                )
            }
        }

        composeTestRule.onNodeWithText("No search plugins are enabled").assertIsDisplayed()
        composeTestRule.onNodeWithText("Install Internet Archive").assertIsDisplayed()
        composeTestRule.onNodeWithText("Manage Search Plugins").assertIsDisplayed()
    }

    @Test
    fun searchForm_callbacksFire() {
        var query = ""
        var searches = 0
        val uiState = mutableStateOf(SearchUiState(enabledPlugins = emptyList()))

        composeTestRule.setContent {
            JSTorrentTheme {
                SearchScreenContent(
                    uiState = uiState.value,
                    onNavigateBack = {},
                    onOpenTorrentDetails = {},
                    onManageSearchPlugins = {},
                    onQueryChanged = {
                        query = it
                        uiState.value = uiState.value.copy(query = it)
                    },
                    onCategoryChanged = {},
                    onTogglePluginSelection = {},
                    onSelectAllPlugins = {},
                    onClearPluginSelection = {},
                    onSearch = { searches += 1 },
                    onInstallRecommended = {},
                    onAddResult = {}
                )
            }
        }

        composeTestRule.onNodeWithText("Search query").performTextInput("ubuntu")
        composeTestRule.onNodeWithContentDescription("Run search").performClick()

        assertEquals("ubuntu", query)
        assertEquals(1, searches)
    }

    @Test
    fun pluginSelector_and_openDetailsAction_areShown() {
        var toggledPluginId: String? = null
        var openedInfoHash: String? = null

        composeTestRule.setContent {
            JSTorrentTheme {
                SearchScreenContent(
                    uiState = SearchUiState(
                        enabledPlugins = listOf(
                            InstalledPluginRecord(
                                pluginId = "plugin-a",
                                manifest = SearchPluginManifest(
                                    id = "plugin-a",
                                    name = "Plugin A",
                                    hosts = listOf("example.com")
                                ),
                                sourceHash = "hash-a",
                                installedAt = 1L,
                                updatedAt = 1L,
                                enabled = true,
                                code = "plugin-code-a"
                            ),
                            InstalledPluginRecord(
                                pluginId = "plugin-b",
                                manifest = SearchPluginManifest(
                                    id = "plugin-b",
                                    name = "Plugin B",
                                    hosts = listOf("example.org")
                                ),
                                sourceHash = "hash-b",
                                installedAt = 1L,
                                updatedAt = 1L,
                                enabled = true,
                                code = "plugin-code-b"
                            )
                        ),
                        selectedPluginIds = setOf("plugin-a", "plugin-b"),
                        results = listOf(
                            SearchResultItemUi(
                                displayResult = SearchDisplayResult(
                                    pluginId = "plugin-a",
                                    pluginName = "Plugin A",
                                    allowedHosts = listOf("example.com"),
                                    result = SearchResult(
                                        name = "Tracked Torrent",
                                        source = "Plugin A",
                                        infoHash = "2222222222222222222222222222222222222222"
                                    )
                                ),
                                resolvedInfoHash = "2222222222222222222222222222222222222222",
                                isTracked = true
                            )
                        )
                    ),
                    onNavigateBack = {},
                    onOpenTorrentDetails = { openedInfoHash = it },
                    onManageSearchPlugins = {},
                    onQueryChanged = {},
                    onCategoryChanged = {},
                    onTogglePluginSelection = { toggledPluginId = it },
                    onSelectAllPlugins = {},
                    onClearPluginSelection = {},
                    onSearch = {},
                    onInstallRecommended = {},
                    onAddResult = {}
                )
            }
        }

        composeTestRule.onNodeWithText("Plugins (2/2)").assertIsDisplayed()
        composeTestRule.onNodeWithText("Plugin B").performClick()
        composeTestRule.onNodeWithText("Already in your torrent list").assertIsDisplayed()
        composeTestRule.onNodeWithText("Open details").performClick()

        assertEquals("plugin-b", toggledPluginId)
        assertEquals("2222222222222222222222222222222222222222", openedInfoHash)
    }
}
