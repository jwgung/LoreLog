package com.lorelog.collect

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Build
import com.konovalov.vad.webrtc.VadWebRTC
import com.konovalov.vad.webrtc.config.FrameSize
import com.konovalov.vad.webrtc.config.Mode
import com.konovalov.vad.webrtc.config.SampleRate
import com.lorelog.data.LogRepository
import com.lorelog.collect.PendingVoice
import com.lorelog.speaker.SpeakerId
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * 녹음+클라우드 음성 수집(VAD 기반). AudioRecord 로 원본 PCM 을 받아, WebRTC VAD 로
 * 말이 있을 때만 구간을 모아, 가까운 발화들을 ~15초로 묶어 WAV 로 저장하고 선택한 엔진으로
 * 전사한다(짧은 토막은 오인식·요청 과다라 묶는다). 조용할 땐 소리 크기만
 * 훑어 부하가 거의 없다. 화면이 꺼져도 끊기지 않는다. 전사 후 오디오는 삭제.
 */
class RecordingVoiceCollector(context: Context) {

    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val audio = appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager

    @Volatile private var running = false
    @Volatile private var btEnabled = false
    private var loopJob: Job? = null

    fun start() {
        if (loopJob?.isActive == true) return
        running = true
        enableBtMic()
        loopJob = scope.launch {
            PendingVoice.transcribePending(appContext)
            runVadLoop()
        }
    }

    fun stop() {
        running = false
        loopJob?.cancel()
        disableBtMic()
    }

    @Suppress("MissingPermission")
    private fun runVadLoop() {
        val minBuf = AudioRecord.getMinBufferSize(SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val recorder = try {
            AudioRecord(
                MediaRecorder.AudioSource.MIC, SAMPLE_RATE,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minBuf, FRAME * 8),
            )
        } catch (e: Exception) {
            LogRepository.setStatus("마이크를 열 수 없습니다")
            return
        }
        val vad = VadWebRTC(
            SampleRate.SAMPLE_RATE_16K,
            FrameSize.FRAME_SIZE_320,
            Mode.AGGRESSIVE,
            SPEECH_MS,
            SILENCE_MS,
        )
        try {
            recorder.startRecording()
            val frame = ShortArray(FRAME)
            val preRoll = ArrayDeque<ShortArray>(PREROLL_FRAMES)
            val segment = ArrayList<ShortArray>()
            val batch = ArrayList<ShortArray>()
            var batchSamples = 0
            var silenceRun = 0
            var inSpeech = false

            fun sealSegmentIntoBatch() {
                if (segment.isEmpty()) return
                batch.addAll(segment)
                batchSamples += segment.sumOf { it.size }
                segment.clear()
            }
            fun emitBatch() {
                sealSegmentIntoBatch()
                if (batch.isEmpty()) return
                val b = ArrayList(batch)
                batch.clear(); batchSamples = 0; silenceRun = 0
                flush(b)
            }

            while (running && loopJob?.isActive == true) {
                var read = 0
                while (read < FRAME) {
                    val r = recorder.read(frame, read, FRAME - read)
                    if (r <= 0) break
                    read += r
                }
                if (read < FRAME) continue
                val speech = try { vad.isSpeech(frame) } catch (_: Exception) { false }

                if (speech) {
                    silenceRun = 0
                    if (!inSpeech) {
                        inSpeech = true
                        segment.clear()
                        segment.addAll(preRoll)
                    }
                    segment.add(frame.copyOf())
                    // 한 발화가 너무 길면 조각으로 끊어 배치에 넣는다
                    if (segment.size >= MAX_SEG_FRAMES) {
                        sealSegmentIntoBatch()
                        inSpeech = false
                    }
                    // 배치가 목표 길이(~15초)를 넘으면 바로 내보낸다
                    if (batchSamples + segment.sumOf { it.size } >= BATCH_TARGET_SAMPLES) {
                        emitBatch()
                    }
                } else {
                    if (inSpeech) {
                        inSpeech = false
                        sealSegmentIntoBatch()
                        if (batchSamples >= BATCH_TARGET_SAMPLES) emitBatch()
                    }
                    preRoll.addLast(frame.copyOf())
                    while (preRoll.size > PREROLL_FRAMES) preRoll.removeFirst()
                    // 말이 멈춘 뒤 일정 시간(대화 끊김) 지나면 배치 마감
                    if (batch.isNotEmpty()) {
                        silenceRun++
                        if (silenceRun >= BATCH_IDLE_FRAMES) emitBatch()
                    }
                }
            }
            emitBatch()
        } catch (e: Exception) {
            // 마이크 분실 등: 조용히 종료, 서비스가 다시 시작한다.
        } finally {
            try { recorder.stop() } catch (_: Exception) {}
            try { recorder.release() } catch (_: Exception) {}
            try { vad.close() } catch (_: Exception) {}
        }
    }

    /** 말 구간 PCM 을 WAV 로 저장하고, 화자 라벨을 뽑아 전사·저장 큐에 넣는다. */
    private fun flush(frames: List<ShortArray>) {
        if (frames.isEmpty()) return
        val total = frames.sumOf { it.size }
        if (total < MIN_SEG_SAMPLES) return
        val pcm = ShortArray(total)
        var off = 0
        for (f in frames) { f.copyInto(pcm, off); off += f.size }
        val speaker = if (SpeakerId.isConfigured(appContext) && SpeakerId.isEnrolled(appContext)) {
            SpeakerId.labelFor(appContext, pcm)
        } else {
            null
        }
        val file = PendingVoice.fileFor(appContext, speaker)
        if (!writeWav(file, pcm)) return
        // 즉시 1회 전사 시도. 실패(키 없음·오프라인)하면 파일이 남아 워커가 나중에 전사한다.
        scope.launch { PendingVoice.transcribeFile(appContext, file) }
    }

    private fun writeWav(file: File, pcm: ShortArray): Boolean = try {
        val dataBytes = pcm.size * 2
        RandomAccessFile(file, "rw").use { f ->
            f.write("RIFF".toByteArray())
            f.write(intLe(36 + dataBytes))
            f.write("WAVE".toByteArray())
            f.write("fmt ".toByteArray())
            f.write(intLe(16))
            f.write(shortLe(1))
            f.write(shortLe(1))
            f.write(intLe(SAMPLE_RATE))
            f.write(intLe(SAMPLE_RATE * 2))
            f.write(shortLe(2))
            f.write(shortLe(16))
            f.write("data".toByteArray())
            f.write(intLe(dataBytes))
            val bb = ByteBuffer.allocate(dataBytes).order(ByteOrder.LITTLE_ENDIAN)
            for (s in pcm) bb.putShort(s)
            f.write(bb.array())
        }
        true
    } catch (e: Exception) {
        file.delete()
        false
    }

    private fun intLe(v: Int) = byteArrayOf(
        (v and 0xff).toByte(), ((v shr 8) and 0xff).toByte(),
        ((v shr 16) and 0xff).toByte(), ((v shr 24) and 0xff).toByte(),
    )
    private fun shortLe(v: Int) = byteArrayOf((v and 0xff).toByte(), ((v shr 8) and 0xff).toByte())

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

    companion object {
        private const val SAMPLE_RATE = 16000
        private const val FRAME = 320
        private const val SPEECH_MS = 100
        private const val SILENCE_MS = 600
        private const val PREROLL_FRAMES = 25
        private const val MAX_SEG_FRAMES = 3000
        private const val MIN_SEG_SAMPLES = 8000
        // 가까운 발화들을 ~15초로 묶어 한 번에 전사(요청 수↓, whisper 정확도↑)
        private const val BATCH_TARGET_SAMPLES = 240000 // 15초 @16kHz
        private const val BATCH_IDLE_FRAMES = 150 // 침묵 ~3초면 배치 마감
    }
}
