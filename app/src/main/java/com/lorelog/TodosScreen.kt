package com.lorelog

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.lorelog.net.DiaryClient
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private val TGround = Color(0xFF14161B)
private val TSurface = Color(0xFF1C2027)
private val TLine = Color(0xFF2B313B)
private val TText = Color(0xFFE8EAEE)
private val TSub = Color(0xFF8A93A1)
private val TAccent = Color(0xFF9DB7FF)
private val TWarn = Color(0xFFE8B86A)

/**
 * 할 일. 비서가 기록(메일·통화·메신저·일정)에서 찾은 것은 제안으로 보이고, 사용자가 받아야 할 일이 된다.
 * 사용자는 끝냈으면 누르고 아니면 지운다. 지운 것은 다시 제안하지 않는다. 아래 입력창으로 직접 말해 넣을 수도 있다.
 */
@Composable
internal fun TodosScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var todos by remember { mutableStateOf<DiaryClient.Todos?>(null) }
    var failed by remember { mutableStateOf(false) }
    var input by remember { mutableStateOf("") }
    // 이름이 틀린 할 일을 고치는 중: (id, 지금 이름)
    var editing by remember { mutableStateOf<Pair<String, String>?>(null) }
    var editText by remember { mutableStateOf("") }
    var changed by remember { mutableStateOf("") }

    suspend fun reload() {
        val t = DiaryClient.fetchTodos(context)
        if (t == null) failed = true else { failed = false; todos = t }
    }
    LaunchedEffect(Unit) {
        reload()
        DiaryClient.refreshTodos(context) // 서버가 새 할 일을 뽑는 동안 목록을 다시 읽는다
        repeat(25) {
            delay(3_000)
            reload()
            if (todos?.extracting == false && it >= 1) return@LaunchedEffect
        }
    }
    fun act(id: String, action: String) = scope.launch { DiaryClient.actTodo(context, id, action); reload() }
    fun edit(item: DiaryClient.Todo) { editing = item.id to item.title; editText = item.title; changed = "" }
    fun saveEdit() {
        val (id, old) = editing ?: return
        val right = editText.trim()
        editing = null
        if (right.isEmpty() || right == old) return
        scope.launch {
            changed = DiaryClient.correct(context, old, right, "todo:$id") ?: "고치지 못했어요. 연결을 확인하고 다시 해 주세요."
            reload()
        }
    }

    Column(modifier = Modifier.fillMaxSize().background(TGround).safeDrawingPadding().imePadding()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 12.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text("할 일", color = TText, fontSize = 18.sp)
            NavButton(NavIcon.CHAT, "대화로", onClick = onBack)
        }
        Column(
            modifier = Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(
                "기록에서 찾은 건 위에 제안으로 올려요. 맞으면 받아 주시고, 아닌 건 지워 주세요. 지운 건 다시 올리지 않아요. 이름이 틀렸으면 눌러서 고쳐 주세요.",
                color = TSub, fontSize = 13.sp, lineHeight = 20.sp, modifier = Modifier.padding(vertical = 6.dp),
            )
            if (changed.isNotEmpty()) Text(changed, color = TAccent, fontSize = 13.sp, lineHeight = 20.sp)
            editing?.let {
                Column(
                    modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).border(1.dp, TAccent, RoundedCornerShape(16.dp)).padding(14.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text("맞는 이름으로 고쳐 주세요", color = TSub, fontSize = 12.sp)
                    BasicTextField(
                        value = editText, onValueChange = { editText = it }, maxLines = 3,
                        textStyle = TextStyle(color = TText, fontSize = 15.sp, lineHeight = 22.sp),
                        cursorBrush = SolidColor(TAccent), modifier = Modifier.fillMaxWidth(),
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        Text("고치기", color = TAccent, fontSize = 14.sp, modifier = Modifier.clickable { saveEdit() }.padding(vertical = 6.dp))
                        Text("그만두기", color = TSub, fontSize = 14.sp, modifier = Modifier.clickable { editing = null }.padding(vertical = 6.dp))
                    }
                }
            }
            val t = todos
            when {
                t == null && failed -> Text("서버에 연결하지 못했어요. 연결을 확인하고 다시 열어 주세요.", color = TSub, fontSize = 13.sp)
                t == null -> Text("불러오는 중…", color = TSub, fontSize = 13.sp)
                else -> {
                    if (t.proposed.isNotEmpty()) {
                        Text("기록에서 찾은 할 일", color = TSub, fontSize = 12.sp, modifier = Modifier.padding(start = 4.dp))
                        t.proposed.forEach { item -> ProposedRow(item, onAccept = { act(item.id, "accept") }, onDismiss = { act(item.id, "dismiss") }, onEdit = { edit(item) }) }
                        androidx.compose.foundation.layout.Spacer(Modifier.size(6.dp))
                    }
                    if (t.open.isEmpty() && t.proposed.isEmpty()) {
                        Text(
                            if (t.extracting) "기록에서 할 일을 찾고 있어요…" else "지금 할 일이 없어요. 하고 싶은 일이 있으면 아래에 말해 주세요.",
                            color = TSub, fontSize = 14.sp, lineHeight = 22.sp,
                        )
                    }
                    t.open.forEach { item ->
                        TodoRow(item, done = false, onToggle = { act(item.id, "done") }, onDismiss = { act(item.id, "dismiss") }, onEdit = { edit(item) })
                    }
                    if (t.open.isNotEmpty() && t.extracting) Text("더 찾고 있어요…", color = TSub, fontSize = 12.sp)
                    if (t.done.isNotEmpty()) {
                        Text("끝낸 일", color = TSub, fontSize = 12.sp, modifier = Modifier.padding(start = 4.dp, top = 14.dp))
                        t.done.forEach { item -> TodoRow(item, done = true, onToggle = { act(item.id, "reopen") }, onDismiss = null) }
                    }
                }
            }
            androidx.compose.foundation.layout.Spacer(Modifier.size(12.dp))
        }
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 16.dp)
                .clip(RoundedCornerShape(28.dp)).background(TSurface).border(1.dp, TLine, RoundedCornerShape(28.dp))
                .padding(start = 20.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(modifier = Modifier.weight(1f).padding(vertical = 8.dp)) {
                if (input.isEmpty()) Text("할 일 말하기…", color = TSub, fontSize = 15.sp)
                BasicTextField(
                    value = input, onValueChange = { input = it }, maxLines = 2,
                    textStyle = TextStyle(color = TText, fontSize = 15.sp, lineHeight = 22.sp),
                    cursorBrush = SolidColor(TAccent), modifier = Modifier.fillMaxWidth(),
                )
            }
            IconCircle(label = "추가", enabled = input.isNotBlank(), filled = true, onClick = {
                val text = input.trim(); input = ""
                scope.launch { DiaryClient.addTodo(context, text); reload() }
            }) { Icon(Lucide.Up, contentDescription = null, tint = if (input.isNotBlank()) Color(0xFF0E1630) else Color(0xFF6F7785), modifier = Modifier.size(22.dp)) }
        }
    }
}

/** 비서의 제안: 받으면(+) 할 일이 되고, 지우면(×) 다시 올리지 않는다. */
@Composable
private fun ProposedRow(item: DiaryClient.Todo, onAccept: () -> Unit, onDismiss: () -> Unit, onEdit: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).border(1.dp, TLine, RoundedCornerShape(16.dp))
            .padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(
            modifier = Modifier.weight(1f).clickable(onClick = onEdit).semantics { contentDescription = "이름 고치기" }.padding(vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            Text(item.title, fontSize = 15.sp, lineHeight = 22.sp, color = TText)
            val meta = listOf(item.source.takeIf { it.isNotBlank() }?.let { "${it}에서" }, item.due.takeIf { it.isNotBlank() })
                .filterNotNull().joinToString(" · ")
            if (meta.isNotEmpty()) Text(meta, fontSize = 12.sp, color = TSub)
        }
        Box(
            modifier = Modifier.size(44.dp).clip(CircleShape).clickable(onClick = onAccept).semantics { contentDescription = "할 일로 받기" },
            contentAlignment = Alignment.Center,
        ) { Icon(Lucide.Check, contentDescription = null, tint = TAccent, modifier = Modifier.size(18.dp)) }
        Box(
            modifier = Modifier.size(44.dp).clip(CircleShape).clickable(onClick = onDismiss).semantics { contentDescription = "아니에요" },
            contentAlignment = Alignment.Center,
        ) { Icon(Lucide.Close, contentDescription = null, tint = TSub, modifier = Modifier.size(18.dp)) }
    }
}

@Composable
private fun TodoRow(item: DiaryClient.Todo, done: Boolean, onToggle: () -> Unit, onDismiss: (() -> Unit)?, onEdit: (() -> Unit)? = null) {
    Row(
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(if (done) Color.Transparent else TSurface)
            .then(if (done) Modifier else Modifier.border(1.dp, TLine, RoundedCornerShape(16.dp)))
            .padding(start = 8.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier = Modifier.size(44.dp).clip(CircleShape).clickable(onClick = onToggle)
                .semantics { contentDescription = if (done) "다시 열기" else "끝냈어요" },
            contentAlignment = Alignment.Center,
        ) {
            Box(
                modifier = Modifier.size(24.dp).clip(CircleShape)
                    .then(if (done) Modifier.background(Color(0xFF3C4A66)) else Modifier.border(2.dp, Color(0xFF5A6577), CircleShape)),
                contentAlignment = Alignment.Center,
            ) { if (done) Icon(Lucide.Check, contentDescription = null, tint = Color(0xFFC9D6F5), modifier = Modifier.size(15.dp)) }
        }
        Column(
            modifier = Modifier.weight(1f)
                .then(if (onEdit != null) Modifier.clickable(onClick = onEdit).semantics { contentDescription = "이름 고치기" } else Modifier)
                .padding(vertical = 8.dp, horizontal = 6.dp),
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            Text(
                item.title, fontSize = 15.sp, lineHeight = 22.sp,
                color = if (done) TSub else TText, textDecoration = if (done) TextDecoration.LineThrough else null,
            )
            if (done && item.auto) Text("답장을 보내서 자동으로 끝냈어요", fontSize = 12.sp, color = TSub)
            if (!done) {
                val meta = listOf(item.source.takeIf { it.isNotBlank() }?.let { "${it}에서" }, item.due.takeIf { it.isNotBlank() })
                    .filterNotNull().joinToString(" · ")
                if (meta.isNotEmpty()) Text(meta, fontSize = 12.sp, color = TSub)
            }
        }
        if (onDismiss != null) {
            Box(
                modifier = Modifier.size(44.dp).clip(CircleShape).clickable(onClick = onDismiss).semantics { contentDescription = "지우기" },
                contentAlignment = Alignment.Center,
            ) { Icon(Lucide.Close, contentDescription = null, tint = TSub, modifier = Modifier.size(18.dp)) }
        }
    }
}
