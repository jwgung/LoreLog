package com.lifelog

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
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
import com.lifelog.collect.CallRecordingImporter
import com.lifelog.collect.AudioPrefs
import com.lifelog.collect.CollectorPrefs
import com.lifelog.data.TimelineItem
import com.lifelog.net.EndpointConfig
import com.lifelog.net.LlmConfig
import com.lifelog.net.SttConfig
import com.lifelog.speaker.SpeakerId
import com.lifelog.net.ModelCatalog
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
    val collectorOn = remember {
        mutableStateMapOf<String, Boolean>().apply { CollectorPrefs.ALL.forEach { put(it.first, CollectorPrefs.enabled(context, it.first)) } }
    }
    var llmUrl by remember { mutableStateOf(LlmConfig.baseUrl(context)) }
    var llmModel by remember { mutableStateOf(LlmConfig.model(context)) }
    var llmKey by remember { mutableStateOf(LlmConfig.apiKey(context) ?: "") }
    var sttUrl by remember { mutableStateOf(SttConfig.baseUrl(context)) }
    var sttModel by remember { mutableStateOf(SttConfig.model(context)) }
    var sttKey by remember { mutableStateOf(SttConfig.apiKey(context) ?: "") }
    var btMic by remember { mutableStateOf(AudioPrefs.btMic(context)) }
    var eagleKey by remember { mutableStateOf(SpeakerId.accessKey(context) ?: "") }
    var enrolled by remember { mutableStateOf(SpeakerId.isEnrolled(context)) }
    var callFolder by remember { mutableStateOf(CallRecordingImporter.folder(context)?.toString()) }
    val callFolderPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocumentTree(),
    ) { uri ->
        if (uri != null) {
            CallRecordingImporter.setFolder(context, uri)
            callFolder = uri.toString()
            LogRepository.setStatus("통화 녹음 폴더 지정됨")
        }
    }

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

    var showSettings by remember { mutableStateOf(false) }
    var hideNotif by remember { mutableStateOf(NotifPrefs.hide(context)) }
    var overlayBadge by remember { mutableStateOf(NotifPrefs.overlay(context)) }

    Column(
        modifier = Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(top = 16.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("LifeLog", style = MaterialTheme.typography.headlineMedium)
            TextButton(onClick = { showSettings = !showSettings }) {
                Text(if (showSettings) "닫기" else "설정")
            }
        }
        Text(
            if (running) "기록 중 — 이 기기에만 저장됩니다" else "멈춤",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (status.isNotEmpty()) {
            Text(status, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }

        if (!screenGranted && CollectorPrefs.enabled(context, CollectorPrefs.SCREEN)) {
            Surface(
                color = MaterialTheme.colorScheme.errorContainer,
                shape = MaterialTheme.shapes.medium,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(
                    modifier = Modifier.padding(12.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Text(
                        "화면 수집이 꺼져 있습니다",
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                    )
                    Text(
                        "접근성이 꺼지면 앱 안에서 본 내용이 안 쌓입니다. 새 버전 설치나 절전(앱 재우기)으로 꺼지기도 합니다.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = {
                            context.startActivity(
                                Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
                                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                            )
                        }) { Text("접근성 켜기") }
                        OutlinedButton(onClick = {
                            context.startActivity(
                                Intent(
                                    Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                                    Uri.parse("package:${context.packageName}"),
                                ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                            )
                        }) { Text("앱 정보") }
                    }
                    Text(
                        "토글이 회색이면: 앱 정보 → ⋮ → '제한된 설정 허용'. 덜 꺼지게 하려면 배터리를 '제한 없음'으로.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                    )
                }
            }
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

        if (running && partial.isNotBlank()) {
            Text(partial, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (summaries.isNotEmpty()) {
            Text("최근 요약", style = MaterialTheme.typography.labelLarge)
            Text(summaries.first().text, style = MaterialTheme.typography.bodyMedium)
        }


        if (showSettings) {
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
            Text(
                "토글이 회색이면: 앱 정보 → 우상단 ⋮ → '제한된 설정 허용'을 먼저 누르세요. " +
                    "플레이스토어 밖에서 설치한 앱이라 OS가 접근성을 막아둔 것입니다.",
                style = MaterialTheme.typography.bodySmall,
            )
            TextButton(onClick = {
                context.startActivity(
                    Intent(
                        Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        Uri.parse("package:${context.packageName}"),
                    ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text("앱 정보 열기 (제한된 설정 허용)") }
        }

        if (!notifGranted) {
            OutlinedButton(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text("알림 기록 켜기 (알림 접근)") }
        }

        Text("수집 항목", style = MaterialTheme.typography.labelLarge)
        CollectorPrefs.ALL.forEach { (key, label) ->
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(label, style = MaterialTheme.typography.bodyMedium)
                Switch(
                    checked = collectorOn[key] ?: true,
                    onCheckedChange = {
                        collectorOn[key] = it
                        CollectorPrefs.set(context, key, it)
                        LoggerService.reapply(context)
                    },
                )
            }
        }

        Text("알림 / 표시", style = MaterialTheme.typography.labelLarge)
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("알림 숨기기 (가능한 만큼)", style = MaterialTheme.typography.bodyMedium)
            Switch(
                checked = hideNotif,
                onCheckedChange = {
                    hideNotif = it
                    NotifPrefs.setHide(context, it)
                    LoggerService.reapply(context)
                    LoggerService.showIdle(context)
                },
            )
        }
        Text(
            "멈춤 알림은 숨겨집니다. 기록 중 알림은 OS 규칙상 완전히 못 숨기고 최소화만 됩니다" +
                "(상태바 아이콘 없이 알림창 맨 아래로).",
            style = MaterialTheme.typography.bodySmall,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("플로팅 '감시중' 배지 (다른 앱 위에)", style = MaterialTheme.typography.bodyMedium)
            Switch(
                checked = overlayBadge,
                onCheckedChange = {
                    overlayBadge = it
                    NotifPrefs.setOverlay(context, it)
                    if (it && !Settings.canDrawOverlays(context)) {
                        context.startActivity(
                            Intent(
                                Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                                Uri.parse("package:${context.packageName}"),
                            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                        )
                    }
                    LoggerService.reapply(context)
                },
            )
        }
        TextButton(onClick = {
            context.startActivity(
                Intent(
                    Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                    Uri.parse("package:${context.packageName}"),
                ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }) { Text("다른 앱 위에 표시 허용") }

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
        ModelPicker(
            label = "모델",
            baseUrl = llmUrl,
            apiKey = llmKey,
            value = llmModel,
            onValue = { llmModel = it },
            filter = {
                !it.contains("whisper", true) && !it.contains("embedding", true) &&
                    !it.contains("tts", true) && !it.contains("imagen", true) &&
                    !it.contains("veo", true) && !it.contains("aqa", true)
            },
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
        TextButton(onClick = {
            context.startActivity(
                Intent(Intent.ACTION_VIEW, Uri.parse("https://aistudio.google.com/apikey"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }) { Text("Gemini 키 발급 (무료)") }

        Text("음성 전사 (Groq)", style = MaterialTheme.typography.labelLarge)
        Text(
            "음성은 녹음→Groq 전사로만 수집합니다(VAD로 말할 때만, 화면 꺼져도 유지). 키가 없으면 음성은 쉽니다.",
            style = MaterialTheme.typography.bodySmall,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("블루투스 이어폰 마이크로 녹음", style = MaterialTheme.typography.bodyMedium)
            Switch(
                checked = btMic,
                onCheckedChange = {
                    btMic = it
                    AudioPrefs.setBtMic(context, it)
                    LoggerService.reapply(context)
                },
            )
        }
        OutlinedTextField(value = sttUrl, onValueChange = { sttUrl = it }, label = { Text("베이스 URL") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        ModelPicker(
            label = "모델",
            baseUrl = sttUrl,
            apiKey = sttKey,
            value = sttModel,
            onValue = { sttModel = it },
            filter = { it.contains("whisper", true) },
        )
        OutlinedTextField(value = sttKey, onValueChange = { sttKey = it }, label = { Text("Groq API 키") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedButton(onClick = {
            SttConfig.set(context, sttUrl, sttModel, sttKey)
            LogRepository.setStatus("음성 전사 저장됨")
            LoggerService.reapply(context)
        }) { Text("음성 전사 저장") }
        TextButton(onClick = {
            context.startActivity(
                Intent(Intent.ACTION_VIEW, Uri.parse("https://console.groq.com/keys"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }) { Text("Groq 키 발급 (무료)") }

        Text("통화 녹음 가져오기", style = MaterialTheme.typography.labelLarge)
        Text(
            "라이브 통화는 OS가 막습니다. 전화앱 설정에서 '통화 자동 녹음'을 켜 두고, " +
                "그 저장 폴더(예: 내장메모리 Recordings/Call)를 지정하면 새 녹음을 전사합니다. " +
                "전사는 위 STT(Groq) 키를 씁니다.",
            style = MaterialTheme.typography.bodySmall,
        )
        Text(
            if (callFolder != null) "지정됨: ${Uri.decode(callFolder)}" else "폴더 미지정",
            style = MaterialTheme.typography.bodySmall,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = { callFolderPicker.launch(null) }) { Text("통화 녹음 폴더 선택") }
            OutlinedButton(onClick = {
                LogRepository.setStatus("통화 녹음 가져오는 중…")
                coroutineScope.launch { CallRecordingImporter.importNew(context) }
            }) { Text("지금 가져오기") }
        }

        Text("화자 식별 (내 목소리)", style = MaterialTheme.typography.labelLarge)
        Text(
            "내 목소리를 한 번 등록하면, 녹음+클라우드 조각마다 '나/상대/모름'을 자동으로 붙입니다. " +
                "온디바이스(Picovoice Eagle)라 소리는 기기 밖으로 안 나갑니다.",
            style = MaterialTheme.typography.bodySmall,
        )
        OutlinedTextField(
            value = eagleKey,
            onValueChange = { eagleKey = it },
            label = { Text("Picovoice AccessKey") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Text(
            if (enrolled) "상태: 등록됨" else "상태: 미등록",
            style = MaterialTheme.typography.bodySmall,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = {
                SpeakerId.setAccessKey(context, eagleKey)
                LogRepository.setStatus("AccessKey 저장됨")
            }) { Text("키 저장") }
            OutlinedButton(onClick = {
                SpeakerId.setAccessKey(context, eagleKey)
                LogRepository.setStatus("내 목소리 등록 중… 10~20초 말해주세요 (0%)")
                coroutineScope.launch {
                    val ok = SpeakerId.enroll(context) { pct, fb ->
                        if (pct >= 0) LogRepository.setStatus("등록 중… ${pct.toInt()}% ($fb)")
                        else LogRepository.setStatus("등록 실패: $fb")
                    }
                    if (ok) {
                        enrolled = true
                        LogRepository.setStatus("내 목소리 등록 완료")
                    }
                }
            }) { Text("내 목소리 등록") }
        }
        if (enrolled) {
            TextButton(onClick = {
                SpeakerId.clearEnrollment(context)
                enrolled = false
                LogRepository.setStatus("등록 지움")
            }) { Text("등록 지우기") }
        }
        TextButton(onClick = {
            context.startActivity(
                Intent(Intent.ACTION_VIEW, Uri.parse("https://console.picovoice.ai/"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }) { Text("Picovoice 키 발급 (무료)") }
        }

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

@Composable
private fun ModelPicker(
    label: String,
    baseUrl: String,
    apiKey: String,
    value: String,
    onValue: (String) -> Unit,
    filter: (String) -> Boolean,
) {
    var expanded by remember { mutableStateOf(false) }
    var models by remember { mutableStateOf<List<String>>(emptyList()) }
    var loading by remember { mutableStateOf(false) }
    var msg by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    OutlinedTextField(
        value = value,
        onValueChange = onValue,
        label = { Text(label) },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    Box {
        TextButton(onClick = {
            expanded = true
            if (!loading) {
                loading = true
                msg = null
                scope.launch {
                    val list = ModelCatalog.fetch(baseUrl, apiKey).filter(filter)
                    loading = false
                    models = list
                    msg = if (list.isEmpty()) "목록 없음 — URL·키 확인 후 다시" else null
                }
            }
        }) { Text(if (loading) "불러오는 중…" else "모델 목록에서 고르기 ▾") }
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            if (models.isEmpty()) {
                DropdownMenuItem(
                    text = { Text(msg ?: "불러오는 중…") },
                    onClick = { expanded = false },
                )
            } else {
                models.forEach { m ->
                    DropdownMenuItem(
                        text = { Text(m) },
                        onClick = { onValue(m); expanded = false },
                    )
                }
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
