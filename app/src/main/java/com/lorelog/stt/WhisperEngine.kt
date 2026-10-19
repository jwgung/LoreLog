package com.lorelog.stt

import android.content.Context
import com.lorelog.net.TranscribeConfig
import com.lorelog.net.WhisperModels
import dev.ffmpegkit.whisper.Whisper
import dev.ffmpegkit.whisper.WhisperConfig
import dev.ffmpegkit.whisper.WhisperModel
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.File

/**
 * 온디바이스 whisper.cpp 전사. 모델을 1회 로드해 캐시하고, WAV 를 한국어로 전사한다.
 * 모델 선택이 바뀌면 이전 모델을 풀고 새로 로드한다. 미지원 ABI 등은 null 로 떨어진다.
 */
object WhisperEngine {
    private val lock = Mutex()
    private var loadedId: String? = null
    private var model: WhisperModel? = null

    fun isReady(context: Context): Boolean =
        WhisperModels.isReady(context, TranscribeConfig.modelId(context))

    suspend fun transcribe(context: Context, file: File): String? = lock.withLock {
        val id = TranscribeConfig.modelId(context)
        val path = WhisperModels.fileFor(context, id)
        if (!path.exists() || path.length() < 1_000_000L) return@withLock null
        try {
            if (loadedId != id || model == null) {
                model?.let { Whisper.releaseModel(it) }
                model = null
                model = Whisper.loadModel(context, path.absolutePath)
                loadedId = id
            }
            val m = model ?: return@withLock null
            val res = Whisper.transcribe(m, file.absolutePath, WhisperConfig(language = "ko"))
            res.text.trim().takeIf { it.isNotEmpty() }
        } catch (e: Throwable) {
            runCatching { model?.let { Whisper.releaseModel(it) } }
            model = null
            loadedId = null
            null
        }
    }
}
