package com.lorelog.collect

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import com.lorelog.net.ServerTranscriber
import com.lorelog.net.TranscribeConfig
import com.lorelog.stt.WhisperEngine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.File

/**
 * 대화 화면의 "말하기". 마이크로 한마디를 녹음해 내 서버 또는 온디바이스 whisper 로만 받아 적는다.
 * 일기에 남기는 말은 가장 사적이라, 수집용 전사 설정이 Gemini(클라우드)여도 이쪽은 절대 외부로 보내지 않는다.
 * 상시 기록이 마이크를 쥐고 있으면 잠깐 양보시키고, 끝나면 되돌린다. 녹음 파일은 전사 뒤 바로 지운다.
 */
object Dictation {
    enum class State { IDLE, LISTENING, TRANSCRIBING }

    private val _state = MutableStateFlow(State.IDLE)
    val state: StateFlow<State> = _state

    @Volatile private var stopRequested = false
    private const val SAMPLE_RATE = 16_000
    private const val MAX_MS = 120_000L

    /** 말하기에 쓸 수 있나(온디바이스 모델이 내려받아져 있나). */
    fun available(context: Context): Boolean =
        if (TranscribeConfig.engine(context) == TranscribeConfig.Engine.SERVER) ServerTranscriber.isReady(context)
        else WhisperEngine.isReady(context)

    /** 듣는 중이면 끝내고 받아 적기로 넘어간다. */
    fun stop() { stopRequested = true }

    /** 녹음해서 받아 적은 글을 돌려준다. 못 열었거나 말이 없었거나 전사에 실패하면 null. RECORD_AUDIO 권한은 호출 전에 받아 둔다. */
    @SuppressLint("MissingPermission")
    suspend fun listen(context: Context): String? = withContext(Dispatchers.IO) {
        if (_state.value != State.IDLE) return@withContext null
        stopRequested = false
        val file = File(context.cacheDir, "dictation.wav")
        try {
            _state.value = State.LISTENING
            RecordingVoiceCollector.yieldMic(MAX_MS + 30_000)
            delay(1_500) // 상시 기록이 마이크를 놓을 시간
            val minBuf = AudioRecord.getMinBufferSize(SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            val rec = AudioRecord(
                MediaRecorder.AudioSource.MIC, SAMPLE_RATE,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuf, 8192),
            )
            if (rec.state != AudioRecord.STATE_INITIALIZED) { rec.release(); return@withContext null }
            val pcm = ByteArrayOutputStream()
            val buf = ByteArray(4096)
            val started = System.currentTimeMillis()
            rec.startRecording()
            while (!stopRequested && System.currentTimeMillis() - started < MAX_MS) {
                val n = rec.read(buf, 0, buf.size)
                if (n > 0) pcm.write(buf, 0, n) else if (n < 0) break
            }
            rec.stop(); rec.release()
            RecordingVoiceCollector.yieldMic(0)
            val bytes = pcm.toByteArray()
            if (bytes.size < SAMPLE_RATE * 2 / 2) return@withContext null // 0.5초 미만
            _state.value = State.TRANSCRIBING
            file.outputStream().use { it.write(wavHeader(bytes.size)); it.write(bytes) }
            if (TranscribeConfig.engine(context) == TranscribeConfig.Engine.SERVER) ServerTranscriber.transcribe(context, file)
            else WhisperEngine.transcribe(context, file) // GEMINI 설정이어도 일기는 외부로 보내지 않는다
        } catch (e: Exception) {
            null
        } finally {
            RecordingVoiceCollector.yieldMic(0)
            file.delete()
            _state.value = State.IDLE
        }
    }

    private fun wavHeader(dataLen: Int): ByteArray {
        fun le(v: Int) = byteArrayOf(v.toByte(), (v shr 8).toByte(), (v shr 16).toByte(), (v shr 24).toByte())
        fun le16(v: Int) = byteArrayOf(v.toByte(), (v shr 8).toByte())
        return "RIFF".toByteArray() + le(36 + dataLen) + "WAVE".toByteArray() + "fmt ".toByteArray() +
            le(16) + le16(1) + le16(1) + le(SAMPLE_RATE) + le(SAMPLE_RATE * 2) + le16(2) + le16(16) +
            "data".toByteArray() + le(dataLen)
    }
}
