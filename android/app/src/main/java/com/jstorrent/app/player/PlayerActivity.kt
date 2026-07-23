@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package com.jstorrent.app.player

import android.app.PictureInPictureParams
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Color
import android.os.Handler
import android.os.Build
import android.os.Bundle
import android.os.Looper
import android.util.Rational
import android.widget.Toast
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.ViewConfiguration
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Fullscreen
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.PictureInPictureAlt
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.lifecycleScope
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.VideoSize
import androidx.media3.common.C
import androidx.media3.common.MimeTypes
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import androidx.media3.ui.PlayerView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import com.jstorrent.app.JSTorrentApplication
import com.jstorrent.app.R
import com.jstorrent.app.ui.theme.JSTorrentTheme
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.abs
import kotlin.math.min

private const val YOUTUBE_STYLE_SEEK_MS = 10_000L

class PlayerActivity : ComponentActivity() {
    private val app: JSTorrentApplication
        get() = application as JSTorrentApplication

    private var screenState by mutableStateOf<PlayerScreenState>(PlayerScreenState.Preparing)
    private var player by mutableStateOf<ExoPlayer?>(null)
    private var bufferingMessage by mutableStateOf<String?>(null)
    private var playerErrorMessage by mutableStateOf<String?>(null)
    private var isFullscreen by mutableStateOf(false)
    private var isInPictureInPictureUiMode by mutableStateOf(false)
    private var playbackSessionRegistered = false
    private var subtitleLabel by mutableStateOf<String?>(null)

    private val subtitlePicker = registerForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri ->
        if (uri == null) return@registerForActivityResult
        try {
            contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        } catch (_: SecurityException) {
            // Best effort only; temporary permission from picker is enough for current session.
        }
        if (!attachExternalSubtitle(uri)) {
            Toast.makeText(this, getString(R.string.player_subtitle_load_error), Toast.LENGTH_SHORT).show()
        }
    }

    private val playerListener = object : Player.Listener {
        override fun onPlaybackStateChanged(playbackState: Int) {
            bufferingMessage = when (playbackState) {
                Player.STATE_IDLE -> getString(R.string.player_loading_video)
                Player.STATE_BUFFERING -> getString(R.string.player_buffering)
                Player.STATE_READY, Player.STATE_ENDED -> null
                else -> null
            }
        }

        override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
            playerErrorMessage = buildPlayerErrorMessage(error)
            bufferingMessage = null
        }

        override fun onVideoSizeChanged(videoSize: VideoSize) {
            updatePictureInPictureParams()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        val streamingRequest = PlayerActivityLauncher.fromIntent(intent)
        val localRequest = PlayerActivityLauncher.localFromIntent(intent)
        val launchSource = when {
            streamingRequest != null -> PlayerLaunchSource.Stream(streamingRequest)
            localRequest != null -> PlayerLaunchSource.Local(localRequest)
            else -> null
        }

        if (launchSource == null) {
            screenState = PlayerScreenState.Error(getString(R.string.player_invalid_request))
        } else {
            subtitleLabel = null
            lifecycleScope.launch {
                prepareAndStartPlayback(launchSource)
            }
            if (launchSource is PlayerLaunchSource.Stream) {
                lifecycleScope.launch {
                    monitorPlaybackTorrent(launchSource.request.infoHash)
                }
            }
        }

        setContent {
            JSTorrentTheme {
                PlayerActivityScreen(
                    state = screenState,
                    player = player,
                    bufferingMessage = bufferingMessage,
                    playerErrorMessage = playerErrorMessage,
                    isFullscreen = isFullscreen,
                    isInPictureInPicture = isInPictureInPictureUiMode,
                    hasExternalSubtitle = subtitleLabel != null,
                    onSetFullscreen = ::setFullscreenMode,
                    onEnterPictureInPicture = ::enterPictureInPictureIfPossible,
                    onLoadSubtitle = ::openSubtitlePicker,
                    onClearSubtitle = ::clearExternalSubtitle,
                    onClose = ::closePlayer
                )
            }
        }
    }

    override fun onStart() {
        super.onStart()
        app.serviceLifecycleManager.onActivityStart()
        applySystemBarsVisibility()
        updatePictureInPictureParams()
    }

    override fun onStop() {
        super.onStop()
        app.serviceLifecycleManager.onActivityStop()
    }

    override fun onDestroy() {
        releasePlayer()
        setPlaybackSessionRegistered(false)
        setFullscreenMode(false)
        super.onDestroy()
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            enterPictureInPictureIfPossible()
        }
    }

    override fun onPictureInPictureModeChanged(
        isInPictureInPictureMode: Boolean,
        newConfig: Configuration
    ) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        isInPictureInPictureUiMode = isInPictureInPictureMode
        applySystemBarsVisibility()
    }

    private suspend fun prepareAndStartPlayback(source: PlayerLaunchSource) {
        screenState = PlayerScreenState.Preparing
        playerErrorMessage = null
        bufferingMessage = getString(R.string.player_loading_video)
        if (source is PlayerLaunchSource.Stream) {
            setPlaybackSessionRegistered(true)
        }

        try {
            val exoPlayer = when (source) {
                is PlayerLaunchSource.Stream -> {
                    withContext(Dispatchers.IO) {
                        app.ensureEngineStarted()
                        TorrentPlaybackCoordinator(app.engineServiceRepository)
                            .prepareForPlayback(
                                PlaybackPreparationInput(
                                    infoHash = source.request.infoHash,
                                    fileIndex = source.request.fileIndex,
                                    filePath = source.request.filePath,
                                    isFileSelected = source.request.isFileSelected,
                                    torrentUserState = source.request.torrentUserState,
                                    torrentStatus = source.request.torrentStatus
                                )
                            )
                    }

                    withContext(Dispatchers.Main.immediate) {
                        buildStreamingPlayer(source.request)
                    }
                }
                is PlayerLaunchSource.Local -> {
                    withContext(Dispatchers.Main.immediate) {
                        buildLocalPlayer(source.request)
                    }
                }
            }

            player = exoPlayer
            screenState = PlayerScreenState.Ready(
                fileName = when (source) {
                    is PlayerLaunchSource.Stream -> source.request.fileName
                    is PlayerLaunchSource.Local -> source.request.title
                }
            )
            updatePictureInPictureParams()
        } catch (t: Throwable) {
            releasePlayer()
            if (source is PlayerLaunchSource.Stream) {
                setPlaybackSessionRegistered(false)
            }
            screenState = PlayerScreenState.Error(t.message ?: getString(R.string.player_unknown_error))
        }
    }

    private fun buildStreamingPlayer(request: PlayerLaunchRequest): ExoPlayer {
        releasePlayer()

        val dataSourceFactory = TorrentPlaybackDataSourceFactory(app, request)
        val mediaItem = MediaItem.Builder()
            .setMediaId("${request.infoHash}:${request.fileIndex}")
            .setUri(PlayerActivityLauncher.buildPlaybackUri(request))
            .build()
        val mediaSource = ProgressiveMediaSource.Factory(dataSourceFactory).createMediaSource(mediaItem)
        val loadControl = DefaultLoadControl.Builder()
            .setBufferDurationsMs(
                5_000,
                20_000,
                1_500,
                3_000
            )
            .build()

        return ExoPlayer.Builder(this)
            .setLoadControl(loadControl)
            .build()
            .also { exoPlayer ->
                exoPlayer.addListener(playerListener)
                exoPlayer.setMediaSource(mediaSource)
                exoPlayer.prepare()
                exoPlayer.playWhenReady = true
            }
    }

    private fun buildLocalPlayer(request: LocalPlaybackRequest): ExoPlayer {
        releasePlayer()

        val mediaItemBuilder = MediaItem.Builder()
            .setMediaId(request.uri.toString())
            .setUri(request.uri)

        request.mimeType?.let(mediaItemBuilder::setMimeType)

        return ExoPlayer.Builder(this)
            .build()
            .also { exoPlayer ->
                exoPlayer.addListener(playerListener)
                exoPlayer.setMediaItem(mediaItemBuilder.build())
                exoPlayer.prepare()
                exoPlayer.playWhenReady = true
            }
    }

    private fun releasePlayer() {
        val hadPlayer = player != null
        player?.removeListener(playerListener)
        player?.release()
        player = null
        subtitleLabel = null
        if (hadPlayer) {
            setPlaybackSessionRegistered(false)
        }
    }

    private fun closePlayer() {
        if (isTaskRoot) {
            startActivity(
                Intent(this, com.jstorrent.app.NativeStandaloneActivity::class.java).apply {
                    addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                }
            )
        }
        finish()
    }

    private fun openSubtitlePicker() {
        subtitlePicker.launch(arrayOf("*/*"))
    }

    private fun clearExternalSubtitle() {
        val exoPlayer = player ?: return
        val mediaItem = exoPlayer.currentMediaItem ?: return
        val currentPosition = exoPlayer.currentPosition
        val playWhenReady = exoPlayer.playWhenReady

        exoPlayer.setMediaItem(
            mediaItem.buildUpon()
                .setSubtitleConfigurations(emptyList())
                .build(),
            currentPosition
        )
        exoPlayer.prepare()
        exoPlayer.playWhenReady = playWhenReady
        subtitleLabel = null
    }

    private fun attachExternalSubtitle(uri: android.net.Uri): Boolean {
        val exoPlayer = player ?: return false
        val mediaItem = exoPlayer.currentMediaItem ?: return false
        val subtitleMimeType = inferSubtitleMimeType(uri) ?: return false
        val currentPosition = exoPlayer.currentPosition
        val playWhenReady = exoPlayer.playWhenReady

        val subtitleConfig = MediaItem.SubtitleConfiguration.Builder(uri)
            .setMimeType(subtitleMimeType)
            .setRoleFlags(C.ROLE_FLAG_SUBTITLE)
            .build()

        exoPlayer.setMediaItem(
            mediaItem.buildUpon()
                .setSubtitleConfigurations(listOf(subtitleConfig))
                .build(),
            currentPosition
        )
        exoPlayer.prepare()
        exoPlayer.playWhenReady = playWhenReady
        subtitleLabel = uri.lastPathSegment?.substringAfterLast('/') ?: getString(R.string.player_subtitle_loaded)
        return true
    }

    private fun inferSubtitleMimeType(uri: android.net.Uri): String? {
        val resolverType = contentResolver.getType(uri)?.lowercase()
        if (resolverType != null) {
            when {
                resolverType.contains("vtt") -> return MimeTypes.TEXT_VTT
                resolverType.contains("subrip") || resolverType.contains("srt") -> return MimeTypes.APPLICATION_SUBRIP
                resolverType.contains("ssa") || resolverType.contains("ass") -> return MimeTypes.TEXT_SSA
                resolverType.contains("ttml") || resolverType.contains("xml") -> return MimeTypes.APPLICATION_TTML
            }
        }

        return when (uri.lastPathSegment?.substringAfterLast('.', "")?.lowercase()) {
            "srt" -> MimeTypes.APPLICATION_SUBRIP
            "vtt", "webvtt" -> MimeTypes.TEXT_VTT
            "ssa", "ass" -> MimeTypes.TEXT_SSA
            "ttml", "dfxp", "xml" -> MimeTypes.APPLICATION_TTML
            else -> null
        }
    }

    private suspend fun monitorPlaybackTorrent(infoHash: String) {
        var hasSeenPlaybackTorrent = false

        app.engineServiceRepository.state.collectLatest { state ->
            if (screenState !is PlayerScreenState.Ready || isFinishing || isDestroyed) {
                return@collectLatest
            }

            val torrents = state?.torrents ?: return@collectLatest
            val torrent = torrents.firstOrNull { it.infoHash == infoHash }
            if (torrent != null) {
                hasSeenPlaybackTorrent = true
            }

            val playbackUnavailable = (hasSeenPlaybackTorrent && torrent == null) ||
                torrent?.userState == "stopped" ||
                torrent?.status == "stopped"

            if (playbackUnavailable) {
                releasePlayer()
                setFullscreenMode(false)
                finish()
            }
        }
    }

    private fun setFullscreenMode(enabled: Boolean) {
        isFullscreen = enabled
        applySystemBarsVisibility()
    }

    private fun setPlaybackSessionRegistered(enabled: Boolean) {
        if (playbackSessionRegistered == enabled) return
        playbackSessionRegistered = enabled

        if (enabled) {
            app.serviceLifecycleManager.onPlaybackSessionStarted()
        } else {
            app.serviceLifecycleManager.onPlaybackSessionStopped()
        }
    }

    private fun applySystemBarsVisibility() {
        val controller = WindowCompat.getInsetsController(window, window.decorView)
        controller.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE

        if (isFullscreen && !isInPictureInPictureUiMode) {
            controller.hide(WindowInsetsCompat.Type.systemBars())
        } else {
            controller.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    private fun enterPictureInPictureIfPossible(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return false
        if (isInPictureInPictureUiMode) return false
        if (screenState !is PlayerScreenState.Ready || player == null) return false
        if (isFinishing || isDestroyed) return false

        val params = buildPictureInPictureParams() ?: return false
        return enterPictureInPictureMode(params)
    }

    private fun updatePictureInPictureParams() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        buildPictureInPictureParams()?.let(::setPictureInPictureParams)
    }

    private fun buildPictureInPictureParams(): PictureInPictureParams? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return null

        val builder = PictureInPictureParams.Builder()
        val videoSize = player?.videoSize
        if (videoSize != null && videoSize.width > 0 && videoSize.height > 0) {
            builder.setAspectRatio(Rational(videoSize.width, videoSize.height))
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val canAutoEnter = screenState is PlayerScreenState.Ready && player != null
            builder.setAutoEnterEnabled(canAutoEnter)
        }
        return builder.build()
    }

    private fun buildPlayerErrorMessage(error: androidx.media3.common.PlaybackException): String {
        val rootCause = generateSequence(error.cause) { it.cause }.lastOrNull()
        val usefulCause = generateSequence(error.cause) { it.cause }
            .firstOrNull { it.message?.isNotBlank() == true }
            ?: rootCause

        val message = usefulCause?.message?.takeIf { it.isNotBlank() }
            ?: error.localizedMessage
            ?: getString(R.string.player_unknown_error)

        return if (usefulCause is IOException) {
            message
        } else {
            message
        }
    }
}

private sealed interface PlayerLaunchSource {
    data class Stream(val request: PlayerLaunchRequest) : PlayerLaunchSource
    data class Local(val request: LocalPlaybackRequest) : PlayerLaunchSource
}

private sealed interface PlayerScreenState {
    data object Preparing : PlayerScreenState
    data class Ready(val fileName: String) : PlayerScreenState
    data class Error(val message: String) : PlayerScreenState
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PlayerActivityScreen(
    state: PlayerScreenState,
    player: ExoPlayer?,
    bufferingMessage: String?,
    playerErrorMessage: String?,
    isFullscreen: Boolean,
    isInPictureInPicture: Boolean,
    hasExternalSubtitle: Boolean,
    onSetFullscreen: (Boolean) -> Unit,
    onEnterPictureInPicture: () -> Unit,
    onLoadSubtitle: () -> Unit,
    onClearSubtitle: () -> Unit,
    onClose: () -> Unit
) {
    val title = when (state) {
        PlayerScreenState.Preparing -> stringResource(R.string.player_title)
        is PlayerScreenState.Ready -> state.fileName
        is PlayerScreenState.Error -> stringResource(R.string.player_error_title)
    }

    if ((isFullscreen || isInPictureInPicture) && state is PlayerScreenState.Ready) {
        PlayerReadyContent(
            modifier = Modifier.fillMaxSize(),
            player = player,
            bufferingMessage = bufferingMessage,
            playerErrorMessage = playerErrorMessage,
            isFullscreen = true,
            isInPictureInPicture = isInPictureInPicture,
            hasExternalSubtitle = hasExternalSubtitle,
            onSetFullscreen = onSetFullscreen,
            onEnterPictureInPicture = onEnterPictureInPicture,
            onLoadSubtitle = onLoadSubtitle,
            onClearSubtitle = onClearSubtitle,
            onClose = onClose
        )
        return
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = onClose) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.player_close)
                        )
                    }
                },
                actions = {
                    if (state is PlayerScreenState.Ready) {
                        PlayerOverflowMenu(
                            hasExternalSubtitle = hasExternalSubtitle,
                            onLoadSubtitle = onLoadSubtitle,
                            onClearSubtitle = onClearSubtitle
                        )
                        IconButton(onClick = onEnterPictureInPicture) {
                            Icon(
                                imageVector = Icons.Filled.PictureInPictureAlt,
                                contentDescription = stringResource(R.string.player_enter_picture_in_picture)
                            )
                        }
                        IconButton(onClick = { onSetFullscreen(true) }) {
                            Icon(
                                imageVector = Icons.Filled.Fullscreen,
                                contentDescription = stringResource(R.string.player_enter_fullscreen)
                            )
                        }
                    }
                }
            )
        }
    ) { innerPadding ->
        when (state) {
            PlayerScreenState.Preparing -> {
                LoadingState(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(innerPadding),
                    title = stringResource(R.string.player_preparing_title),
                    message = stringResource(R.string.player_preparing_message)
                )
            }

            is PlayerScreenState.Ready -> {
                PlayerReadyContent(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(innerPadding),
                    player = player,
                    bufferingMessage = bufferingMessage,
                    playerErrorMessage = playerErrorMessage,
                    isFullscreen = false,
                    isInPictureInPicture = false,
                    hasExternalSubtitle = hasExternalSubtitle,
                    onSetFullscreen = onSetFullscreen,
                    onEnterPictureInPicture = onEnterPictureInPicture,
                    onLoadSubtitle = onLoadSubtitle,
                    onClearSubtitle = onClearSubtitle,
                    onClose = onClose
                )
            }

            is PlayerScreenState.Error -> {
                LoadingState(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(innerPadding),
                    title = stringResource(R.string.player_error_title),
                    message = state.message,
                    showSpinner = false
                )
            }
        }
    }
}

@Composable
private fun PlayerReadyContent(
    modifier: Modifier = Modifier,
    player: ExoPlayer?,
    bufferingMessage: String?,
    playerErrorMessage: String?,
    isFullscreen: Boolean,
    isInPictureInPicture: Boolean,
    hasExternalSubtitle: Boolean,
    onSetFullscreen: (Boolean) -> Unit,
    onEnterPictureInPicture: () -> Unit,
    onLoadSubtitle: () -> Unit,
    onClearSubtitle: () -> Unit,
    onClose: () -> Unit
) {
    val currentOnSetFullscreen = rememberUpdatedState(onSetFullscreen)

    BackHandler(enabled = isFullscreen && !isInPictureInPicture) {
        currentOnSetFullscreen.value(false)
    }

    Box(
        modifier = modifier.background(MaterialTheme.colorScheme.surface)
    ) {
        player?.let { exoPlayer ->
            AndroidView(
                modifier = Modifier.fillMaxSize(),
                factory = { context ->
                    val density = context.resources.displayMetrics.density
                    val swipeDistanceThreshold = 72f * density
                    val swipeVelocityThreshold = 240f * density
                    val tapSlop = ViewConfiguration.get(context).scaledTouchSlop.toFloat()
                    val tapTimeoutMs = ViewConfiguration.getTapTimeout().toLong()
                    val doubleTapTimeoutMs = ViewConfiguration.getDoubleTapTimeout().toLong()
                    val tapHandler = Handler(Looper.getMainLooper())
                    var touchDownX = 0f
                    var touchDownY = 0f
                    var playerViewWidth = 0
                    var rightTapCount = 0
                    var holdToSpeedActive = false
                    var originalPlaybackParameters = exoPlayer.playbackParameters

                    fun isRightSide(x: Float, width: Int): Boolean = x >= width * 0.5f

                    val applyAccumulatedSeek = Runnable {
                        if (rightTapCount >= 2) {
                            val seekDeltaMs = (rightTapCount - 1) * YOUTUBE_STYLE_SEEK_MS
                            val duration = exoPlayer.duration
                            val targetPosition = if (duration > 0) {
                                min(exoPlayer.currentPosition + seekDeltaMs, duration)
                            } else {
                                exoPlayer.currentPosition + seekDeltaMs
                            }
                            exoPlayer.seekTo(targetPosition)
                        }
                        rightTapCount = 0
                    }

                    val gestureDetector = GestureDetector(
                        context,
                        object : GestureDetector.SimpleOnGestureListener() {
                            override fun onDown(e: MotionEvent): Boolean {
                                touchDownX = e.x
                                touchDownY = e.y
                                return true
                            }

                            override fun onLongPress(e: MotionEvent) {
                                if (!isRightSide(e.x, playerViewWidth)) return
                                if (holdToSpeedActive) return

                                originalPlaybackParameters = exoPlayer.playbackParameters
                                holdToSpeedActive = true
                                exoPlayer.playbackParameters = PlaybackParameters(
                                    2f,
                                    originalPlaybackParameters.pitch
                                )
                            }

                            override fun onFling(
                                e1: MotionEvent?,
                                e2: MotionEvent,
                                velocityX: Float,
                                velocityY: Float
                            ): Boolean {
                                val start = e1 ?: return false
                                val deltaX = e2.x - start.x
                                val deltaY = e2.y - start.y
                                if (abs(deltaY) <= abs(deltaX)) return false
                                if (abs(deltaY) < swipeDistanceThreshold) return false
                                if (abs(velocityY) < swipeVelocityThreshold) return false

                                if (deltaY < 0f) {
                                    currentOnSetFullscreen.value(true)
                                    return true
                                }

                                currentOnSetFullscreen.value(false)
                                return true
                            }
                        }
                    )

                    PlayerView(context).apply {
                        this.player = exoPlayer
                        useController = !isInPictureInPicture
                        setShutterBackgroundColor(Color.BLACK)
                        keepScreenOn = true
                        setOnTouchListener { view, event ->
                            playerViewWidth = view.width
                            gestureDetector.onTouchEvent(event)
                            when (event.actionMasked) {
                                MotionEvent.ACTION_UP -> {
                                    if (holdToSpeedActive) {
                                        exoPlayer.playbackParameters = originalPlaybackParameters
                                        holdToSpeedActive = false
                                    }

                                    val isTap =
                                        abs(event.x - touchDownX) <= tapSlop &&
                                            abs(event.y - touchDownY) <= tapSlop &&
                                            (event.eventTime - event.downTime) <= tapTimeoutMs

                                    if (isTap && isRightSide(event.x, view.width)) {
                                        tapHandler.removeCallbacks(applyAccumulatedSeek)
                                        rightTapCount += 1
                                        tapHandler.postDelayed(applyAccumulatedSeek, doubleTapTimeoutMs)
                                    } else if (isTap) {
                                        tapHandler.removeCallbacks(applyAccumulatedSeek)
                                        rightTapCount = 0
                                    }
                                }

                                MotionEvent.ACTION_CANCEL -> {
                                    if (holdToSpeedActive) {
                                        exoPlayer.playbackParameters = originalPlaybackParameters
                                        holdToSpeedActive = false
                                    }
                                    tapHandler.removeCallbacks(applyAccumulatedSeek)
                                    rightTapCount = 0
                                }
                            }
                            false
                        }
                    }
                },
                update = { view ->
                    view.player = exoPlayer
                    view.useController = !isInPictureInPicture
                }
            )
        }

        if (bufferingMessage != null) {
            LoadingState(
                modifier = Modifier
                    .align(Alignment.Center)
                    .padding(24.dp),
                title = stringResource(R.string.player_loading_video),
                message = bufferingMessage
            )
        }

        if (playerErrorMessage != null) {
            Surface(
                modifier = Modifier
                    .align(if (isFullscreen) Alignment.TopStart else Alignment.TopCenter)
                    .statusBarsPadding()
                    .padding(16.dp),
                color = MaterialTheme.colorScheme.errorContainer,
                tonalElevation = 4.dp
            ) {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    Text(
                        text = stringResource(R.string.player_playback_failed),
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                        fontWeight = FontWeight.SemiBold
                    )
                    Text(
                        text = playerErrorMessage,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onErrorContainer
                    )
                }
            }
        }
    }
}

@Composable
private fun PlayerOverflowMenu(
    hasExternalSubtitle: Boolean,
    onLoadSubtitle: () -> Unit,
    onClearSubtitle: () -> Unit
) {
    var expanded by remember { mutableStateOf(false) }

    Box {
        IconButton(onClick = { expanded = true }) {
            Icon(
                imageVector = Icons.Filled.MoreVert,
                contentDescription = stringResource(R.string.player_more_options)
            )
        }

        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false }
        ) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.player_load_subtitles)) },
                onClick = {
                    expanded = false
                    onLoadSubtitle()
                }
            )
            if (hasExternalSubtitle) {
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.player_remove_subtitles)) },
                    onClick = {
                        expanded = false
                        onClearSubtitle()
                    }
                )
            }
        }
    }
}

@Composable
private fun LoadingState(
    modifier: Modifier = Modifier,
    title: String,
    message: String,
    showSpinner: Boolean = true
) {
    Box(
        modifier = modifier,
        contentAlignment = Alignment.Center
    ) {
        Column(
            modifier = Modifier.padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            if (showSpinner) {
                CircularProgressIndicator()
            }
            Text(
                text = title,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold
            )
            Text(
                text = message,
                style = MaterialTheme.typography.bodyLarge
            )
        }
    }
}
