package com.lorelog.collect

import android.content.Context
import com.lorelog.data.LogRepository
import com.lorelog.net.GeminiTranscriber
import com.lorelog.net.LlmConfig
import com.lorelog.net.TranscribeConfig
import com.lorelog.stt.WhisperEngine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * 전사 대기 음성 보관소. 전사 엔진(Gemini/온디바이스 whisper)이 준비 안 됐으면 말 구간
 * WAV 를 앱 전용 외부 폴더에 쌓아 두고(= 음성파일로 보관/반출 가능), 준비되면 그 파일만 변환한다.
 * 파일명에 화자 라벨을 담아(seg_<ts>__<라벨>.wav) 나중에 전사해도 라벨을 보존한다.
 */
object PendingVoice {
    private const val DIR = "voice_pending"
    private val NAME_LABEL = Regex("__(.+)\\.wav$")

    /** 현재 선택된 엔진이 전사할 준비가 됐는지(키 or 모델). */
    fun isReady(context: Context): Boolean = when (TranscribeConfig.engine(context)) {
        TranscribeConfig.Engine.GEMINI -> LlmConfig.isConfigured(context)
        TranscribeConfig.Engine.WHISPER -> WhisperEngine.isReady(context)
    }

    fun dir(context: Context): File {
        val base = context.applicationContext.getExternalFilesDir(null) ?: context.applicationContext.cacheDir
        return File(base, DIR).apply { mkdirs() }
    }

    fun pendingCount(context: Context): Int =
        dir(context).listFiles()?.count { it.name.endsWith(".wav") } ?: 0

    fun fileFor(context: Context, label: String?): File =
        File(dir(context), "seg_${System.currentTimeMillis()}__${label ?: "none"}.wav")

    /** 선택된 엔진으로 파일 하나를 전사해 텍스트를 돌려준다(저장은 호출자 몫). 실패 시 null. */
    suspend fun transcribeViaEngine(context: Context, f: File): String? = withContext(Dispatchers.IO) {
        when (TranscribeConfig.engine(context)) {
            TranscribeConfig.Engine.GEMINI -> GeminiTranscriber.transcribe(context, f)
            TranscribeConfig.Engine.WHISPER -> WhisperEngine.transcribe(context, f)
        }
    }

    /** 파일 하나 전사. 성공하면 저장·삭제하고 true. 실패하면 파일을 남긴다. */
    suspend fun transcribeFile(context: Context, f: File): Boolean = withContext(Dispatchers.IO) {
        val text = transcribeViaEngine(context, f) ?: return@withContext false
        val label = NAME_LABEL.find(f.name)?.groupValues?.get(1)?.takeIf { it.isNotBlank() && it != "none" }
        LogRepository.addVoice(text, speaker = label)
        f.delete()
        true
    }

    /** 밀린 음성파일을 모두 전사. 전사한 개수 반환. 준비 안 됐으면 0(파일은 그대로 보관). */
    suspend fun transcribePending(context: Context): Int = withContext(Dispatchers.IO) {
        if (!isReady(context)) return@withContext 0
        var n = 0
        dir(context).listFiles()?.sortedBy { it.name }?.forEach { f ->
            if (f.name.endsWith(".wav") && transcribeFile(context, f)) n++
        }
        n
    }
}
