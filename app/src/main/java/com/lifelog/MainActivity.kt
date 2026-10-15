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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.lifelog.collect.UsageAccess
import com.lifelog.data.LogRepository
import com.lifelog.data.TimelineItem
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

@Composable
private fun HomeScreen() {
    val context = LocalContext.current
    val running by LoggerService.isRunning.collectAsStateWithLifecycle()
    val timeline by LogRepository.timeline.collectAsStateWithLifecycle()
    val partial by LogRepository.partial.collectAsStateWithLifecycle()
    val status by LogRepository.status.collectAsStateWithLifecycle()

    // 특수 권한(사용 정보 접근, 접근성)은 설정에서 켜고 돌아온다. 화면이 다시 보일 때 다시 확인한다.
    var usageGranted by remember { mutableStateOf(UsageAccess.isGranted(context)) }
    var screenGranted by remember { mutableStateOf(isScreenCaptureEnabled(context)) }
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                usageGranted = UsageAccess.isGranted(context)
                screenGranted = isScreenCaptureEnabled(context)
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

    Column(
        modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 20.dp),
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

        if (running && partial.isNotBlank()) {
            Text(
                partial,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
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
            LazyColumn(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                items(timeline, key = { itemKey(it) }) { item -> TimelineRow(item) }
            }
        }
    }
}

private fun itemKey(item: TimelineItem): String = when (item) {
    is TimelineItem.Voice -> "v${item.seg.id}"
    is TimelineItem.App -> "a${item.event.id}"
    is TimelineItem.Screen -> "s${item.event.id}"
    is TimelineItem.Location -> "l${item.point.id}"
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
        }
    }
}
