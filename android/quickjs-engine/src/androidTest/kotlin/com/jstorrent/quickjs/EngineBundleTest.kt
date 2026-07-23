package com.jstorrent.quickjs

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.jstorrent.io.file.FileManagerImpl
import com.jstorrent.quickjs.bindings.NativeBindings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import kotlin.test.assertEquals

/**
 * Instrumented tests for loading the JSTorrent engine bundle.
 *
 * These tests verify that the TypeScript engine bundle (engine.bundle.js)
 * loads correctly in QuickJS and exposes the expected API.
 */
@RunWith(AndroidJUnit4::class)
class EngineBundleTest {

    private lateinit var engine: QuickJsEngine
    private lateinit var bindings: NativeBindings
    private lateinit var bundleContent: String
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @Before
    fun setUp() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        engine = QuickJsEngine()
        bindings = NativeBindings(context, engine.jsThread, scope, FileManagerImpl(context))
        engine.postAndWait {
            bindings.registerAll(engine.context)
        }

        // Load engine bundle from the test APK assets.
        val testContext = instrumentation.context
        bundleContent = testContext.assets
            .open("engine.bundle.js")
            .bufferedReader()
            .use { it.readText() }
    }

    @After
    fun tearDown() {
        bindings.shutdown()
        engine.close()
    }

    @Test
    fun bundleLoadsWithoutError() {
        // Evaluate the bundle - should not throw
        engine.evaluate(bundleContent, "engine.bundle.js")
    }

    @Test
    fun jstorrentGlobalIsObject() {
        engine.evaluate(bundleContent, "engine.bundle.js")

        val result = engine.evaluate("typeof jstorrent")
        assertEquals("object", result, "jstorrent should be an object")
    }

    @Test
    fun jstorrentInitIsFunction() {
        engine.evaluate(bundleContent, "engine.bundle.js")

        val result = engine.evaluate("typeof jstorrent.init")
        assertEquals("function", result, "jstorrent.init should be a function")
    }

    @Test
    fun jstorrentIsInitializedIsFunction() {
        engine.evaluate(bundleContent, "engine.bundle.js")

        val result = engine.evaluate("typeof jstorrent.isInitialized")
        assertEquals("function", result, "jstorrent.isInitialized should be a function")
    }

    @Test
    fun jstorrentShutdownIsFunction() {
        engine.evaluate(bundleContent, "engine.bundle.js")

        val result = engine.evaluate("typeof jstorrent.shutdown")
        assertEquals("function", result, "jstorrent.shutdown should be a function")
    }

    @Test
    fun jstorrentGetEngineIsFunction() {
        engine.evaluate(bundleContent, "engine.bundle.js")

        val result = engine.evaluate("typeof jstorrent.getEngine")
        assertEquals("function", result, "jstorrent.getEngine should be a function")
    }

    @Test
    fun isInitializedReturnsFalseBeforeInit() {
        engine.evaluate(bundleContent, "engine.bundle.js")

        val result = engine.evaluate("jstorrent.isInitialized()")
        assertEquals(false, result, "isInitialized() should return false before init")
    }

    @Test
    fun getEngineReturnsNullBeforeInit() {
        engine.evaluate(bundleContent, "engine.bundle.js")

        val result = engine.evaluate("jstorrent.getEngine()")
        assertEquals(null, result, "getEngine() should return null before init")
    }
}
