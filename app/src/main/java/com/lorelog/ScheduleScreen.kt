package com.lorelog

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.lorelog.data.Schedule
import com.lorelog.net.DiaryClient
import java.time.LocalDate

private val SGround = Color(0xFF14161B)
private val SSurface = Color(0xFF1C2027)
private val SLine = Color(0xFF2B313B)
private val SText = Color(0xFFE8EAEE)
private val SSub = Color(0xFF8A93A1)
private val SAccent = Color(0xFF9DB7FF)
private val SWarn = Color(0xFFE8B86A)

/**
 * 일정(#62). 캘린더를 만들지 않고 읽는다: 금고에 들어온 일정을 오늘부터 7일 보여 주고 한 주씩 앞뒤로 넘긴다.
 * 일정마다 비서 메모(답하지 않은 요청, 같은 건의 마지막 상태, 그 사람과의 마지막 기록, 열린 할 일)가 서버의
 * 템플릿 문장으로 붙고, 확실한 것이 없으면 비어 있다. 일정을 누르면 그 일정·건 질문으로 대화가 열린다.
 */
@Composable
internal fun ScheduleScreen(onBack: () -> Unit, onAsk: (String) -> Unit) {
    val context = LocalContext.current
    val today = remember { LocalDate.now() }
    var offset by remember { mutableIntStateOf(0) }
    val span = remember(offset) { Schedule.page(today, offset) }
    var result by remember { mutableStateOf<DiaryClient.ScheduleResult?>(null) }
    LaunchedEffect(span) {
        result = null
        result = DiaryClient.schedule(context, span)
    }

    Column(modifier = Modifier.fillMaxSize().background(SGround).safeDrawingPadding()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 12.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text("일정", color = SText, fontSize = 18.sp)
            NavButton(NavIcon.CHAT, "대화로", onClick = onBack)
        }
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            TextButton(onClick = { offset-- }) { Text("‹ 앞 주", color = SSub) }
            if (offset != 0) TextButton(onClick = { offset = 0 }) { Text("오늘부터", color = SAccent) }
            TextButton(onClick = { offset++ }) { Text("다음 주 ›", color = SSub) }
        }
        Column(
            modifier = Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            when (val r = result) {
                null -> Text("불러오는 중…", color = SSub, fontSize = 13.sp)
                DiaryClient.ScheduleResult.Failed ->
                    Text("금고에 연결하지 못했어요. 연결을 확인하고 다시 열어 주세요.", color = SSub, fontSize = 13.sp)
                DiaryClient.ScheduleResult.NotReady ->
                    Text("금고가 아직 기록을 정리하지 않았어요. 잠시 뒤 다시 열어 주세요.", color = SSub, fontSize = 13.sp)
                is DiaryClient.ScheduleResult.Ready -> {
                    if (r.events.isEmpty()) {
                        Text(
                            "이 주에는 금고에 들어온 일정이 없어요. 폰 캘린더나 CalDAV 일정이 금고로 들어오면 여기에 보여요.",
                            color = SSub, fontSize = 13.sp, lineHeight = 20.sp,
                        )
                    }
                    Schedule.byDay(span, r.events).forEach { d ->
                        Text(
                            Schedule.dayLabel(d.day, today), color = SSub, fontSize = 12.sp,
                            modifier = Modifier.padding(start = 4.dp, top = 8.dp),
                        )
                        if (d.events.isEmpty()) Text("일정 없음", color = Color(0xFF5A6577), fontSize = 13.sp, modifier = Modifier.padding(start = 4.dp))
                        d.events.forEach { e -> EventRow(e, onClick = { if (e.question.isNotBlank()) onAsk(e.question) }) }
                    }
                }
            }
            Spacer(Modifier.size(24.dp))
        }
    }
}

@Composable
private fun EventRow(e: Schedule.Event, onClick: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(SSurface)
            .border(1.dp, SLine, RoundedCornerShape(16.dp)).clickable(onClick = onClick)
            .semantics { contentDescription = "${e.title}, 대화로 묻기" }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(e.title, color = SText, fontSize = 15.sp, lineHeight = 22.sp)
        val meta = listOf(Schedule.time(e), e.place.takeIf { it.isNotBlank() }, e.attendees.take(3).joinToString(", ").takeIf { it.isNotBlank() })
            .filterNotNull().joinToString(" · ")
        Text(meta, color = SSub, fontSize = 12.sp)
        e.notes.forEach { n ->
            Text(n.text, color = if (n.kind == "request") SWarn else if (n.guess) SSub else SText, fontSize = 13.sp, lineHeight = 20.sp)
        }
    }
}
