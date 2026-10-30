package com.lorelog

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import com.lorelog.collect.Dictation
import androidx.compose.material3.Icon
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.withStyle
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.lorelog.data.LogRepository
import com.lorelog.net.DiaryClient
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private val Ground = Color(0xFF14161B)
private val Surface = Color(0xFF1C2027)
private val Bubble = Color(0xFF222730)
private val Line = Color(0xFF2B313B)
private val Text0 = Color(0xFFE8EAEE)
private val Sub = Color(0xFF8A93A1)
private val Accent = Color(0xFF9DB7FF)
private val Ink = Color(0xFF0E1630)
private val Picked = Color(0xFF3A4766)

/** 대화 화면에 덧붙는 말풍선(서버 초안 뒤에 사용자가 주고받은 것). */
private sealed class Msg {
    data class Mine(val text: String) : Msg()
    data class Assistant(val text: String) : Msg()
    data class Fixed(val wrong: String, val right: String) : Msg()
    data object Typing : Msg()
}

/**
 * 앱의 홈. 하루 끝에 비서가 먼저 말을 건네고(오늘의 정리 + 질문 하나), 사용자는 한마디로 답하거나
 * 틀린 문장을 눌러 고친다. 보내면 저장되고 저장 버튼은 없다. 가만히 두면 정리는 맞는 것으로 본다.
 */
@Composable
internal fun ChatScreen(onDrawer: () -> Unit, onCollect: () -> Unit, onTodos: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val running by LoggerService.isRunning.collectAsStateWithLifecycle()
    var day by remember { mutableStateOf(DiaryPrefs.diaryDay()) }
    val dayLabel = remember(day) {
        SimpleDateFormat("M월 d일 EEEE", Locale.KOREAN).format(SimpleDateFormat("yyyy-MM-dd", Locale.US).parse(day) ?: Date())
    }
    // 앱을 켜 둔 채 날이 바뀌면(새벽 4시) 다음에 돌아올 때 새 날로 바꾼다.
    val lifecycleOwner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    androidx.compose.runtime.DisposableEffect(lifecycleOwner) {
        val obs = androidx.lifecycle.LifecycleEventObserver { _, e ->
            if (e == androidx.lifecycle.Lifecycle.Event.ON_RESUME) day = DiaryPrefs.diaryDay()
        }
        lifecycleOwner.lifecycle.addObserver(obs)
        onDispose { lifecycleOwner.lifecycle.removeObserver(obs) }
    }
    var pendingCards by remember { mutableStateOf(-1) }
    var started by remember { mutableStateOf(false) }
    var reload by remember { mutableStateOf(0) }
    var draft by remember { mutableStateOf<DiaryClient.Draft?>(null) }
    val extra = remember { mutableStateListOf<Msg>() }
    var selected by remember { mutableStateOf<Int?>(null) }
    var fixing by remember { mutableStateOf<String?>(null) }
    var input by remember { mutableStateOf("") }
    var todoCount by remember { mutableStateOf(0) }
    LaunchedEffect(Unit) { todoCount = DiaryClient.fetchTodos(context)?.let { it.open.size + it.proposed.size } ?: 0 }
    // 앱을 다시 열어도 오늘 나눈 대화가 이어서 보이게 서버에서 불러온다.
    LaunchedEffect(day) {
        extra.clear(); selected = null; fixing = null
        var pendingFix: Msg.Fixed? = null
        val restored = mutableListOf<Msg>()
        for (l in DiaryClient.fetchChat(context, day)) {
            if (l.role == "user") {
                restored.add(Msg.Mine(l.text))
                pendingFix = l.wrong?.let { Msg.Fixed(it, l.text) }
            } else {
                restored.add(Msg.Assistant(l.text))
                pendingFix?.let { restored.add(it); pendingFix = null }
            }
        }
        if (restored.isNotEmpty() && extra.isEmpty()) extra.addAll(restored)
    }
    val scroll = rememberScrollState()
    val dictation by Dictation.state.collectAsStateWithLifecycle()
    var notice by remember { mutableStateOf("") }

    fun startListening() {
        notice = ""
        if (!Dictation.available(context)) {
            notice = "말로 남기려면 받아 적을 곳이 필요해요. 서랍 → 수집·연동 설정에서 서버 주소를 넣거나, 음성 전사 방식에서 온디바이스 모델을 받아 주세요. 말은 밖으로 나가지 않아요."
            return
        }
        scope.launch {
            val text = Dictation.listen(context)
            if (text == null) notice = "받아 적지 못했어요. 서버에 닿지 않았거나 말소리가 너무 짧았을 수 있어요. 다시 한 번 말해 주세요."
            else input = if (input.isBlank()) text else input.trimEnd() + " " + text
        }
    }
    val askMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) startListening() else notice = "마이크 권한이 없어서 말로 남길 수 없어요."
    }
    fun onMic() {
        when (dictation) {
            Dictation.State.LISTENING -> Dictation.stop()
            Dictation.State.TRANSCRIBING -> {}
            Dictation.State.IDLE ->
                if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) startListening()
                else askMic.launch(Manifest.permission.RECORD_AUDIO)
        }
    }

    // 확인할 대화가 없으면 바로 정리를 가져온다. 있으면 사용자가 카드를 처리한 뒤 "정리 보기"를 누른다.
    LaunchedEffect(pendingCards) { if (pendingCards == 0) started = true }
    LaunchedEffect(started, reload, day) {
        if (!started) return@LaunchedEffect
        draft = DiaryClient.Draft.Generating
        // 최신 기록과 대화 상태를 서버로 먼저 보낸다. 그래야 정리에 오늘 것이 들어간다.
        LogRepository.pushToHttp(context)
        LogRepository.pushConversations(context)
        var refresh = reload > 0
        repeat(100) {
            val d = DiaryClient.fetchDraft(context, day, refresh)
            refresh = false
            draft = d
            if (d !is DiaryClient.Draft.Generating) return@LaunchedEffect
            delay(3_000)
        }
        draft = DiaryClient.Draft.Error("정리가 오래 걸려요. 잠시 뒤에 다시 시도해 주세요.")
    }
    LaunchedEffect(extra.size, draft, selected) { scroll.animateScrollTo(scroll.maxValue) }

    fun send() {
        val t = input.trim()
        if (t.isEmpty()) return
        input = ""
        extra.add(Msg.Mine(t))
        val wrong = fixing
        fixing = null; selected = null
        extra.add(Msg.Typing)
        scope.launch {
            val reply = DiaryClient.chat(context, day, t, wrong)
            extra.remove(Msg.Typing)
            if (reply == null) {
                extra.add(Msg.Assistant("연결이 안 돼서 답을 못 드렸어요. 말씀하신 건 다시 보내 주시면 남길게요."))
                return@launch
            }
            extra.add(Msg.Assistant(reply))
            if (wrong != null) extra.add(Msg.Fixed(wrong, t))
        }
    }

    Column(modifier = Modifier.fillMaxSize().background(Ground).safeDrawingPadding().imePadding()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 12.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column {
                Text("오늘", color = Text0, fontSize = 18.sp)
                Text(
                    if (running) "● 기록 중" else "○ 멈춤",
                    color = Sub, fontSize = 12.sp,
                    modifier = Modifier.clickable { onCollect() }.padding(vertical = 6.dp),
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                NavButton(NavIcon.TODO, "할 일", badge = todoCount, onClick = onTodos)
                NavButton(NavIcon.SETTINGS, "수집·연동 설정", onClick = onCollect)
                NavButton(NavIcon.DRAWER, "서랍", onClick = onDrawer)
            }
        }

        Column(
            modifier = Modifier.weight(1f).fillMaxWidth().verticalScroll(scroll).padding(horizontal = 20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(dayLabel, color = Sub, fontSize = 12.sp, modifier = Modifier.align(Alignment.CenterHorizontally).padding(vertical = 6.dp))

            if (!started) {
                AssistantBubble("먼저 확인하고 싶은 대화가 있어요. 누구와 나눈 얘기인지 알려 주시면 정리에 쓸게요.")
                ConversationReview(onPendingChanged = { pendingCards = it })
                OutlinedButton(onClick = { started = true }) { Text("지금은 건너뛰고 정리 보기", color = Sub) }
            } else {
                // started 이후에도 카드 상태를 계속 알려야 하므로 숨은 형태로 유지한다.
                ConversationReview(onPendingChanged = { pendingCards = it })
            }

            when (val d = draft) {
                null -> {}
                DiaryClient.Draft.Generating -> AssistantBubble("오늘 하루를 정리하고 있어요…")
                is DiaryClient.Draft.Error -> {
                    AssistantBubble(d.message)
                    OutlinedButton(onClick = { reload++ }) { Text("다시 시도", color = Sub) }
                }
                is DiaryClient.Draft.Ready -> {
                    if (d.summary.isEmpty()) {
                        AssistantBubble("오늘은 정리할 기록이 거의 없어요. 하고 싶은 말이 있으면 남겨 주세요.")
                    } else {
                        AssistantBubble("오늘은 이런 하루였던 것 같아요.")
                        Box(
                            modifier = Modifier.widthIn(max = bubbleMax()).clip(RoundedCornerShape(18.dp)).background(Bubble).padding(14.dp),
                        ) {
                            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                                d.summary.forEachIndexed { i, line ->
                                    Text(
                                        styledSentence(line), color = Text0, fontSize = 15.sp, lineHeight = 26.sp, fontFamily = FontFamily.Serif,
                                        modifier = Modifier.clip(RoundedCornerShape(6.dp))
                                            .background(if (selected == i) Picked else Color.Transparent)
                                            .clickable { selected = if (selected == i) null else i; fixing = null }
                                            .padding(horizontal = 4.dp, vertical = 2.dp),
                                    )
                                }
                            }
                        }
                        selected?.let { i ->
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(start = 8.dp)) {
                                Button(onClick = { fixing = d.summary[i] }) { Text("이 문장이 틀렸어요") }
                                TextButton(onClick = { selected = null; fixing = null }) { Text("닫기", color = Sub) }
                            }
                        }
                        Text(
                            "틀린 문장은 눌러서 알려 주세요. 가만히 두시면 맞는 걸로 둘게요.",
                            color = Sub, fontSize = 12.sp, lineHeight = 19.sp, modifier = Modifier.padding(horizontal = 4.dp),
                        )
                    }
                    if (d.question.isNotBlank()) AssistantBubble(d.question, topGap = 8)
                }
            }

            extra.forEach { m ->
                when (m) {
                    is Msg.Mine -> Box(
                        modifier = Modifier.align(Alignment.End).widthIn(max = bubbleMax(0.78f))
                            .clip(RoundedCornerShape(18.dp, 18.dp, 4.dp, 18.dp)).background(Accent).padding(horizontal = 16.dp, vertical = 12.dp),
                    ) { Text(m.text, color = Ink, fontSize = 15.sp, lineHeight = 23.sp) }
                    is Msg.Assistant -> AssistantBubble(m.text)
                    Msg.Typing -> AssistantBubble("…")
                    is Msg.Fixed -> Column(
                        modifier = Modifier.padding(start = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Text("고친 기록", color = Sub, fontSize = 12.sp)
                        Text(m.wrong, color = Sub, fontSize = 13.sp, textDecoration = TextDecoration.LineThrough)
                        Text(m.right, color = Color(0xFFC9CED8), fontSize = 13.sp)
                    }
                }
            }
        }

        if (notice.isNotEmpty()) {
            Text(notice, color = Sub, fontSize = 12.sp, lineHeight = 19.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp))
        }
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 16.dp)
                .clip(RoundedCornerShape(28.dp)).background(Surface).border(1.dp, Line, RoundedCornerShape(28.dp))
                .padding(start = 20.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(modifier = Modifier.weight(1f).padding(vertical = 8.dp)) {
                if (input.isEmpty()) {
                    Text(
                        when {
                            dictation == Dictation.State.LISTENING -> "듣고 있어요… 다 말하면 마이크를 다시 눌러 주세요"
                            dictation == Dictation.State.TRANSCRIBING -> "받아 적는 중…"
                            fixing != null -> "맞는 내용을 말씀해 주세요"
                            else -> "한마디 남기기…"
                        },
                        color = Sub, fontSize = 15.sp,
                    )
                }
                BasicTextField(
                    value = input, onValueChange = { input = it },
                    textStyle = TextStyle(color = Text0, fontSize = 15.sp, lineHeight = 22.sp),
                    cursorBrush = SolidColor(Accent), maxLines = 4, modifier = Modifier.fillMaxWidth(),
                )
            }
            IconCircle(
                label = if (dictation == Dictation.State.LISTENING) "말하기 끝내기" else "말로 남기기",
                enabled = dictation != Dictation.State.TRANSCRIBING, filled = false, onClick = { onMic() },
            ) { Icon(if (dictation == Dictation.State.LISTENING) Lucide.Stop else Lucide.Mic, contentDescription = null, tint = if (dictation == Dictation.State.LISTENING) Color(0xFFE3A3B8) else Color(0xFFC9CED8), modifier = Modifier.size(22.dp)) }
            IconCircle(label = "보내기", enabled = input.isNotBlank(), filled = true, onClick = { send() }) { Icon(Lucide.Up, contentDescription = null, tint = if (input.isNotBlank()) Ink else Color(0xFF6F7785), modifier = Modifier.size(22.dp)) }
        }
    }
}

@Composable
private fun AssistantBubble(text: String, topGap: Int = 0) {
    Box(
        modifier = Modifier.padding(top = topGap.dp).widthIn(max = bubbleMax())
            .clip(RoundedCornerShape(4.dp, 18.dp, 18.dp, 18.dp)).background(Bubble).padding(horizontal = 16.dp, vertical = 14.dp),
    ) { Text(text, color = Text0, fontSize = 15.sp, lineHeight = 26.sp, fontFamily = FontFamily.Serif) }
}

/** 말풍선 최대 폭: 화면 폭의 비율. 태블릿에서 너무 넓어지지 않게 560dp 까지. */
@Composable
private fun bubbleMax(ratio: Float = 0.86f): androidx.compose.ui.unit.Dp =
    minOf((androidx.compose.ui.platform.LocalConfiguration.current.screenWidthDp * ratio).dp, 560.dp)

/** 권한이 필요한 항목 한 줄. 누르면 해당 시스템 설정으로 간다. */
@Composable
internal fun PermRow(title: String, onClick: () -> Unit) = DrawerRow(title, "켜기", onClick)

internal enum class NavIcon { DRAWER, CHAT, SETTINGS, BACK, TODO }

/** 화면 이동용 아이콘 버튼(44dp 원). 표준 Material 아이콘을 쓰고 접근성 설명을 붙인다. */
@Composable
internal fun NavButton(icon: NavIcon, label: String, badge: Int = 0, onClick: () -> Unit) {
    val vector = when (icon) {
        NavIcon.DRAWER -> Lucide.Archive
        NavIcon.CHAT -> Lucide.Chat
        NavIcon.SETTINGS -> Lucide.Settings
        NavIcon.BACK -> Lucide.Back
        NavIcon.TODO -> Lucide.Todo
    }
    Box(modifier = Modifier.size(44.dp)) {
        Box(
            modifier = Modifier.size(44.dp).clip(CircleShape).border(1.dp, Line, CircleShape).clickable(onClick = onClick),
            contentAlignment = Alignment.Center,
        ) { Icon(vector, contentDescription = label, tint = Color(0xFFB4BBC8), modifier = Modifier.size(22.dp)) }
        if (badge > 0) {
            // 원 안에 두면 잘리므로 원 위에 따로 얹는다.
            Box(
                modifier = Modifier.align(Alignment.TopEnd).offset(x = 4.dp, y = (-4).dp).size(20.dp).clip(CircleShape).background(Accent),
                contentAlignment = Alignment.Center,
            ) { Text(if (badge > 9) "9+" else "$badge", color = Ink, fontSize = 11.sp) }
        }
    }
}

private val TimeLead = Regex("^((?:오전|오후|저녁|아침|새벽|밤|정오)(?:부터)?\\s*(?:\\d{1,2}시)?(?:\\s*\\d{1,2}분)?(?:경)?)")

/** 정리 문장 앞의 시각 표현("오후 5시 36분경")을 흐리게 해서 무슨 일이었는지가 먼저 읽히게 한다. */
private fun styledSentence(line: String): androidx.compose.ui.text.AnnotatedString =
    androidx.compose.ui.text.buildAnnotatedString {
        val m = TimeLead.find(line)
        if (m == null) { append(line); return@buildAnnotatedString }
        withStyle(androidx.compose.ui.text.SpanStyle(color = Sub, fontSize = 13.sp)) { append(m.value) }
        append(line.substring(m.value.length))
    }

/** 윤곽선 알약 버튼. 화면 위쪽 이동 버튼에 쓴다. */
@Composable
internal fun Pill(text: String, onClick: () -> Unit) {
    Box(
        modifier = Modifier.clip(RoundedCornerShape(50)).border(1.dp, Line, RoundedCornerShape(50))
            .clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 12.dp),
    ) { Text(text, color = Sub, fontSize = 13.sp) }
}

/** 44dp 원형 아이콘 버튼. 채움(보내기)과 윤곽선(마이크) 두 가지. */
@Composable
internal fun IconCircle(label: String, enabled: Boolean, filled: Boolean, onClick: () -> Unit, icon: @Composable () -> Unit) {
    Box(
        modifier = Modifier.size(44.dp).clip(CircleShape)
            .background(if (filled) (if (enabled) Accent else Color(0xFF232833)) else Color.Transparent)
            .then(if (filled) Modifier else Modifier.border(1.dp, Line, CircleShape))
            .clickable(enabled = enabled, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) { icon() }
}

/** 서랍의 한 행: 이름, 지금 값, ›. 누르면 그 설정으로 들어간다. */
@Composable
internal fun DrawerRow(title: String, value: String, onClick: () -> Unit) {
    Column(modifier = Modifier.fillMaxWidth().clickable(onClick = onClick)) {
        androidx.compose.material3.HorizontalDivider(color = Line)
        Row(
            modifier = Modifier.fillMaxWidth().padding(vertical = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(title, color = Text0, fontSize = 15.sp)
            Text(if (value.isEmpty()) "›" else "$value  ›", color = Sub, fontSize = 14.sp)
        }
    }
}

/** 서랍: 비서와 나 사이의 설정이 모이는 곳. 항목마다 한 줄이고, 누르면 그 설정으로 들어간다. */
@Composable
internal fun DrawerScreen(onBack: () -> Unit, onOpen: (AppScreen) -> Unit) {
    val context = LocalContext.current
    var allowedCount by remember { mutableStateOf<Int?>(null) }
    LaunchedEffect(Unit) { allowedCount = DiaryClient.fetchSenders(context)?.count { it.allowed } }
    val reminder = if (!DiaryPrefs.enabled(context)) "꺼짐" else {
        val h = DiaryPrefs.hour(context)
        val half = if (h < 12) "오전" else "오후"
        "$half ${if (h % 12 == 0) 12 else h % 12}:%02d".format(DiaryPrefs.minute(context))
    }
    Column(
        modifier = Modifier.fillMaxSize().background(Ground).safeDrawingPadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text("서랍", color = Text0, fontSize = 18.sp)
            NavButton(NavIcon.CHAT, "대화", onClick = onBack)
        }
        Text(
            "저에 대해 아는 것은 여기에만 있고, 이 폰과 내 서버 밖으로 나가지 않아요. 나만 열 수 있어요.",
            color = Sub, fontSize = 13.sp, lineHeight = 21.sp, modifier = Modifier.padding(bottom = 6.dp),
        )
        DrawerRow("하루 정리 알림", reminder) { onOpen(AppScreen.REMINDER) }
        DrawerRow("일기에 쓰는 메신저", allowedCount?.let { "${it}곳" } ?: "") { onOpen(AppScreen.SENDERS) }
        DrawerRow("수집·연동 설정", "") { onOpen(AppScreen.COLLECT) }
        androidx.compose.material3.HorizontalDivider(color = Line)
        Text("내 것으로 가져가기", color = Sub, fontSize = 12.sp, modifier = Modifier.padding(top = 10.dp))
        Pill("내보내기·삭제") { onOpen(AppScreen.EXPORT) }
        Text("내보낸 파일은 텍스트와 JSON이라 다른 앱에서도 열려요.", color = Sub, fontSize = 12.sp, lineHeight = 19.sp, modifier = Modifier.padding(bottom = 32.dp))
    }
}

/** 서랍 안의 한 가지 설정 화면. 위에 서랍으로 돌아가는 알약이 있다. */
@Composable
internal fun DrawerPage(title: String, onBack: () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().background(Ground).safeDrawingPadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(title, color = Text0, fontSize = 18.sp)
            NavButton(NavIcon.BACK, "서랍으로", onClick = onBack)
        }
        content()
        androidx.compose.foundation.layout.Spacer(Modifier.size(32.dp))
    }
}
