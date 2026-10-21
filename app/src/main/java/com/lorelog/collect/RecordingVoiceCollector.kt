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
import com.konovalov.vad.silero.VadSilero
import com.konovalov.vad.silero.config.FrameSize
import com.konovalov.vad.silero.config.Mode
import com.konovalov.vad.silero.config.SampleRate
import com.lorelog.data.LogRepository
import com.lorelog.collect.PendingVoice
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * 녹음+클라우드 음성 수집(VAD 기반). AudioRecord 로 원본 PCM 을 받아, Silero VAD(신경망, ONNX)로
 * 사람 말이 있을 때만 구간을 모아, 가까운 발화들을 ~4분으로 묶어 WAV 로 저장하고 선택한 엔진으로
 * 전사한다(짧은 토막은 오인식·요청 과다라 묶는다). WebRTC VAD 는 키보드·식기·에어컨 소리에 자주
 * 깨어나 잡음 배치로 한도를 태워서 교체했다. 32ms 프레임당 추론 1ms 안쪽. 화면이 꺼져도 끊기지 않는다.
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

    /**
     * 마이크를 다른 앱에 양보해야 하나. 안드로이드는 마이크를 한 앱만 쓰게 하므로(다른 쪽은 무음),
     * 우리가 들고 있으면 음성 녹음·카메라 동영상 앱이 "다른 앱이 마이크 사용 중"으로 실패한다.
     * 1) 수동 양보(알림·화면 버튼, 기본 10분) 2) 녹음·카메라 앱이 맨 앞에 있음(사용 정보 접근 권한 필요)
     * 3) 통화 중(일반·VoIP) 4) 다른 앱이 실제로 녹음 중(AudioManager 활성 녹음 목록). 하나라도 맞으면 마이크를 놓고 기다린다.
     */
    private fun yieldReason(mySession: Int?): String? {
        val now = System.currentTimeMillis()
        if (_yieldedUntil.value > now) return "수동 양보"
        val fg = LogRepository.foregroundApp.value
        if (fg != null && fg in MIC_APPS) return "녹음·카메라 앱 사용 중"
        // 통화 중: 전화 앱이 특권 경로로 녹음하고 우리 입력은 어차피 OS 가 무음 처리한다. 들고 있어 봐야
        // 전화 앱의 통화 녹음과 부딪힐 수만 있으니 놓는다. MODE_IN_CALL = 일반 통화, MODE_IN_COMMUNICATION = VoIP
        // (카카오톡 통화 등). 단 S 미만에서 블루투스 마이크를 켜면 우리가 IN_COMMUNICATION 을 쓰므로 그땐 제외.
        val mode = try { audio.mode } catch (_: Exception) { AudioManager.MODE_NORMAL }
        if (mode == AudioManager.MODE_IN_CALL) return "통화 중"
        if (mode == AudioManager.MODE_IN_COMMUNICATION && (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S || !btEnabled)) return "통화 중(VoIP)"
        val others = try {
            audio.activeRecordingConfigurations.filter { mySession == null || it.clientAudioSessionId != mySession }
        } catch (_: Exception) { emptyList() }
        if (others.isNotEmpty()) {
            // 음성 비서 호출어 감지처럼 상시 녹음하는 시스템 구성요소가 잡히면 영원히 양보하게 된다.
            // 다른 앱의 "녹음"이 30분 넘게 끊기지 않으면 그건 녹음이 아니라 상시 감지로 보고 무시한다.
            if (othersSince == 0L) othersSince = now
            if (now - othersSince < OTHERS_MAX_MS) return "다른 앱이 녹음 중"
            if (!othersIgnoredLogged) { othersIgnoredLogged = true; Log.w(TAG, "another recorder active >30min; ignoring (always-on detector?)") }
            return null
        }
        othersSince = 0L; othersIgnoredLogged = false
        return null
    }
    private var othersSince = 0L
    private var othersIgnoredLogged = false

    /** 양보 조건이면 마이크를 놓고, 풀리면 다시 연다. 녹음 세션 하나가 끝날 때마다(양보·오류) 여기로 돌아온다. */
    private suspend fun runVadLoop() {
        var yielding = false
        while (running && loopJob?.isActive == true) {
            val reason = yieldReason(null)
            if (reason != null) {
                if (!yielding) {
                    yielding = true
                    Log.i(TAG, "mic yielded: $reason")
                    LogRepository.setStatus("마이크 양보 중($reason) — 끝나면 자동 재개")
                }
                delay(YIELD_POLL_MS)
                continue
            }
            if (yielding) {
                yielding = false
                Log.i(TAG, "mic resumed")
                LogRepository.setStatus("")
            }
            val ok = recordSession()
            if (!ok) return // 열기 실패·치명적 오류: 상태 줄에 이미 알렸다. 다음 start()/reapply 까지 쉰다
        }
    }

    /** 마이크를 열어 양보 조건이 생기거나 stop() 될 때까지 VAD 루프를 돈다. 정상(양보)이면 true, 못 열었거나 죽었으면 false. */
    @Suppress("MissingPermission")
    private fun recordSession(): Boolean {
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
            return false
        }
        if (recorder.state != AudioRecord.STATE_INITIALIZED) {
            Log.w(TAG, "AudioRecord not initialized (state=${recorder.state})")
            LogRepository.setStatus("마이크 초기화 실패 — 다른 앱이 쓰는 중일 수 있음")
            try { recorder.release() } catch (_: Exception) {}
            return false
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
        val vad = try {
            VadSilero(appContext, SampleRate.SAMPLE_RATE_16K, FrameSize.FRAME_SIZE_512, Mode.NORMAL, SPEECH_MS, SILENCE_MS)
        } catch (e: Exception) {
            Log.w(TAG, "VadSilero init failed: ${e.javaClass.simpleName}: ${e.message}")
            LogRepository.setStatus("음성 감지 모델 초기화 실패(${e.javaClass.simpleName}) — 기록을 껐다 켜 주세요")
            try { recorder.release() } catch (_: Exception) {}
            return false
        }
        try {
            recorder.startRecording()
            if (recorder.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                Log.w(TAG, "startRecording did not enter RECORDING state")
                LogRepository.setStatus("마이크 녹음 시작 실패")
                return false
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
                    batchStartMs = System.currentTimeMillis() - segment.size.toLong() * FRAME * 1000 / SAMPLE_RATE
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

            val mySession = recorder.audioSessionId
            while (running && loopJob?.isActive == true) {
                // 1초마다 양보 조건 확인. 맞으면 모은 것을 내보내고 마이크를 놓는다(바깥 루프가 기다렸다 다시 연다)
                if (frames % YIELD_CHECK_FRAMES == 0L && yieldReason(mySession) != null) break
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
                    // 배치가 목표 길이(~4분)를 넘으면 바로 내보낸다
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
            Log.i(TAG, "record session ended after $frames frames")
            return true
        } catch (e: Exception) {
            // 마이크 분실 등. 루프가 죽으면 다음 start()/reapply 까지 음성은 안 잡힌다.
            Log.w(TAG, "VAD loop died: ${e.javaClass.simpleName}: ${e.message}")
            LogRepository.setStatus("음성 녹음 루프 중단(${e.javaClass.simpleName}) — 기록을 껐다 켜 주세요")
            return false
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
        private const val YIELD_POLL_MS = 1_000L
        private const val OTHERS_MAX_MS = 30 * 60 * 1000L
        private const val YIELD_CHECK_FRAMES = 31L // 32ms × 31 ≈ 1초
        const val YIELD_DEFAULT_MS = 10 * 60 * 1000L

        /** 맨 앞에 있으면 마이크를 양보할 앱(음성 녹음기·카메라). 이 앱들은 마이크를 못 잡으면 녹음을 시작하지 못한다. */
        private val MIC_APPS = setOf(
            "com.sec.android.app.voicenote", // 삼성 음성 녹음
            "com.samsung.android.app.voicenote",
            "com.google.android.apps.recorder", // 구글 레코더
            "com.android.soundrecorder", // AOSP·샤오미
            "com.coloros.soundrecorder", "com.oplus.soundrecorder", // 오포·원플러스
            "com.sec.android.app.camera", // 삼성 카메라(동영상)
            "com.google.android.GoogleCamera", "com.android.camera", "com.android.camera2",
            "com.naver.clovanote", // 클로바노트
        )

        /** 수동 양보가 끝나는 시각(epoch ms). 0 이면 양보 아님. 알림·화면 버튼이 바꾸고 녹음 루프가 읽는다. */
        private val _yieldedUntil = MutableStateFlow(0L)
        val yieldedUntil: StateFlow<Long> = _yieldedUntil.asStateFlow()

        /** [ms] 동안 마이크를 놓는다. 0 이하면 양보 해제. */
        fun yieldMic(ms: Long = YIELD_DEFAULT_MS) {
            _yieldedUntil.value = if (ms > 0) System.currentTimeMillis() + ms else 0L
        }
        // Silero 는 16kHz 에서 512 샘플(32ms) 프레임. 아래 "프레임 수" 상수는 전부 32ms 기준.
        private const val FRAME = 512
        private const val SILENCE_CHECK_FRAMES = 156L // 32ms × 156 ≈ 5초
        private const val SPEECH_MS = 128 // 말로 치기까지 최소 지속(프레임 4개)
        private const val SILENCE_MS = 640 // 말이 끝났다고 보기까지 침묵(프레임 20개)
        private const val PREROLL_FRAMES = 16 // 첫마디 앞 ~0.5초
        private const val MAX_SEG_FRAMES = 1875 // 한 발화 최대 60초
        // 배치 안 말소리 합이 이보다 짧으면 버린다(3초). 사람 말 한마디는 보통 2~3초 넘고, 잡음에 잠깐
        // 깨어난 0.5~1초짜리 조각은 전사해도 빈 문장·환각이라 요청만 낭비한다.
        private const val MIN_SEG_SAMPLES = 16000 * 3
        // 가까운 발화들을 ~4분으로 묶어 한 번에 전사. 요청 수가 곧 비용이다: Gemini 무료 티어는
        // 모델당 하루 500회. 4분 배치면 연속 대화도 시간당 15회, 회의 6시간에 90회.
        // 전사 결과가 최대 4분 늦게 뜨는 대신 하루 종일 버틴다. (whisper 는 긴 배치가 정확도도 더 좋다)
        private const val BATCH_TARGET_SAMPLES = 16000 * 240 // 240초 @16kHz (7.7MB PCM, 인라인 한도 20MB 안)
        private const val BATCH_IDLE_FRAMES = 5625 // 침묵 ~3분(대화 끊김)이면 배치 마감. 잠깐 조용한 틈엔 쪼개지 않는다
    }
}
