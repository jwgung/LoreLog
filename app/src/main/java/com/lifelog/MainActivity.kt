package com.lifelog

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.FileProvider
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import com.lifelog.collect.UsageAccess
import com.lifelog.data.LogRepository
import com.lifelog.data.TimelineItem
import com.lifelog.net.EndpointConfig
import com.lifelog.net.LlmConfig
import com.lifelog.summary.Summarizer
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val context = LocalContext.current
            val colors = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S)
                dynamicDarkColorScheme(context) else darkColorScheme()
            MaterialTheme(colorScheme = colors) {
                Surface(modifier = Modifier.fillMaxSize()) {
                    HomeScreen()
                }
            }
        }
    }
}

private val timeFormat = SimpleDateFormat("HH:mm:ss", Locale.getDefault())

private fun isScreenCaptureEnabled(context: android.content.Context): Boolean {
    val enabled = Settings.Secure.getString(
        context.contentResolver,
        Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
    ) ?: return false
    return enabled.contains("${context.packageName}/") && enabled.contains("ScreenContentService")
}

private fun isNotificationAccessEnabled(context: android.content.Context): Boolean =
    NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName)

@Composable
private fun HomeScreen() {
    val context = LocalContext.current
    val running by LoggerService.isRunning.collectAsStateWithLifecycle()
    val timeline by LogRepository.timeline.collectAsStateWithLifecycle()
    val partial by LogRepository.partial.collectAsStateWithLifecycle()
    val status by LogRepository.status.collectAsStateWithLifecycle()
    val summaries by LogRepository.summaries.collectAsStateWithLifecycle()
    val conversations by LogRepository.conversations.collectAsStateWithLifecycle()

    // 특수 권한(사용 정보 접근, 접근성)은 설정에서 켜고 돌아온다. 화면이 다시 보일 때 다시 확인한다.
    var usageGranted by remember { mutableStateOf(UsageAccess.isGranted(context)) }
    var screenGranted by remember { mutableStateOf(isScreenCaptureEnabled(context)) }
    var notifGranted by remember { mutableStateOf(isNotificationAccessEnabled(context)) }
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                usageGranted = UsageAccess.isGranted(context)
                screenGranted = isScreenCaptureEnabled(context)
                notifGranted = isNotificationAccessEnabled(context)
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { result ->
        if (result[Manifest.permission.RECORD_AUDIO] == true) {
            LoggerService.start(context)
        }
    }

    val coroutineScope = rememberCoroutineScope()
    var showDeleteDialog by remember { mutableStateOf(false) }
    var endpointUrl by remember { mutableStateOf(EndpointConfig.url(context) ?: "") }
    var endpointToken by remember { mutableStateOf(EndpointConfig.token(context) ?: "") }
    val tagInputs = remember { mutableStateMapOf<Long, String>() }
    var llmUrl by remember { mutableStateOf(LlmConfig.baseUrl(context)) }
    var llmModel by remember { mutableStateOf(LlmConfig.model(context)) }
    var llmKey by remember { mutableStateOf(LlmConfig.apiKey(context) ?: "") }

    if (showDeleteDialog) {
        AlertDialog(
            onDismissRequest = { showDeleteDialog = false },
            title = { Text("전체 삭제") },
            text = { Text("이 기기에 쌓인 모든 기록을 지웁니다. 되돌릴 수 없습니다.") },
            confirmButton = {
                TextButton(onClick = {
                    LogRepository.deleteAll()
                    showDeleteDialog = false
                }) { Text("삭제") }
            },
            dismissButton = {
                TextButton(onClick = { showDeleteDialog = false }) { Text("취소") }
            },
        )
    }

    Column(
        modifier = Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            "LifeLog",
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.padding(top = 16.dp),
        )
        Text(
            if (running) "기록 중 — 이 기기에만 저장됩니다" else "멈춤",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (status.isNotEmpty()) {
            Text(status, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }

        Button(onClick = {
            if (running) {
                LoggerService.stop(context)
            } else {
                val needed = buildList {
                    if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO)
                        != PackageManager.PERMISSION_GRANTED
                    ) add(Manifest.permission.RECORD_AUDIO)
                    if (ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION)
                        != PackageManager.PERMISSION_GRANTED
                    ) add(Manifest.permission.ACCESS_FINE_LOCATION)
                    for (p in listOf(Manifest.permission.READ_CALL_LOG, Manifest.permission.READ_SMS, Manifest.permission.READ_CONTACTS, Manifest.permission.READ_CALENDAR)) {
                        if (ContextCompat.checkSelfPermission(context, p) != PackageManager.PERMISSION_GRANTED) add(p)
                    }
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED
                    ) add(Manifest.permission.POST_NOTIFICATIONS)
                }
                if (needed.isEmpty()) LoggerService.start(context)
                else permissionLauncher.launch(needed.toTypedArray())
            }
        }) {
            Text(if (running) "정지" else "기록 시작")
        }

        if (!usageGranted) {
            OutlinedButton(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text("앱 사용 기록 켜기 (사용 정보 접근)") }
        }

        if (!screenGranted) {
            OutlinedButton(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text("화면 내용 기록 켜기 (접근성)") }
        }

        if (!notifGranted) {
            OutlinedButton(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text("알림 기록 켜기 (알림 접근)") }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = {
                coroutineScope.launch {
                    val file = LogRepository.exportToFile(context)
                    val uri = FileProvider.getUriForFile(
                        context, "${context.packageName}.fileprovider", file,
                    )
                    val send = Intent(Intent.ACTION_SEND).apply {
                        type = "application/json"
                        putExtra(Intent.EXTRA_STREAM, uri)
                        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    }
                    context.startActivity(
                        Intent.createChooser(send, "LifeLog 내보내기")
                            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    )
                }
            }) { Text("내보내기 (JSON)") }
            OutlinedButton(onClick = {
                coroutineScope.launch { LogRepository.exportIncremental(context) }
            }) { Text("폴더로 (증분)") }
            OutlinedButton(onClick = { showDeleteDialog = true }) { Text("전체 삭제") }
        }

        OutlinedButton(onClick = {
            coroutineScope.launch {
                when (val r = LogRepository.summarizeRecent(context, 24L * 60 * 60 * 1000)) {
                    is Summarizer.SummaryResult.Ok -> LogRepository.setStatus("요약 완료")
                    is Summarizer.SummaryResult.Unavailable -> LogRepository.setStatus("요약 불가: ${r.reason}")
                }
            }
        }) { Text("지금 요약 (최근 24시간)") }

        OutlinedButton(onClick = {
            coroutineScope.launch {
                val n = LogRepository.buildConversations(context)
                LogRepository.setStatus("대화 묶기: ${n}건")
            }
        }) { Text("지금 대화 묶기") }

        if (summaries.isNotEmpty()) {
            Text("최근 요약", style = MaterialTheme.typography.labelLarge)
            Text(summaries.first().text, style = MaterialTheme.typography.bodyMedium)
        }

        if (running && partial.isNotBlank()) {
            Text(
                partial,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        Text("HTTP 내보내기", style = MaterialTheme.typography.labelLarge)
        OutlinedTextField(
            value = endpointUrl,
            onValueChange = { endpointUrl = it },
            label = { Text("엔드포인트 URL") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = endpointToken,
            onValueChange = { endpointToken = it },
            label = { Text("토큰 (선택)") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = {
                EndpointConfig.set(context, endpointUrl, endpointToken)
                LogRepository.setStatus("엔드포인트 저장됨")
            }) { Text("저장") }
            OutlinedButton(onClick = {
                coroutineScope.launch {
                    val n = LogRepository.pushToHttp(context)
                    LogRepository.setStatus(
                        when (n) {
                            -1 -> "엔드포인트 미설정"
                            -2 -> "전송 실패"
                            else -> "전송됨: ${n}건"
                        },
                    )
                }
            }) { Text("지금 전송") }
        }

        Text("요약 LLM (클라우드, 무료 티어 키)", style = MaterialTheme.typography.labelLarge)
        OutlinedTextField(
            value = llmUrl,
            onValueChange = { llmUrl = it },
            label = { Text("베이스 URL") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = llmModel,
            onValueChange = { llmModel = it },
            label = { Text("모델") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = llmKey,
            onValueChange = { llmKey = it },
            label = { Text("API 키") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedButton(onClick = {
            LlmConfig.set(context, llmUrl, llmModel, llmKey)
            LogRepository.setStatus("요약 LLM 저장됨")
        }) { Text("요약 LLM 저장") }

        val pending = conversations.filter { it.status == "pending" }
        if (pending.isNotEmpty()) {
            Text("태그할 대화 (${pending.size})", style = MaterialTheme.typography.labelLarge)
            pending.take(10).forEach { conv ->
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(
                        timeFormat.format(Date(conv.startMs)) + "\u2013" + timeFormat.format(Date(conv.endMs)) +
                            if (conv.context.isNotEmpty()) " \u00b7 ${conv.context}" else "",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedTextField(
                            value = tagInputs[conv.id] ?: "",
                            onValueChange = { tagInputs[conv.id] = it },
                            label = { Text("누구와") },
                            singleLine = true,
                            modifier = Modifier.weight(1f),
                        )
                        OutlinedButton(onClick = {
                            LogRepository.tagConversation(conv.id, tagInputs[conv.id].orEmpty())
                            tagInputs.remove(conv.id)
                        }) { Text("저장") }
                    }
                }
            }
        }

        HorizontalDivider()

        if (timeline.isEmpty()) {
            Text(
                "아직 기록이 없습니다. 기록을 시작하고 말하거나 앱을 써 보세요.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 8.dp),
            )
        } else {
            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                timeline.take(200).forEach { item -> TimelineRow(item) }
            }
        }
    }
}

private fun itemKey(item: TimelineItem): String = when (item) {
    is TimelineItem.Voice -> "v${item.seg.id}"
    is TimelineItem.App -> "a${item.event.id}"
    is TimelineItem.Screen -> "s${item.event.id}"
    is TimelineItem.Location -> "l${item.point.id}"
    is TimelineItem.Notification -> "n${item.event.id}"
    is TimelineItem.Comm -> "c${item.event.id}"
    is TimelineItem.Calendar -> "k${item.entry.id}"
}

@Composable
private fun TimelineRow(item: TimelineItem) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            timeFormat.format(Date(item.timestampMs)),
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        when (item) {
            is TimelineItem.Voice -> Text(item.seg.text, style = MaterialTheme.typography.bodyLarge)
            is TimelineItem.App -> Text(
                "▸ ${item.event.label}",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            is TimelineItem.Screen -> Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    "▦ ${item.event.label}",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    item.event.text.take(140),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
            is TimelineItem.Location -> Text(
                "◎ %.5f, %.5f (±%.0fm)".format(
                    item.point.latitude, item.point.longitude, item.point.accuracyM,
                ),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            is TimelineItem.Notification -> Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    "✉ " + item.event.label,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    listOf(item.event.title, item.event.text).filter { it.isNotEmpty() }
                        .joinToString(" — ").take(140),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
            is TimelineItem.Comm -> {
                val e = item.event
                val who = e.partyName ?: e.party
                val dir = when (e.direction) {
                    "in" -> "수신"; "out" -> "발신"; "missed" -> "부재중"; else -> ""
                }
                val icon = if (e.kind == "call") "☎" else "✉"
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        "$icon $who",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        if (e.kind == "call") dir else "$dir: ${e.content}".take(140),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }
            is TimelineItem.Calendar -> Text(
                "📅 ${item.entry.title}" + if (item.entry.location.isNotEmpty()) " @${item.entry.location}" else "",
                style = MaterialTheme.typography.bodyMedium,
            )
        }
    }
}
