package com.lorelog.stt

import android.content.Context
import com.lorelog.net.WhisperModels
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/** whisper 모델(.bin)을 filesDir/whisper 로 내려받는다. 진행률을 StateFlow 로 노출. */
object ModelDownloader {
    sealed class State {
        data object Idle : State()
        data class Downloading(val id: String, val pct: Int) : State()
        data class Done(val id: String) : State()
        data class Failed(val id: String, val reason: String) : State()
    }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state.asStateFlow()

    suspend fun download(context: Context, id: String): Boolean = withContext(Dispatchers.IO) {
        val info = WhisperModels.byId(id)
        val dest = WhisperModels.fileFor(context, id)
        if (dest.exists() && dest.length() > 1_000_000L) {
            _state.value = State.Done(id); return@withContext true
        }
        val tmp = File(dest.parentFile, dest.name + ".part")
        var conn: HttpURLConnection? = null
        try {
            _state.value = State.Downloading(id, 0)
            conn = (URL(info.url).openConnection() as HttpURLConnection).apply {
                connectTimeout = 20_000
                readTimeout = 60_000
                instanceFollowRedirects = true
            }
            if (conn.responseCode !in 200..299) {
                _state.value = State.Failed(id, "HTTP ${conn.responseCode}"); return@withContext false
            }
            val total = conn.contentLengthLong.takeIf { it > 0 } ?: (info.approxMb * 1_000_000L)
            conn.inputStream.use { input ->
                tmp.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    var read: Int
                    var done = 0L
                    var lastPct = -1
                    while (input.read(buf).also { read = it } >= 0) {
                        out.write(buf, 0, read)
                        done += read
                        val pct = ((done * 100) / total).toInt().coerceIn(0, 100)
                        if (pct != lastPct) { lastPct = pct; _state.value = State.Downloading(id, pct) }
                    }
                }
            }
            if (dest.exists()) dest.delete()
            if (!tmp.renameTo(dest)) { tmp.copyTo(dest, overwrite = true); tmp.delete() }
            _state.value = State.Done(id)
            true
        } catch (e: Exception) {
            tmp.delete()
            _state.value = State.Failed(id, e.message ?: "실패")
            false
        } finally {
            conn?.disconnect()
        }
    }
}
