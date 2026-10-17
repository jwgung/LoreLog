package com.lifelog.speaker

import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import ai.picovoice.eagle.Eagle
import ai.picovoice.eagle.EagleProfile
import ai.picovoice.eagle.EagleProfiler
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * 내 목소리 등록(한 번) 후 녹음 조각마다 "나/상대/모름"을 판별한다.
 * Picovoice Eagle(온디바이스)로 성문 임베딩을 만들고 대조한다. 소리는 기기 밖으로 안 나간다.
 */
object SpeakerId {
    private const val PREF = "lifelog_secret"
    private const val KEY_ACCESS = "eagle_access_key"
    private const val THRESHOLD = 0.5f

    @Volatile private var eagle: Eagle? = null
    @Volatile private var profile: EagleProfile? = null

    private fun prefs(context: Context) = EncryptedSharedPreferences.create(
        context.applicationContext,
        PREF,
        MasterKey.Builder(context.applicationContext)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    fun accessKey(context: Context): String? =
        prefs(context).getString(KEY_ACCESS, null)?.takeIf { it.isNotBlank() }

    fun setAccessKey(context: Context, key: String) {
        prefs(context).edit().putString(KEY_ACCESS, key.trim()).apply()
        release()
    }

    private fun profileFile(context: Context) = File(context.applicationContext.filesDir, "speaker.profile")
    fun isEnrolled(context: Context): Boolean = profileFile(context).exists()
    fun isConfigured(context: Context): Boolean = accessKey(context) != null

    fun clearEnrollment(context: Context) {
        profileFile(context).delete()
        release()
    }

    fun release() {
        try { eagle?.delete() } catch (_: Exception) {}
        try { profile?.delete() } catch (_: Exception) {}
        eagle = null
        profile = null
    }

    /** 마이크로 내 목소리를 녹음해 등록한다. 진행률(0~100)과 피드백을 콜백으로 알린다. 완료면 true. */
    suspend fun enroll(context: Context, onProgress: (Float, String) -> Unit): Boolean = withContext(Dispatchers.IO) {
        val key = accessKey(context) ?: run { onProgress(-1f, "키 필요"); return@withContext false }
        var profiler: EagleProfiler? = null
        var recorder: AudioRecord? = null
        try {
            profiler = EagleProfiler.Builder().setAccessKey(key).build(context)
            val sampleRate = profiler.sampleRate
            val frameLen = profiler.frameLength
            val minBuf = AudioRecord.getMinBufferSize(sampleRate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            recorder = AudioRecord(
                MediaRecorder.AudioSource.MIC, sampleRate,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minBuf, frameLen * 4),
            )
            recorder.startRecording()
            val frame = ShortArray(frameLen)
            var pct = 0f
            var guard = 0
            while (pct < 100f && guard < 800) {
                var read = 0
                while (read < frame.size) {
                    val r = recorder.read(frame, read, frame.size - read)
                    if (r <= 0) break
                    read += r
                }
                guard++
                if (read < frame.size) continue
                pct = try { profiler.enroll(frame) } catch (_: Exception) { pct }
                onProgress(pct, "녹음 중")
            }
            if (pct >= 100f) {
                val profile = profiler.export()
                profileFile(context).writeBytes(profile.bytes)
                release()
                true
            } else {
                onProgress(pct, "미완료")
                false
            }
        } catch (e: Exception) {
            onProgress(-1f, e.message ?: "오류")
            false
        } finally {
            try { recorder?.stop() } catch (_: Exception) {}
            try { recorder?.release() } catch (_: Exception) {}
            profiler?.delete()
        }
    }

    private fun ensureEagle(context: Context): Eagle? {
        eagle?.let { return it }
        val key = accessKey(context) ?: return null
        if (!isEnrolled(context)) return null
        return try {
            profile = EagleProfile(profileFile(context).readBytes())
            Eagle.Builder().setAccessKey(key).build(context).also { eagle = it }
        } catch (e: Exception) {
            null
        }
    }

    /** 조각 PCM에서 내 목소리 비중을 보고 "나/상대/모름" 반환. 미설정이면 null. */
    fun labelFor(context: Context, pcm: ShortArray): String? {
        if (pcm.isEmpty()) return null
        val e = ensureEagle(context) ?: return null
        val prof = profile ?: return null
        val profiles = arrayOf(prof)
        val frameLen = e.minProcessSamples
        if (pcm.size < frameLen) return null
        var sum = 0f
        var n = 0
        try {
            var i = 0
            while (i + frameLen <= pcm.size) {
                val scores = e.process(pcm.copyOfRange(i, i + frameLen), profiles)
                sum += scores.getOrElse(0) { 0f }
                n++
                i += frameLen
            }
        } catch (ex: Exception) {
            return null
        }
        if (n == 0) return "모름"
        val avg = sum / n
        return when {
            avg >= THRESHOLD -> "나"
            avg >= THRESHOLD * 0.4f -> "모름"
            else -> "상대"
        }
    }
}
