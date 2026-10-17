package com.lifelog.collect

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.MediaRecorder
import android.os.Build
import com.lifelog.data.LogRepository
import com.lifelog.net.GroqTranscriber
import com.lifelog.net.SttConfig
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.io.File

/**
 * 녹음+클라우드 음성 수집. 마이크를 조각(기본 60초)으로 녹음하고, 소리가 거의 없는 조각은
 * 건너뛴다(간단 VAD). 소리가 있는 조각만 Groq Whisper 로 전사해 저장하고 오디오는 지운다.
 * 구글 실시간 인식과 달리 화면이 꺼져도 끊기지 않는다.
 */
class RecordingVoiceCollector(context: Context) {

    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val dir = File(appContext.cacheDir, "audio").apply { mkdirs() }
    private val audio = appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    @Volatile private var btEnabled = false

    @Volatile private var running = false
    private var recorder: MediaRecorder? = null
    private var loopJob: Job? = null

    fun start() {
        if (loopJob?.isActive == true) return
        if (!SttConfig.isConfigured(appContext)) {
            LogRepository.setStatus("녹음+클라우드 음성엔 전사 키가 필요합니다")
            return
        }
        running = true
        enableBtMic()
        loopJob = scope.launch {
            sweepLeftovers()
            while (running && isActive) {
                val file = File(dir, "chunk_${System.currentTimeMillis()}.m4a")
                val maxAmp = recordChunk(file)
                when {
                    !running -> file.delete()
                    maxAmp > VAD_THRESHOLD -> transcribe(file)
                    else -> file.delete()
                }
            }
        }
    }

    fun stop() {
        running = false
        loopJob?.cancel()
        releaseRecorder()
        disableBtMic()
    }

    private suspend fun recordChunk(file: File): Int {
        val rec = newRecorder(file)
        recorder = rec
        try {
            rec.prepare()
            rec.start()
        } catch (e: Exception) {
            releaseRecorder()
            file.delete()
            delay(2000)
            return 0
        }
        var maxAmp = 0
        var elapsed = 0L
        while (running && elapsed < CHUNK_MS) {
            delay(POLL_MS)
            elapsed += POLL_MS
            try { maxAmp = maxOf(maxAmp, rec.maxAmplitude) } catch (_: Exception) {}
        }
        releaseRecorder()
        return maxAmp
    }

    /** 블루투스 이어폰 마이크로 입력을 돌린다(설정 시). 이어폰이 없으면 폰 마이크로 둔다. */
    private fun enableBtMic() {
        if (!AudioPrefs.btMic(appContext)) return
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                val dev = audio.availableCommunicationDevices
                    .firstOrNull { it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO }
                if (dev != null) { audio.setCommunicationDevice(dev); btEnabled = true }
            } else {
                audio.mode = AudioManager.MODE_IN_COMMUNICATION
                @Suppress("DEPRECATION") audio.startBluetoothSco()
                @Suppress("DEPRECATION") audio.isBluetoothScoOn = true
                btEnabled = true
            }
        } catch (_: Exception) {}
    }

    private fun disableBtMic() {
        if (!btEnabled) return
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                audio.clearCommunicationDevice()
            } else {
                @Suppress("DEPRECATION") audio.stopBluetoothSco()
                @Suppress("DEPRECATION") audio.isBluetoothScoOn = false
                audio.mode = AudioManager.MODE_NORMAL
            }
        } catch (_: Exception) {}
        btEnabled = false
    }

    private fun newRecorder(file: File): MediaRecorder {
        val rec = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) MediaRecorder(appContext) else @Suppress("DEPRECATION") MediaRecorder()
        rec.setAudioSource(MediaRecorder.AudioSource.MIC)
        rec.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
        rec.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
        rec.setAudioSamplingRate(16000)
        rec.setAudioEncodingBitRate(24000)
        rec.setOutputFile(file.absolutePath)
        return rec
    }

    private fun releaseRecorder() {
        recorder?.let {
            try { it.stop() } catch (_: Exception) {}
            try { it.release() } catch (_: Exception) {}
        }
        recorder = null
    }

    /** 소리 있는 조각을 전사해 저장하고 오디오 삭제. 실패하면 파일을 남겨 다음에 다시 시도한다. */
    private fun transcribe(file: File) {
        scope.launch {
            val text = GroqTranscriber.transcribe(appContext, file)
            if (text != null) {
                LogRepository.addVoice(text)
                file.delete()
            }
        }
    }

    private fun sweepLeftovers() {
        dir.listFiles()?.forEach { f -> if (f.name.endsWith(".m4a")) transcribe(f) }
    }

    companion object {
        private const val CHUNK_MS = 60_000L
        private const val POLL_MS = 1000L
        private const val VAD_THRESHOLD = 1500
    }
}
