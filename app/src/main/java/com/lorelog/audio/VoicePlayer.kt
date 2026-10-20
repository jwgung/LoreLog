package com.lorelog.audio

import android.media.AudioAttributes
import android.media.MediaPlayer
import android.util.Log
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File

/**
 * 타임라인에서 음성 조각을 들어보는 단순 재생기. 한 번에 하나만 재생하고, 같은 파일을 다시 누르면 멈춘다.
 * 녹음(AudioRecord)과는 별개라 기록 중에도 재생된다.
 */
object VoicePlayer {
    private const val TAG = "VoicePlayer"
    private var player: MediaPlayer? = null

    /** 지금 재생 중인 파일 경로. 없으면 null. */
    private val _playing = MutableStateFlow<String?>(null)
    val playing: StateFlow<String?> = _playing.asStateFlow()

    /** 최근 재생한 조각의 텍스트(시각, 정규화 텍스트). 되돌아온 소리를 전사한 결과를 걸러낼 때 비교한다. */
    private val recent = ArrayDeque<Pair<Long, String>>()
    private const val RECENT_MAX = 20
    private const val RECENT_WINDOW_MS = 30 * 60 * 1000L
    private const val ECHO_THRESHOLD = 0.6

    /** [text] 는 이 파일의 전사 텍스트(에코 판정용). 대기 파일처럼 없으면 null. */
    fun toggle(file: File, text: String? = null) {
        if (_playing.value == file.absolutePath) { stop(); return }
        stop()
        if (!file.exists()) { Log.w(TAG, "missing ${file.name}"); return }
        text?.let { remember(it) }
        try {
            val p = MediaPlayer()
            p.setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            p.setDataSource(file.absolutePath)
            p.setOnCompletionListener { stop() }
            p.setOnErrorListener { _, what, extra -> Log.w(TAG, "error $what/$extra"); stop(); true }
            p.prepare()
            p.start()
            player = p
            _playing.value = file.absolutePath
        } catch (e: Exception) {
            Log.w(TAG, "play failed: ${e.message}")
            stop()
        }
    }

    @Synchronized
    private fun remember(text: String) {
        val n = normalize(text)
        if (n.length < 4) return
        recent.addLast(System.currentTimeMillis() to n)
        while (recent.size > RECENT_MAX) recent.removeFirst()
    }

    /**
     * [captured] 가 최근 재생한 텍스트의 되울림인가. 글자 2-gram 중 재생 텍스트에도 있는 비율이
     * [ECHO_THRESHOLD] 이상이면 에코. 그 위에 실제 대화가 섞이면 비율이 떨어져 남는다.
     */
    @Synchronized
    fun isEcho(captured: String): Boolean {
        val now = System.currentTimeMillis()
        while (recent.isNotEmpty() && now - recent.first().first > RECENT_WINDOW_MS) recent.removeFirst()
        if (recent.isEmpty()) return false
        val c = normalize(captured)
        if (c.length < 4) return recent.any { it.second.contains(c) }
        val grams = bigrams(c)
        if (grams.isEmpty()) return false
        return recent.any { (_, played) ->
            val pg = bigrams(played)
            val hit = grams.count { it in pg }
            hit.toDouble() / grams.size >= ECHO_THRESHOLD
        }
    }

    private fun normalize(t: String): String = t.lowercase().filter { it.isLetterOrDigit() }

    private fun bigrams(t: String): Set<String> =
        if (t.length < 2) emptySet() else (0 until t.length - 1).mapTo(HashSet()) { t.substring(it, it + 2) }

    fun stop() {
        player?.let { p ->
            runCatching { if (p.isPlaying) p.stop() }
            runCatching { p.release() }
        }
        player = null
        _playing.value = null
    }
}
