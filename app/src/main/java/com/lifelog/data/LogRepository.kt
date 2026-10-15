package com.lifelog.data

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * 수집 결과의 단일 출처. 화면은 여기 StateFlow 를 구독한다.
 * DB 쓰기는 IO 스레드에서, 화면용 목록은 메모리 StateFlow 로 즉시 갱신한다.
 */
object LogRepository {
    private const val KEEP = 500

    private lateinit var store: LogStore
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val _voiceSegments = MutableStateFlow<List<VoiceSegment>>(emptyList())
    val voiceSegments: StateFlow<List<VoiceSegment>> = _voiceSegments.asStateFlow()

    private val _appEvents = MutableStateFlow<List<AppEvent>>(emptyList())
    val appEvents: StateFlow<List<AppEvent>> = _appEvents.asStateFlow()

    private val _screenEvents = MutableStateFlow<List<ScreenContentEvent>>(emptyList())
    val screenEvents: StateFlow<List<ScreenContentEvent>> = _screenEvents.asStateFlow()

    /** 음성·앱·화면 내용을 시간순(최신 먼저)으로 합친 타임라인. */
    val timeline: StateFlow<List<TimelineItem>> =
        combine(_voiceSegments, _appEvents, _screenEvents) { voice, apps, screens ->
            (voice.map { TimelineItem.Voice(it) } +
                apps.map { TimelineItem.App(it) } +
                screens.map { TimelineItem.Screen(it) })
                .sortedByDescending { it.timestampMs }
                .take(KEEP)
        }.stateIn(scope, SharingStarted.Eagerly, emptyList())

    /** 아직 확정되지 않은 실시간 부분 인식 텍스트(저장하지 않음). */
    private val _partial = MutableStateFlow("")
    val partial: StateFlow<String> = _partial.asStateFlow()

    /** 엔진 상태 안내(예: 음성 인식 불가). 비어 있으면 정상. */
    private val _status = MutableStateFlow("")
    val status: StateFlow<String> = _status.asStateFlow()

    fun init(context: Context) {
        store = LogStore(context.applicationContext)
        scope.launch {
            _voiceSegments.value = store.recentVoice(KEEP)
            _appEvents.value = store.recentApp(KEEP)
            _screenEvents.value = store.recentScreen(KEEP)
        }
    }

    fun addVoice(text: String, speaker: String? = null) {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return
        scope.launch {
            val ts = System.currentTimeMillis()
            val id = store.insertVoice(ts, trimmed, speaker)
            val seg = VoiceSegment(id, ts, trimmed, speaker)
            _voiceSegments.update { (listOf(seg) + it).take(KEEP) }
        }
    }

    fun addApp(packageName: String, label: String, timestampMs: Long) {
        scope.launch {
            val id = store.insertApp(timestampMs, packageName, label)
            val event = AppEvent(id, timestampMs, packageName, label)
            _appEvents.update { (listOf(event) + it).take(KEEP) }
        }
    }

    fun addScreen(packageName: String, label: String, text: String, timestampMs: Long) {
        scope.launch {
            val id = store.insertScreen(timestampMs, packageName, label, text)
            val event = ScreenContentEvent(id, timestampMs, packageName, label, text)
            _screenEvents.update { (listOf(event) + it).take(KEEP) }
        }
    }

    fun setPartial(text: String) { _partial.value = text }
    fun setStatus(text: String) { _status.value = text }
}
