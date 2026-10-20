package com.lorelog.collect

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import com.lorelog.audio.VoicePlayer
import android.media.MediaRecorder
import android.os.Build
import android.util.Log
import com.konovalov.vad.webrtc.VadWebRTC
import com.konovalov.vad.webrtc.config.FrameSize
import com.konovalov.vad.webrtc.config.Mode
import com.konovalov.vad.webrtc.config.SampleRate
import com.lorelog.data.LogRepository
import com.lorelog.collect.PendingVoice
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
 * 말이 있을 때만 구간을 모아, 가까운 발화들을 ~2분으로 묶어 WAV 로 저장하고 선택한 엔진으로
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
        // 밀린 전사는 별도 코루틴. 같은 코루틴에 두면 파일이 많거나 한도 대기 중일 때 녹음이 그만큼 늦게 시작된다.
        scope.launch { PendingVoice.transcribePending(appContext) }
        loopJob = scope.launch { runVadLoop() }
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
            Log.w(TAG, "AudioRecord create failed: ${e.javaClass.simpleName}: ${e.message}")
            LogRepository.setStatus("마이크를 열 수 없습니다")
            return
        }
        if (recorder.state != AudioRecord.STATE_INITIALIZED) {
            Log.w(TAG, "AudioRecord not initialized (state=${recorder.state})")
            LogRepository.setStatus("마이크 초기화 실패 — 다른 앱이 쓰는 중일 수 있음")
            try { recorder.release() } catch (_: Exception) {}
            return
        }
        // 에코 캔슬러: 이 기기가 재생 중인 소리(타임라인 "듣기")를 마이크 입력에서 뺀다. 지원 안 하면 null.
        val aec = if (AcousticEchoCanceler.isAvailable()) {
            runCatching { AcousticEchoCanceler.create(recorder.audioSessionId)?.apply { enabled = true } }.getOrNull()
        } else null
        // 잡음 억제·자동 음량: 떨어진 사람 말을 키우고 배경 소음을 줄인다. 설정에서 끌 수 있다.
        val clean = AudioPrefs.micClean(appContext)
        val ns = if (clean && NoiseSuppressor.isAvailable()) {
            runCatching { NoiseSuppressor.create(recorder.audioSessionId)?.apply { enabled = true } }.getOrNull()
        } else null
        val agc = if (clean && AutomaticGainControl.isAvailable()) {
            runCatching { AutomaticGainControl.create(recorder.audioSessionId)?.apply { enabled = true } }.getOrNull()
        } else null
        Log.i(TAG, "AEC ${if (aec?.enabled == true) "on" else "unavailable"}, NS ${if (ns?.enabled == true) "on" else "off"}, AGC ${if (agc?.enabled == true) "on" else "off"}")
        val vad = VadWebRTC(
            SampleRate.SAMPLE_RATE_16K,
            FrameSize.FRAME_SIZE_320,
            Mode.AGGRESSIVE,
            SPEECH_MS,
            SILENCE_MS,
        )
        try {
            recorder.startRecording()
            if (recorder.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                Log.w(TAG, "startRecording did not enter RECORDING state")
                LogRepository.setStatus("마이크 녹음 시작 실패")
                return
            }
            Log.i(TAG, "VAD loop started (bt=$btEnabled)")
            var frames = 0L
            var silencedReported = false
            val frame = ShortArray(FRAME)
            val preRoll = ArrayDeque<ShortArray>(PREROLL_FRAMES)
            val segment = ArrayList<ShortArray>()
            val batch = ArrayList<ShortArray>()
            var batchSamples = 0
            var silenceRun = 0
            var inSpeech = false
            var batchStartMs = 0L // 이 배치의 첫 말이 시작된 시각(기록 시각으로 쓴다)
            var segmentPlayback = false // 이 발화 중에 앱이 음성을 재생하고 있었나
            var batchPlayback = false // 이 배치가 재생 중 캡처 묶음인가(라벨 "재생 중")

            fun flushBatchOnly() {
                if (batch.isEmpty()) return
                val b = ArrayList(batch)
                val startMs = batchStartMs
                val playback = batchPlayback
                batch.clear(); batchSamples = 0; silenceRun = 0; batchStartMs = 0L; batchPlayback = false
                flush(b, startMs, playback)
            }
            fun sealSegmentIntoBatch() {
                if (segment.isEmpty()) return
                // 재생 중/아님 발화는 섞지 않는다. 라벨이 배치 단위라 경계에서 끊는다.
                if (batch.isNotEmpty() && batchPlayback != segmentPlayback) flushBatchOnly()
                if (batch.isEmpty()) {
                    // 프리롤(0.5초)만큼 앞당겨 실제 첫마디 시각에 맞춘다
                    batchStartMs = System.currentTimeMillis() - segment.size * FRAME_MS
                    batchPlayback = segmentPlayback
                }
                batch.addAll(segment)
                batchSamples += segment.sumOf { it.size }
                segment.clear()
                segmentPlayback = false
            }
            fun emitBatch() {
                sealSegmentIntoBatch()
                flushBatchOnly()
            }

            while (running && loopJob?.isActive == true) {
                var read = 0
                while (read < FRAME) {
                    val r = recorder.read(frame, read, FRAME - read)
                    if (r <= 0) break
                    read += r
                }
                if (read < FRAME) continue
                frames++
                // 5초마다: OS 가 이 녹음을 무음 처리 중인지(다른 앱이 마이크 점유·백그라운드 제한) 확인
                if (frames % SILENCE_CHECK_FRAMES == 0L) {
                    val silenced = try { recorder.activeRecordingConfiguration?.isClientSilenced == true } catch (_: Exception) { false }
                    if (silenced && !silencedReported) {
                        silencedReported = true
                        Log.w(TAG, "mic is SILENCED by OS (another app recording, or background start)")
                        LogRepository.setStatus("마이크가 무음 처리됨 — 다른 앱이 녹음 중이거나 백그라운드 제한. 기록을 껐다 켜 보세요.")
                    } else if (!silenced && silencedReported) {
                        silencedReported = false
                        Log.i(TAG, "mic unsilenced")
                        LogRepository.setStatus("마이크 입력 복구됨")
                    }
                }
                val speech = try { vad.isSpeech(frame) } catch (_: Exception) { false }

                if (speech) {
                    silenceRun = 0
                    if (VoicePlayer.playing.value != null) segmentPlayback = true
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
                    // 배치가 목표 길이(~2분)를 넘으면 바로 내보낸다
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
            Log.i(TAG, "VAD loop ended normally after $frames frames")
        } catch (e: Exception) {
            // 마이크 분실 등. 루프가 죽으면 다음 start()/reapply 까지 음성은 안 잡힌다.
            Log.w(TAG, "VAD loop died: ${e.javaClass.simpleName}: ${e.message}")
            LogRepository.setStatus("음성 녹음 루프 중단(${e.javaClass.simpleName}) — 기록을 껐다 켜 주세요")
        } finally {
            try { aec?.release() } catch (_: Exception) {}
            try { ns?.release() } catch (_: Exception) {}
            try { agc?.release() } catch (_: Exception) {}
            try { recorder.stop() } catch (_: Exception) {}
            try { recorder.release() } catch (_: Exception) {}
            try { vad.close() } catch (_: Exception) {}
        }
    }

    /**
     * 묶은 말 구간 PCM 을 WAV 로 저장하고 전사·저장 큐에 넣는다. startMs = 첫마디 시각(파일명에 박혀 기록 시각이 된다).
     * playback = 앱이 음성을 재생하는 동안 잡힌 묶음. "재생 중" 라벨이 붙고, 전사 결과가 재생한 내용과 같으면 버린다.
     */
    private fun flush(frames: List<ShortArray>, startMs: Long, playback: Boolean) {
        if (frames.isEmpty()) return
        val total = frames.sumOf { it.size }
        if (total < MIN_SEG_SAMPLES) {
            Log.d(TAG, "drop short batch: ${total / 16} ms"); return
        }
        Log.i(TAG, "flush batch: ${total / 16} ms")
        val pcm = ShortArray(total)
        var off = 0
        for (f in frames) { f.copyInto(pcm, off); off += f.size }
        val label = if (playback) PendingVoice.LABEL_PLAYBACK else null
        val file = PendingVoice.fileFor(appContext, label, startMs.takeIf { it > 0 } ?: System.currentTimeMillis())
        if (!writeWav(file, pcm)) { Log.w(TAG, "writeWav failed: $file"); return }
        PendingVoice.refresh(appContext)
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
        private const val TAG = "RecordingVoice"
        private const val SAMPLE_RATE = 16000
        private const val SILENCE_CHECK_FRAMES = 250L // 20ms × 250 = 5초
        private const val FRAME = 320
        private const val FRAME_MS = 20L
        private const val SPEECH_MS = 100
        private const val SILENCE_MS = 600
        private const val PREROLL_FRAMES = 25
        private const val MAX_SEG_FRAMES = 3000
        private const val MIN_SEG_SAMPLES = 8000
        // 가까운 발화들을 ~2분으로 묶어 한 번에 전사. 요청 수가 곧 비용이다: Gemini 무료 티어는
        // 모델당 하루 500회라 15초 배치(시간당 240회)로는 두 시간 대화에 바닥난다. 2분 배치면 시간당 30회.
        // 전사 결과가 최대 2.5분 늦게 뜨는 대신 하루 종일 버틴다. (whisper 는 긴 배치가 정확도도 더 좋다)
        private const val BATCH_TARGET_SAMPLES = 16000 * 120 // 120초 @16kHz (3.8MB PCM)
        private const val BATCH_IDLE_FRAMES = 1500 // 침묵 ~30초(대화 끊김)면 배치 마감
    }
}
