package com.lorelog

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
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
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
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.FileProvider
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import com.lorelog.collect.UsageAccess
import com.lorelog.data.LogRepository
import com.lorelog.collect.CallRecordingImporter
import com.lorelog.collect.PendingVoice
import com.lorelog.collect.AudioPrefs
import com.lorelog.audio.VoiceAudio
import com.lorelog.audio.VoicePlayer
import java.io.File
import com.lorelog.collect.CollectorPrefs
import com.lorelog.data.TimelineItem
import com.lorelog.net.EndpointConfig
import com.lorelog.net.LlmConfig
import com.lorelog.net.TranscribeConfig
import com.lorelog.net.UpdateChecker
import com.lorelog.net.WhisperModels
import com.lorelog.stt.ModelDownloader
import com.lorelog.stt.WhisperEngine
import com.lorelog.net.ModelCatalog
import com.lorelog.summary.Summarizer
import com.lorelog.summary.SummaryPrefs
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
    val llmUrl = LlmConfig.baseUrl(context)
    var llmKey by remember { mutableStateOf(LlmConfig.apiKey(context) ?: "") }
    var showKey by remember { mutableStateOf(false) }
    var keyChecking by remember { mutableStateOf(false) }
    var txEngine by remember { mutableStateOf(TranscribeConfig.engine(context)) }
    var whisperId by remember { mutableStateOf(TranscribeConfig.modelId(context)) }
    val dlState by ModelDownloader.state.collectAsStateWithLifecycle()
    var update by remember { mutableStateOf<UpdateChecker.Result?>(null) }
    var checking by remember { mutableStateOf(false) }
    var btMic by remember { mutableStateOf(AudioPrefs.btMic(context)) }
    var micClean by remember { mutableStateOf(AudioPrefs.micClean(context)) }
    var summaryOn by remember { mutableStateOf(SummaryPrefs.enabled(context)) }
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
                    LogRepository.deleteAll(context)
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
            Text("LoreLog", style = MaterialTheme.typography.headlineMedium)
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
        Text("통화 녹음 가져오기", style = MaterialTheme.typography.labelLarge)
        Text(
            "라이브 통화는 OS가 막습니다. 전화앱 설정에서 '통화 자동 녹음'을 켜 두고, " +
                "그 저장 폴더(예: 내장메모리 Recordings/Call)를 지정하면 새 녹음을 전사합니다. " +
                "전사는 아래 '음성 전사 방식'에서 고른 엔진(Gemini/온디바이스)을 씁니다. 15분마다 새 녹음을 확인합니다.",
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

        Text("내보내기 / 삭제", style = MaterialTheme.typography.labelLarge)
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
            }) { Text("JSON 파일로 공유") }
            OutlinedButton(onClick = {
                LogRepository.setStatus("폴더에 쓰는 중…")
                coroutineScope.launch {
                    val n = LogRepository.exportIncremental(context)
                    LogRepository.setStatus(if (n > 0) "폴더에 새 기록 ${n}줄 추가됨" else "폴더에 추가할 새 기록 없음")
                }
            }) { Text("폴더에 새 기록 추가") }
            OutlinedButton(onClick = { showDeleteDialog = true }) { Text("전체 삭제") }
        }
        Text(
            "JSON 공유 = 전체 기록을 파일 하나로 다른 앱에 보냄(복사본, 커서 안 움직임). " +
                "폴더 추가 = 지난번 이후 새 기록만 Android/data/com.lorelog/files/exports/ 에 NDJSON 으로 이어 씀(매일 자동, 여기선 즉시). " +
                "폴더·HTTP 로 내보낸 기록은 보관 오디오가 지워지고 30일 뒤 정리 대상이 됩니다.",
            style = MaterialTheme.typography.bodySmall,
        )

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

        Text("Gemini API 키", style = MaterialTheme.typography.labelLarge)
        Text(
            "클라우드 전사와 요약이 함께 쓰는 키입니다. 입력하면 바로 저장됩니다. " +
                "AI Studio 무료 키면 됩니다(Flash 계열 무료, 한도는 모델별).",
            style = MaterialTheme.typography.bodySmall,
        )
        OutlinedTextField(
            value = llmKey,
            onValueChange = { llmKey = it; LlmConfig.setKey(context, it) },
            label = { Text("API 키") },
            singleLine = true,
            visualTransformation = if (showKey) VisualTransformation.None else PasswordVisualTransformation(),
            trailingIcon = { TextButton(onClick = { showKey = !showKey }) { Text(if (showKey) "숨김" else "표시") } },
            modifier = Modifier.fillMaxWidth(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            OutlinedButton(
                enabled = !keyChecking && llmKey.isNotBlank(),
                onClick = {
                    keyChecking = true
                    LogRepository.setStatus("키 확인 중…")
                    coroutineScope.launch {
                        val n = ModelCatalog.fetch(llmUrl, llmKey).size
                        keyChecking = false
                        LogRepository.setStatus(if (n > 0) "✓ 키 유효 — 모델 ${n}개 사용 가능" else "✗ 키 또는 URL 이 틀렸습니다")
                    }
                },
            ) { Text(if (keyChecking) "확인 중…" else "키 확인") }
            TextButton(onClick = {
                context.startActivity(
                    Intent(Intent.ACTION_VIEW, Uri.parse("https://aistudio.google.com/apikey"))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text("키 발급 (무료)") }
        }

        Text("요약", style = MaterialTheme.typography.labelLarge)
        Text(
            "선택 기능입니다. 끄면 원본만 모으고 내보냅니다(권장 — 요약은 지식그래프 쪽에서). " +
                "켜면 하루 한 번 최근 24시간을 요약해 앱에서 보여 주고 내보내기에도 포함합니다.",
            style = MaterialTheme.typography.bodySmall,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("요약 사용", style = MaterialTheme.typography.bodyMedium)
            Switch(
                checked = summaryOn,
                onCheckedChange = {
                    summaryOn = it
                    SummaryPrefs.setEnabled(context, it)
                },
            )
        }
        if (summaryOn) {
            Text(
                "모델은 자동입니다. 기본 ${LlmConfig.DEFAULT_MODEL}, 한도가 차면 다른 Flash 모델로 넘어갑니다.",
                style = MaterialTheme.typography.bodySmall,
            )
            OutlinedButton(onClick = {
                LogRepository.setStatus("요약 중…")
                coroutineScope.launch {
                    when (val r = LogRepository.summarizeRecent(context, 24L * 60 * 60 * 1000)) {
                        is Summarizer.SummaryResult.Ok -> LogRepository.setStatus("요약 완료")
                        is Summarizer.SummaryResult.Unavailable -> LogRepository.setStatus("요약 불가: ${r.reason}")
                    }
                }
            }) { Text("지금 요약 (최근 24시간)") }
        }

        Text("버전 / 업데이트", style = MaterialTheme.typography.labelLarge)
        Text("현재 버전 ${UpdateChecker.currentVersion(context)}", style = MaterialTheme.typography.bodySmall)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            OutlinedButton(
                enabled = !checking,
                onClick = {
                    checking = true
                    LogRepository.setStatus("업데이트 확인 중…")
                    coroutineScope.launch {
                        val r = UpdateChecker.check(context)
                        update = r
                        checking = false
                        LogRepository.setStatus(
                            when {
                                r == null -> "업데이트 확인 실패(네트워크·제한)"
                                r.newer -> "새 버전 ${r.latest} 있음 (현재 ${r.current})"
                                else -> "최신 버전입니다 (${r.current})"
                            },
                        )
                    }
                },
            ) { Text(if (checking) "확인 중…" else "업데이트 확인") }
            update?.let { u ->
                if (u.newer) {
                    Button(onClick = {
                        context.startActivity(
                            Intent(Intent.ACTION_VIEW, Uri.parse(u.apkUrl ?: u.pageUrl))
                                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                        )
                    }) { Text("${u.latest} 받기") }
                }
            }
        }

        Text("음성 전사 방식", style = MaterialTheme.typography.labelLarge)
        Text(
            "녹음은 VAD로 말할 때만, 화면 꺼져도 유지됩니다. 전사 방식을 고르세요. " +
                "준비가 안 됐으면 말소리는 음성파일(voice_pending)로 쌓였다가 준비되면 자동 전사됩니다.",
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
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("마이크 보정 (잡음 억제 · 자동 음량)", style = MaterialTheme.typography.bodyMedium)
            Switch(
                checked = micClean,
                onCheckedChange = {
                    micClean = it
                    AudioPrefs.setMicClean(context, it)
                    LoggerService.reapply(context)
                },
            )
        }
        Text(
            "기기 내장 효과(NoiseSuppressor·AutomaticGainControl)를 녹음에 붙입니다. 배경 소음이 줄고 멀리서 하는 말이 커집니다. " +
                "소리가 이상하게 뭉개지면 끄세요. 기기마다 지원이 다릅니다.",
            style = MaterialTheme.typography.bodySmall,
        )
        EngineOption(
            selected = txEngine == TranscribeConfig.Engine.GEMINI,
            title = "클라우드 · Gemini",
            desc = "위 'Gemini API 키' 사용, 모델은 따로 선택. 무료 티어 가능. 소리가 구글로 감.",
        ) {
            txEngine = TranscribeConfig.Engine.GEMINI
            TranscribeConfig.setEngine(context, txEngine)
            coroutineScope.launch { WhisperEngine.release() }
            LoggerService.reapply(context)
        }
        EngineOption(
            selected = txEngine == TranscribeConfig.Engine.WHISPER,
            title = "온디바이스 · whisper.cpp",
            desc = "기기 안에서 전사(소리 안 나감). 모델 다운로드 필요.",
        ) {
            txEngine = TranscribeConfig.Engine.WHISPER
            TranscribeConfig.setEngine(context, txEngine)
            LoggerService.reapply(context)
        }

        when (txEngine) {
            TranscribeConfig.Engine.GEMINI -> {
                Text(
                    if (LlmConfig.isConfigured(context)) {
                        "✓ Gemini 키 있음 — 바로 전사됩니다."
                    } else {
                        "⚠ 위 'Gemini API 키'가 비어 있어 전사가 안 됩니다. 키를 넣으세요."
                    },
                    style = MaterialTheme.typography.bodySmall,
                )
                Text(
                    "모델은 자동입니다. ${TranscribeConfig.DEFAULT_GEMINI_MODEL} 로 받아쓰고, 하루 한도(모델당 500회)가 차면 " +
                        "${TranscribeConfig.GEMINI_FALLBACKS.joinToString(" → ")} 순으로 넘어갑니다. 실패 사유는 상태줄에 표시됩니다.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            TranscribeConfig.Engine.WHISPER -> {
                Text("정확도 — 클수록 정확·느림", style = MaterialTheme.typography.bodySmall)
                WhisperModels.ALL.forEach { m ->
                    val ready = WhisperModels.isReady(context, m.id)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        RadioButton(
                            selected = whisperId == m.id,
                            onClick = {
                                whisperId = m.id
                                TranscribeConfig.setModelId(context, m.id)
                                LoggerService.reapply(context)
                            },
                        )
                        Column {
                            Text(m.label, style = MaterialTheme.typography.bodyMedium)
                            Text(if (ready) "다운로드됨" else "다운로드 필요", style = MaterialTheme.typography.bodySmall)
                        }
                    }
                }
                val selReady = WhisperModels.isReady(context, whisperId)
                val dl = dlState
                val downloadingThis = dl is ModelDownloader.State.Downloading && dl.id == whisperId
                OutlinedButton(
                    enabled = !downloadingThis && !selReady,
                    onClick = {
                        coroutineScope.launch {
                            val ok = ModelDownloader.download(context, whisperId)
                            LogRepository.setStatus(if (ok) "모델 다운로드 완료" else "모델 다운로드 실패")
                            LoggerService.reapply(context)
                        }
                    },
                ) {
                    Text(
                        when {
                            selReady -> "다운로드됨"
                            downloadingThis -> "다운로드 중…"
                            else -> "모델 다운로드 (~${WhisperModels.byId(whisperId).approxMb}MB)"
                        },
                    )
                }
                when (dl) {
                    is ModelDownloader.State.Downloading -> if (dl.id == whisperId) {
                        Text("다운로드 중… ${dl.pct}%", style = MaterialTheme.typography.bodySmall)
                    }
                    is ModelDownloader.State.Failed -> Text("실패: ${dl.reason}", style = MaterialTheme.typography.bodySmall)
                    else -> {}
                }
                Text(
                    "arm64 기기만 지원. 첫 전사에서 모델 로딩에 수 초 걸릴 수 있습니다.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        OutlinedButton(onClick = {
            LogRepository.setStatus("밀린 음성 전사 중…")
            coroutineScope.launch {
                val done = PendingVoice.transcribePending(context)
                LogRepository.setStatus(
                    if (done > 0) "밀린 음성 ${done}건 전사됨" else "전사할 밀린 음성이 없거나 전사 준비가 안 됐습니다",
                )
            }
        }) { Text("밀린 음성 전사 (${PendingVoice.pendingCount(context)}건 대기)") }
        OutlinedButton(onClick = {
            coroutineScope.launch {
                val n = LogRepository.buildConversations(context)
                LogRepository.setStatus("대화 묶기: ${n}건")
            }
        }) { Text("대화 묶기 지금 실행") }
        Text(
            "음성 조각을 5분 간격으로 대화로 묶고 겹치는 통화·일정으로 참석자를 붙입니다. 15분마다 자동으로 돌며, 못 붙인 대화는 '태그할 대화'에 올라옵니다.",
            style = MaterialTheme.typography.bodySmall,
        )

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
private fun EngineOption(
    selected: Boolean,
    title: String,
    desc: String,
    onSelect: () -> Unit,
) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        RadioButton(selected = selected, onClick = onSelect)
        Column {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            Text(desc, style = MaterialTheme.typography.bodySmall)
        }
    }
}

private fun itemKey(item: TimelineItem): String = when (item) {
    is TimelineItem.Voice -> "v${item.seg.id}"
    is TimelineItem.Pending -> "p${item.audio.path}"
    is TimelineItem.App -> "a${item.event.id}"
    is TimelineItem.Screen -> "s${item.event.id}"
    is TimelineItem.Location -> "l${item.point.id}"
    is TimelineItem.Notification -> "n${item.event.id}"
    is TimelineItem.Comm -> "c${item.event.id}"
    is TimelineItem.Calendar -> "k${item.entry.id}"
}

/** 음성 조각 옆 "▶ 듣기 / ■ 정지" 버튼. 보관 오디오가 있을 때만 그린다. */
@Composable
private fun PlayButton(file: File, text: String? = null) {
    val playing by VoicePlayer.playing.collectAsStateWithLifecycle()
    val isThis = playing == file.absolutePath
    TextButton(onClick = { VoicePlayer.toggle(file, text) }) {
        Text(if (isThis) "■ 정지" else "▶ 듣기", style = MaterialTheme.typography.labelMedium)
    }
}

@Composable
private fun TimelineRow(item: TimelineItem) {
    val context = LocalContext.current
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
            is TimelineItem.Voice -> Row(verticalAlignment = Alignment.Top, modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    if (item.seg.speaker != null) {
                        Text(item.seg.speaker, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Text(item.seg.text, style = MaterialTheme.typography.bodyLarge)
                }
                item.seg.audio?.let { PlayButton(VoiceAudio.file(context, it), item.seg.text) }
            }
            is TimelineItem.Pending -> Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Text(
                    "⏳ 전사 대기 · ${VoiceAudio.formatDuration(item.audio.durationMs)}",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f),
                )
                PlayButton(File(item.audio.path))
            }
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
