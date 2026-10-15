package com.lifelog.data

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
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

    /** 아직 확정되지 않은 실시간 부분 인식 텍스트(저장하지 않음). */
    private val _partial = MutableStateFlow("")
    val partial: StateFlow<String> = _partial.asStateFlow()

    /** 엔진 상태 안내(예: 음성 인식 불가). 비어 있으면 정상. */
    private val _status = MutableStateFlow("")
    val status: StateFlow<String> = _status.asStateFlow()

    fun init(context: Context) {
        store = LogStore(context.applicationContext)
        scope.launch { _voiceSegments.value = store.recentVoice(KEEP) }
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

    fun setPartial(text: String) { _partial.value = text }
    fun setStatus(text: String) { _status.value = text }
}
