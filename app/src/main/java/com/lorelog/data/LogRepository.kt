package com.lorelog.data

import android.content.Context
import com.lorelog.net.EndpointConfig
import com.lorelog.net.HttpSink
import com.lorelog.net.LlmConfig
import com.lorelog.summary.CloudSummarizer
import com.lorelog.summary.GeminiNanoSummarizer
import com.lorelog.summary.SummaryPrefs
import com.lorelog.summary.Summarizer
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
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * 수집 결과의 단일 출처. 화면은 여기 StateFlow 를 구독한다.
 * DB 쓰기는 IO 스레드에서, 화면용 목록은 메모리 StateFlow 로 즉시 갱신한다.
 */
object LogRepository {
    private const val KEEP = 500
    private const val DIGEST_MAX_CHARS = 8000
    private const val CONV_GAP_MS = 5 * 60 * 1000L

    private lateinit var store: LogStore
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val _voiceSegments = MutableStateFlow<List<VoiceSegment>>(emptyList())
    val voiceSegments: StateFlow<List<VoiceSegment>> = _voiceSegments.asStateFlow()

    private val _appEvents = MutableStateFlow<List<AppEvent>>(emptyList())
    val appEvents: StateFlow<List<AppEvent>> = _appEvents.asStateFlow()

    private val _screenEvents = MutableStateFlow<List<ScreenContentEvent>>(emptyList())
    val screenEvents: StateFlow<List<ScreenContentEvent>> = _screenEvents.asStateFlow()

    private val _locations = MutableStateFlow<List<LocationPoint>>(emptyList())
    val locations: StateFlow<List<LocationPoint>> = _locations.asStateFlow()

    private val _notifications = MutableStateFlow<List<NotificationEvent>>(emptyList())
    val notifications: StateFlow<List<NotificationEvent>> = _notifications.asStateFlow()

    private val _summaries = MutableStateFlow<List<Summary>>(emptyList())
    val summaries: StateFlow<List<Summary>> = _summaries.asStateFlow()

    private val _commEvents = MutableStateFlow<List<CommEvent>>(emptyList())
    val commEvents: StateFlow<List<CommEvent>> = _commEvents.asStateFlow()

    private val _calendarEvents = MutableStateFlow<List<CalendarEntry>>(emptyList())
    val calendarEvents: StateFlow<List<CalendarEntry>> = _calendarEvents.asStateFlow()

    private val _conversations = MutableStateFlow<List<Conversation>>(emptyList())
    val conversations: StateFlow<List<Conversation>> = _conversations.asStateFlow()

    /** 모든 수집 종류를 시간순(최신 먼저)으로 합친 타임라인. */
    val timeline: StateFlow<List<TimelineItem>> =
        combine(
            combine(_voiceSegments, _appEvents, _screenEvents, _locations, _notifications) {
                voice, apps, screens, locs, notifs ->
                voice.map { TimelineItem.Voice(it) } +
                    apps.map { TimelineItem.App(it) } +
                    screens.map { TimelineItem.Screen(it) } +
                    locs.map { TimelineItem.Location(it) } +
                    notifs.map { TimelineItem.Notification(it) }
            },
            _commEvents,
            _calendarEvents,
        ) { base, comms, cals ->
            (base + comms.map { TimelineItem.Comm(it) } + cals.map { TimelineItem.Calendar(it) })
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
        store = LogStore(context.applicationContext, DbKey.get(context))
        scope.launch {
            _voiceSegments.value = store.recentVoice(KEEP)
            _appEvents.value = store.recentApp(KEEP)
            _screenEvents.value = store.recentScreen(KEEP)
            _locations.value = store.recentLocation(KEEP)
            _notifications.value = store.recentNotification(KEEP)
            _summaries.value = store.recentSummary(KEEP)
            _commEvents.value = store.recentComm(KEEP)
            _calendarEvents.value = store.recentCalendar(KEEP)
            _conversations.value = store.recentConversation(KEEP)
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

    fun addLocation(lat: Double, lon: Double, accuracyM: Float, timestampMs: Long) {
        scope.launch {
            val id = store.insertLocation(timestampMs, lat, lon, accuracyM)
            val point = LocationPoint(id, timestampMs, lat, lon, accuracyM)
            _locations.update { (listOf(point) + it).take(KEEP) }
        }
    }

    fun addNotification(packageName: String, label: String, title: String, text: String, timestampMs: Long) {
        scope.launch {
            val id = store.insertNotification(timestampMs, packageName, label, title, text)
            val event = NotificationEvent(id, timestampMs, packageName, label, title, text)
            _notifications.update { (listOf(event) + it).take(KEEP) }
        }
    }

    /** 모든 기록을 JSON 파일로 내보내 반환한다(공유용, 캐시 폴더). */
    suspend fun exportToFile(context: Context): File = withContext(Dispatchers.IO) {
        val json = store.exportJson(System.currentTimeMillis(), includeSummary = SummaryPrefs.enabled(context))
        val dir = File(context.cacheDir, "exports").apply { mkdirs() }
        val file = File(dir, "lifelog_${System.currentTimeMillis()}.json")
        file.writeText(json)
        file
    }

    /**
     * 증분 내보내기. 지난 내보내기 이후 새 기록만 NDJSON 으로 앱 전용 외부 폴더에 append 한다.
     * (Android/data/<pkg>/files/exports/lifelog-<날짜>.ndjson). 새로 쓴 줄 수를 반환.
     */
    suspend fun exportIncremental(context: Context): Int = withContext(Dispatchers.IO) {
        val prefs = context.getSharedPreferences("export_state", Context.MODE_PRIVATE)
        val lastIds = store.tables().associateWith { prefs.getLong("last_$it", 0L) }
        val result = store.newRows(lastIds, includeSummary = SummaryPrefs.enabled(context))
        if (result.lines.isEmpty()) return@withContext 0
        val dir = File(context.getExternalFilesDir(null), "exports").apply { mkdirs() }
        val date = SimpleDateFormat("yyyy-MM-dd", Locale.US).format(Date())
        File(dir, "lifelog-$date.ndjson").appendText(result.lines.joinToString("\n", postfix = "\n"))
        prefs.edit().apply {
            result.newLastIds.forEach { (table, id) -> putLong("last_$table", id) }
        }.apply()
        result.lines.size
    }

    /**
     * 최근 구간([periodMs]) 기록을 묶어 요약 엔진으로 요약하고, 성공하면 저장한다.
     * 미지원 기기면 Unavailable 을 돌려주고 아무것도 저장하지 않는다(흐름은 그대로).
     */
    suspend fun summarizeRecent(
        context: Context,
        periodMs: Long,
    ): Summarizer.SummaryResult = withContext(Dispatchers.IO) {
        if (!SummaryPrefs.enabled(context)) return@withContext Summarizer.SummaryResult.Unavailable("요약이 꺼져 있습니다(설정에서 켜기)")
        val until = System.currentTimeMillis()
        val since = until - periodMs
        val digest = store.digest(since, until, DIGEST_MAX_CHARS)
        if (digest.isBlank()) return@withContext Summarizer.SummaryResult.Unavailable("요약할 기록이 없습니다")
        val summarizer = if (LlmConfig.isConfigured(context)) CloudSummarizer(context) else GeminiNanoSummarizer(context)
        val result = summarizer.summarize(digest)
        if (result is Summarizer.SummaryResult.Ok) {
            val id = store.insertSummary(until, since, until, result.summary)
            _summaries.update { (listOf(Summary(id, until, since, until, result.summary)) + it).take(KEEP) }
        }
        result
    }

    /**
     * 보존 정리. 이미 내보냈고 [retentionMs] 보다 오래된 원본을 지운다(요약은 유지).
     * 지운 행 수를 반환하고, 화면용 목록을 다시 읽는다.
     */
    suspend fun pruneExportedOlderThan(context: Context, retentionMs: Long): Int = withContext(Dispatchers.IO) {
        val prefs = context.getSharedPreferences("export_state", Context.MODE_PRIVATE)
        val exportedIds = store.tables().associateWith { prefs.getLong("last_$it", 0L) }
        val cutoff = System.currentTimeMillis() - retentionMs
        val deleted = store.prune(cutoff, exportedIds)
        if (deleted > 0) {
            _voiceSegments.value = store.recentVoice(KEEP)
            _appEvents.value = store.recentApp(KEEP)
            _screenEvents.value = store.recentScreen(KEEP)
            _locations.value = store.recentLocation(KEEP)
            _notifications.value = store.recentNotification(KEEP)
            _commEvents.value = store.recentComm(KEEP)
            _calendarEvents.value = store.recentCalendar(KEEP)
        }
        deleted
    }

    /**
     * HTTP 엔드포인트로 증분 전송. 설정된 URL 로 지난 전송 이후 새 기록만 NDJSON 으로 POST.
     * 성공(2xx) 시에만 전송 커서를 전진시킨다(실패하면 다음에 다시 보냄).
     * 반환: 보낸 줄 수(>=0), -1 미설정, -2 전송 실패.
     */
    suspend fun pushToHttp(context: Context): Int = withContext(Dispatchers.IO) {
        val url = EndpointConfig.url(context) ?: return@withContext -1
        val prefs = context.getSharedPreferences("http_state", Context.MODE_PRIVATE)
        val lastIds = store.tables().associateWith { prefs.getLong("last_$it", 0L) }
        val result = store.newRows(lastIds, includeSummary = SummaryPrefs.enabled(context))
        if (result.lines.isEmpty()) return@withContext 0
        val body = result.lines.joinToString("\n", postfix = "\n")
        val ok = HttpSink.post(url, EndpointConfig.token(context), body)
        if (!ok) return@withContext -2
        prefs.edit().apply {
            result.newLastIds.forEach { (table, id) -> putLong("last_$table", id) }
        }.apply()
        result.lines.size
    }

    /** 모든 기록을 지운다(되돌릴 수 없음). */
    fun deleteAll() {
        scope.launch {
            store.deleteAll()
            _voiceSegments.value = emptyList()
            _appEvents.value = emptyList()
            _screenEvents.value = emptyList()
            _locations.value = emptyList()
            _notifications.value = emptyList()
            _summaries.value = emptyList()
            _commEvents.value = emptyList()
            _calendarEvents.value = emptyList()
            _conversations.value = emptyList()
            _partial.value = ""
        }
    }

    fun addComm(kind: String, direction: String, party: String, partyName: String?, content: String, timestampMs: Long) {
        scope.launch {
            val id = store.insertComm(timestampMs, kind, direction, party, partyName, content)
            val event = CommEvent(id, timestampMs, kind, direction, party, partyName, content)
            _commEvents.update { (listOf(event) + it).take(KEEP) }
        }
    }

    fun addCalendar(title: String, location: String, beginMs: Long, endMs: Long, attendees: String) {
        scope.launch {
            val id = store.insertCalendar(beginMs, title, location, endMs, attendees)
            val entry = CalendarEntry(id, beginMs, title, location, endMs, attendees)
            _calendarEvents.update { (listOf(entry) + it).take(KEEP) }
        }
    }

    /**
     * 음성 조각을 시간 간격으로 묶어 대화를 만들고, 겹치는 통화·회의(참석자)로 참석자를 자동 연결한다.
     * 자동으로 안 채워지면 pending 으로 둬 검토 큐에 올린다. 만든 대화 수 반환.
     */
    suspend fun buildConversations(context: Context): Int = withContext(Dispatchers.IO) {
        val prefs = context.getSharedPreferences("conv_state", Context.MODE_PRIVATE)
        val builtUntil = prefs.getLong("built_until", 0L)
        val now = System.currentTimeMillis()
        val segs = store.recentVoice(KEEP).filter { it.timestampMs > builtUntil }.sortedBy { it.timestampMs }
        if (segs.isEmpty()) return@withContext 0
        var created = 0
        var newBuiltUntil = builtUntil
        var groupStart = segs.first().timestampMs
        var groupEnd = segs.first().timestampMs
        for (i in 1..segs.lastIndex) {
            val t = segs[i].timestampMs
            if (t - groupEnd > CONV_GAP_MS) {
                createConversation(groupStart, groupEnd)
                created++; newBuiltUntil = groupEnd
                groupStart = t; groupEnd = t
            } else {
                groupEnd = t
            }
        }
        if (groupEnd < now - CONV_GAP_MS) {
            createConversation(groupStart, groupEnd)
            created++; newBuiltUntil = groupEnd
        }
        prefs.edit().putLong("built_until", newBuiltUntil).apply()
        _conversations.value = store.recentConversation(KEEP)
        created
    }

    private fun createConversation(startMs: Long, endMs: Long) {
        val calls = store.commsBetween(startMs, endMs).filter { it.kind == "call" }
        val cals = store.calendarOverlapping(startMs, endMs)
        val names = LinkedHashSet<String>()
        calls.forEach { names.add(it.partyName ?: it.party) }
        cals.forEach { e -> if (e.attendees.isNotBlank()) e.attendees.split(",").forEach { names.add(it.trim()) } }
        val context = cals.joinToString("; ") { it.title }
        val (source, status) = when {
            calls.isNotEmpty() -> "call" to "auto"
            names.isNotEmpty() -> "calendar" to "auto"
            else -> "" to "pending"
        }
        store.insertConversation(startMs, endMs, names.joinToString(", "), source, status, context)
    }

    /** 검토 큐에서 참석자를 수동으로 채운다. */
    fun tagConversation(id: Long, participants: String) {
        scope.launch {
            store.tagConversation(id, participants)
            _conversations.value = store.recentConversation(KEEP)
        }
    }

    fun setPartial(text: String) { _partial.value = text }
    fun setStatus(text: String) { _status.value = text }
}
