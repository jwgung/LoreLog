package com.lorelog.collect

import android.content.Context
import android.util.Log
import com.lorelog.audio.VoiceAudio
import com.lorelog.audio.VoicePlayer
import com.lorelog.data.LogRepository
import com.lorelog.data.PendingAudio
import com.lorelog.net.ServerTranscriber
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * 전사 대기 음성 보관소. 전사는 금고가 한다(사용자가 고른 음성 모델, [ServerTranscriber]). 금고에 닿지 않으면
 * 말 구간 WAV 를 앱 전용 외부 폴더에 쌓아 두고(= 음성파일로 보관/반출 가능), 닿으면 그 파일을 보낸다.
 * 파일명에 화자 라벨을 담아(seg_<ts>__<라벨>.wav) 나중에 전사해도 라벨을 보존한다.
 * 대기 파일은 타임라인에 "전사 대기"로 보이고 바로 들어볼 수 있다([refresh]).
 */
object PendingVoice {
    private const val TAG = "PendingVoice"
    private const val DIR = "voice_pending"
    private val NAME_LABEL = Regex("__(.+)\\.wav$")
    private val NAME_TS = Regex("^seg_(\\d+)__")
    /** 앱이 음성을 재생하는 동안 잡힌 묶음의 화자 라벨. 되돌아온 소리일 수 있다는 표시. */
    const val LABEL_PLAYBACK = "재생 중"

    fun dir(context: Context): File {
        val base = context.applicationContext.getExternalFilesDir(null) ?: context.applicationContext.cacheDir
        return File(base, DIR).apply { mkdirs() }
    }

    fun pendingCount(context: Context): Int =
        dir(context).listFiles()?.count { it.name.endsWith(".wav") } ?: 0

    /** 대기 파일 목록을 다시 읽어 타임라인("전사 대기" 항목)에 반영한다. 파일을 만들거나 지운 뒤 부른다. */
    fun refresh(context: Context) {
        val list = dir(context).listFiles()?.filter { it.name.endsWith(".wav") }?.map {
            PendingAudio(it.absolutePath, recordedAt(it), VoiceAudio.wavDurationMs(it))
        } ?: emptyList()
        LogRepository.setPendingVoice(list)
    }

    /** 대기 파일 경로. 파일명의 시각(atMs)이 곧 기록 시각이 되므로 첫마디 시각을 넘긴다. */
    fun fileFor(context: Context, label: String?, atMs: Long = System.currentTimeMillis()): File =
        File(dir(context), "seg_${atMs}__${label ?: "none"}.wav")

    /** 파일명의 녹음 시각(seg_<ts>__). 없으면 파일 수정 시각. 밀린 파일도 실제 말한 시각으로 기록한다. */
    private fun recordedAt(f: File): Long =
        NAME_TS.find(f.name)?.groupValues?.get(1)?.toLongOrNull() ?: f.lastModified().takeIf { it > 0 } ?: System.currentTimeMillis()

    /**
     * 파일 하나 전사. 성공하면 저장·삭제하고 true. 말소리가 없었으면("") 저장 없이 삭제하고 true.
     * 실패(null)하면 파일을 남기고 false.
     */
    suspend fun transcribeFile(context: Context, f: File): Boolean = withContext(Dispatchers.IO) {
        val text = ServerTranscriber.transcribe(context, f)
        if (text == null) {
            Log.w(TAG, "transcribe failed, keep ${f.name} (${f.length()} bytes)")
            return@withContext false
        }
        if (text.isEmpty()) {
            Log.i(TAG, "no speech in ${f.name}, drop")
            f.delete()
            refresh(context)
            return@withContext true
        }
        Log.i(TAG, "transcribed ${f.name}: ${text.length} chars")
        val label = NAME_LABEL.find(f.name)?.groupValues?.get(1)?.takeIf { it.isNotBlank() && it != "none" }
        if (label == LABEL_PLAYBACK && VoicePlayer.isEcho(text)) {
            // 재생한 음성이 마이크로 되돌아와 다시 전사된 것. 대화가 섞였으면 유사도가 낮아 여기 안 걸린다.
            Log.i(TAG, "echo of played audio in ${f.name}, drop")
            f.delete()
            refresh(context)
            return@withContext true
        }
        val at = recordedAt(f)
        // 다시 들어볼 수 있게 오디오를 압축 보관(내보낸 뒤/7일 뒤 삭제). keep 이 WAV 를 옮기거나 지운다.
        val audio = VoiceAudio.keep(context, f, at)
        if (audio == null) f.delete()
        LogRepository.addVoice(text, speaker = label, timestampMs = at, audio = audio)
        refresh(context)
        true
    }

    /**
     * 밀린 음성파일을 오래된 것부터 금고로 보내 전사. 전사한 개수 반환. 금고 주소가 없으면 0(파일은 그대로 보관).
     * 한 번 실패하면(금고에 닿지 않음) 멈춘다. 남은 파일은 워커가 나중에 이어서 보낸다.
     */
    suspend fun transcribePending(context: Context): Int = withContext(Dispatchers.IO) {
        refresh(context)
        if (!ServerTranscriber.isReady(context)) return@withContext 0
        var n = 0
        val files = dir(context).listFiles()?.filter { it.name.endsWith(".wav") }?.sortedBy { it.name } ?: emptyList()
        for (f in files) {
            if (!transcribeFile(context, f)) {
                Log.w(TAG, "stop pending after failure at ${f.name} ($n done)")
                break
            }
            n++
        }
        n
    }
}
