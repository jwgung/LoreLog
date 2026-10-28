package com.lorelog

import android.Manifest
import android.app.TimePickerDialog
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.lorelog.audio.VoiceAudio
import com.lorelog.data.LogRepository
import com.lorelog.net.DiaryClient
import com.lorelog.work.DiaryReminderWorker
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** 한 번에 묻는 대화 수 상한(화면을 다시 열면 초기화). */
private const val ASK_PER_SESSION = 5

/** 누구의 대화였는지 한 번에 한 건씩 묻는 카드. 메인 화면과 일기 화면이 같이 쓴다. */
@Composable
internal fun ConversationReview(onPendingChanged: (Int) -> Unit = {}) {
    val context = LocalContext.current
    val conversations by LogRepository.conversations.collectAsStateWithLifecycle()
    val voiceSegments by LogRepository.voiceSegments.collectAsStateWithLifecycle()
    val tagInputs = remember { mutableStateMapOf<Long, String>() }
    // 한 번에 한 건만 묻는다. 말 조각이 2개 미만인 짧은 소리는 묻지 않고, 한 번에 묻는 양도 제한한다.
    val askable = conversations
        .filter { c -> c.status == "pending" && voiceSegments.count { it.timestampMs in c.startMs..c.endMs } >= 2 }
        .sortedBy { it.startMs }
    val knownNames = conversations
        .filter { it.status == "tagged" && it.source == "manual" }
        .flatMap { it.participants.split(",").map { n -> n.trim() } }
        .filter { it.isNotEmpty() }
        .groupingBy { it }.eachCount().entries.sortedByDescending { it.value }.take(6).map { it.key }
    var answered by remember { mutableStateOf(0) }
    if (askable.isNotEmpty() && answered < ASK_PER_SESSION) {
        val conv = askable.first()
        val total = answered + minOf(askable.size, ASK_PER_SESSION - answered)
        val segs = voiceSegments.filter { it.timestampMs in conv.startMs..conv.endMs }.sortedBy { it.timestampMs }
        val playable = segs.firstOrNull { it.audio != null }
        Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.medium, modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("누구의 대화였나요? (${answered + 1}/$total)", style = MaterialTheme.typography.titleSmall)
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text(
                        timeFormat.format(Date(conv.startMs)) + "\u2013" + timeFormat.format(Date(conv.endMs)) +
                            if (conv.context.isNotEmpty()) " \u00b7 ${conv.context}" else "",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                    )
                    playable?.audio?.let { PlayButton(VoiceAudio.file(context, it), playable.text) }
                }
                Text(
                    segs.take(3).joinToString("\n") { it.text.replace('\n', ' ').take(80) } +
                        if (segs.size > 3) "\n\u2026 \uc678 ${segs.size - 3}\uc870\uac01" else "",
                    style = MaterialTheme.typography.bodySmall,
                )
                if (knownNames.isNotEmpty()) {
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp), modifier = Modifier.horizontalScroll(rememberScrollState())) {
                        knownNames.forEach { name ->
                            OutlinedButton(onClick = {
                                val cur = tagInputs[conv.id].orEmpty()
                                tagInputs[conv.id] = if (cur.isBlank()) name else "$cur, $name"
                            }) { Text(name) }
                        }
                    }
                }
                OutlinedTextField(
                    value = tagInputs[conv.id] ?: "",
                    onValueChange = { tagInputs[conv.id] = it },
                    label = { Text("내가 참여한 대화 \u2014 누구와") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        enabled = !tagInputs[conv.id].isNullOrBlank(),
                        onClick = {
                            LogRepository.tagConversation(conv.id, tagInputs[conv.id].orEmpty())
                            tagInputs.remove(conv.id); answered++
                        },
                    ) { Text("저장") }
                    OutlinedButton(onClick = {
                        LogRepository.ambientConversation(conv.id)
                        tagInputs.remove(conv.id); answered++
                    }) { Text("내 대화 아님") }
                    TextButton(onClick = {
                        LogRepository.skipConversation(conv.id)
                        tagInputs.remove(conv.id); answered++
                    }) { Text("모르겠음") }
                }
            }
        }
    } else if (askable.isNotEmpty()) {
        Text(
            "남은 대화 ${askable.size}건은 다음에 물어볼게요.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }

    LaunchedEffect(askable.size) { onPendingChanged(askable.size) }
}

@Composable
internal fun DiaryReminderSettings() {
    val context = LocalContext.current
    var enabled by remember { mutableStateOf(DiaryPrefs.enabled(context)) }
    var hour by remember { mutableStateOf(DiaryPrefs.hour(context)) }
    var minute by remember { mutableStateOf(DiaryPrefs.minute(context)) }
    fun notifGranted() = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
    var granted by remember { mutableStateOf(notifGranted()) }
    val askNotif = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    // 알림을 켜는 순간 권한이 없으면 묻는다(없으면 알림이 조용히 사라진다).
    fun ensurePermission() { if (!notifGranted()) askNotif.launch(Manifest.permission.POST_NOTIFICATIONS) }
    Text(
        "정한 시각에 하루를 정리한 초안을 확인하고 한마디 남기라는 알림을 보냅니다.",
        style = MaterialTheme.typography.bodySmall,
    )
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Switch(checked = enabled, onCheckedChange = {
            enabled = it
            DiaryPrefs.setEnabled(context, it)
            DiaryReminderWorker.schedule(context)
            if (it) ensurePermission()
        })
        OutlinedButton(enabled = enabled, onClick = {
            TimePickerDialog(context, { _, h, m ->
                hour = h; minute = m
                DiaryPrefs.setTime(context, h, m)
                DiaryReminderWorker.schedule(context)
            }, hour, minute, true).show()
        }) { Text("%02d:%02d".format(hour, minute)) }
    }
    if (enabled && !granted) {
        Text(
            "알림 권한이 꺼져 있어 알림이 오지 않습니다.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
        )
        OutlinedButton(onClick = { ensurePermission() }) { Text("알림 권한 허용") }
    }
}

/**
 * 일기 재료로 쓸 메신저를 고른다. 기본은 전부 끔. 앱 전체를 켜거나, 상대별로 고른다.
 * 업무용인지 사적인지는 사람마다 달라서 앱이 판단하지 않는다. 원본 수집에는 영향이 없다(일기 재료만 거른다).
 */
@Composable
internal fun DiarySenderSettings() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var senders by remember { mutableStateOf<List<DiaryClient.Sender>?>(null) }
    val checked = remember { mutableStateMapOf<String, Boolean>() }   // "앱: 상대"
    val wholeApp = remember { mutableStateMapOf<String, Boolean>() }  // 앱 이름
    var filter by remember { mutableStateOf("") }
    var message by remember { mutableStateOf("") }

    Text(
        "고른 상대의 알림만 일기 재료로 씁니다(기본은 모두 제외). 수집 원본은 그대로 남습니다.",
        style = MaterialTheme.typography.bodySmall,
    )
    OutlinedButton(onClick = {
        scope.launch {
            message = "불러오는 중…"
            val list = DiaryClient.fetchSenders(context)
            if (list == null) { message = "서버에서 목록을 가져오지 못했습니다."; return@launch }
            senders = list; message = ""
            checked.clear(); wholeApp.clear()
            list.forEach { if (it.allowed) checked["${it.label}: ${it.title}"] = true }
        }
    }) { Text("상대 목록 불러오기") }
    if (message.isNotEmpty()) Text(message, style = MaterialTheme.typography.bodySmall)

    val list = senders ?: return
    OutlinedTextField(
        value = filter, onValueChange = { filter = it },
        label = { Text("이름에 포함된 글자로 찾기 (예: 회사 이름)") },
        singleLine = true, modifier = Modifier.fillMaxWidth(),
    )
    val shown = list.filter { filter.isBlank() || it.title.contains(filter, true) || it.label.contains(filter, true) }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(onClick = { shown.forEach { checked["${it.label}: ${it.title}"] = true } }) { Text("보이는 것 모두 선택") }
        OutlinedButton(onClick = { shown.forEach { checked.remove("${it.label}: ${it.title}") } }) { Text("모두 해제") }
    }
    shown.groupBy { it.label }.forEach { (label, rows) ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
            Text("앱 전체", style = MaterialTheme.typography.bodySmall)
            Switch(checked = wholeApp[label] == true, onCheckedChange = { wholeApp[label] = it })
        }
        rows.take(40).forEach { r ->
            val key = "${r.label}: ${r.title}"
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(checked = checked[key] == true || wholeApp[label] == true, enabled = wholeApp[label] != true,
                    onCheckedChange = { if (it) checked[key] = true else checked.remove(key) })
                Text("${r.title} (${r.count})", style = MaterialTheme.typography.bodyMedium)
            }
        }
        if (rows.size > 40) Text("… 외 ${rows.size - 40}개는 위 검색칸으로 찾으세요.", style = MaterialTheme.typography.bodySmall)
    }
    Button(onClick = {
        scope.launch {
            val allow = wholeApp.filterValues { it }.keys.map { "$it: *" } +
                checked.filterValues { it }.keys.filter { k -> wholeApp[k.substringBefore(": ")] != true }
            message = if (DiaryClient.saveAllow(context, allow)) "저장했어요(${allow.size}개). 일기 화면의 '다시 정리'부터 반영됩니다." else "저장하지 못했습니다."
        }
    }) { Text("저장") }
}
