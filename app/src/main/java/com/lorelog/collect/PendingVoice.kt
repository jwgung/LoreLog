package com.lorelog.collect

import android.content.Context
import com.lorelog.data.LogRepository
import com.lorelog.net.GroqTranscriber
import com.lorelog.net.SttConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * 전사 대기 음성 보관소. AI(Groq) 연결이 없으면 말 구간 WAV 를 앱 전용 외부 폴더에 쌓아
 * 두고(= 음성파일로 보관/반출 가능), 연결되면 그 파일만 텍스트로 변환한다.
 * 파일명에 화자 라벨을 담아(seg_<ts>__<라벨>.wav) 나중에 전사해도 라벨을 보존한다.
 */
object PendingVoice {
    private const val DIR = "voice_pending"
    private val NAME_LABEL = Regex("__(.+)\\.wav$")

    fun dir(context: Context): File {
        val base = context.applicationContext.getExternalFilesDir(null) ?: context.applicationContext.cacheDir
        return File(base, DIR).apply { mkdirs() }
    }

    fun pendingCount(context: Context): Int =
        dir(context).listFiles()?.count { it.name.endsWith(".wav") } ?: 0

    fun fileFor(context: Context, label: String?): File =
        File(dir(context), "seg_${System.currentTimeMillis()}__${label ?: "none"}.wav")

    /** 파일 하나 전사. 성공하면 저장·삭제하고 true. 실패하면 파일을 남긴다. */
    suspend fun transcribeFile(context: Context, f: File): Boolean = withContext(Dispatchers.IO) {
        val text = GroqTranscriber.transcribe(context, f) ?: return@withContext false
        val label = NAME_LABEL.find(f.name)?.groupValues?.get(1)?.takeIf { it.isNotBlank() && it != "none" }
        LogRepository.addVoice(text, speaker = label)
        f.delete()
        true
    }

    /** 밀린 음성파일을 모두 전사. 전사한 개수 반환. 키가 없으면 0(파일은 그대로 보관). */
    suspend fun transcribePending(context: Context): Int = withContext(Dispatchers.IO) {
        if (!SttConfig.isConfigured(context)) return@withContext 0
        var n = 0
        dir(context).listFiles()?.sortedBy { it.name }?.forEach { f ->
            if (f.name.endsWith(".wav") && transcribeFile(context, f)) n++
        }
        n
    }
}
