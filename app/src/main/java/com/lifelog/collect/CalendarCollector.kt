package com.lifelog.collect

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.provider.CalendarContract
import androidx.core.content.ContextCompat
import com.lifelog.data.LogRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * 캘린더 수집기. 시스템 캘린더(구글 등 동기화된 전부)를 읽어, 일정이 시작되는 때에
 * 그 일정의 시각 자리로 타임라인에 올린다. 주기적으로 새로고침해 새로 추가된 일정도
 * 시작 때가 되면 들어온다. 특정 캘린더 앱에 묶이지 않는다(CalendarContract 시스템 provider).
 */
class CalendarCollector(context: Context) {

    private val appContext = context.applicationContext
    private val cr = appContext.contentResolver
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private var job: Job? = null
    private var lastBeginMs = 0L

    fun start() {
        if (job?.isActive == true) return
        if (ContextCompat.checkSelfPermission(appContext, Manifest.permission.READ_CALENDAR)
            != PackageManager.PERMISSION_GRANTED
        ) {
            LogRepository.setStatus("캘린더 기록을 켜려면 권한이 필요합니다")
            return
        }
        lastBeginMs = System.currentTimeMillis()
        job = scope.launch {
            while (isActive) {
                poll()
                delay(POLL_INTERVAL_MS)
            }
        }
    }

    fun stop() {
        job?.cancel()
        job = null
    }

    /** 지난 폴 이후 시작된 일정 인스턴스를 그 시작 시각으로 기록한다. */
    private fun poll() {
        val now = System.currentTimeMillis()
        if (now <= lastBeginMs) return
        val builder = CalendarContract.Instances.CONTENT_URI.buildUpon()
        ContentUris.appendId(builder, lastBeginMs)
        ContentUris.appendId(builder, now)
        val cols = arrayOf(
            CalendarContract.Instances.TITLE,
            CalendarContract.Instances.BEGIN,
            CalendarContract.Instances.END,
            CalendarContract.Instances.EVENT_LOCATION,
        )
        cr.query(builder.build(), cols, "${CalendarContract.Instances.BEGIN} > ?", arrayOf(lastBeginMs.toString()), "${CalendarContract.Instances.BEGIN} ASC")?.use { c ->
            while (c.moveToNext()) {
                val title = c.getString(0)?.takeIf { it.isNotBlank() } ?: "(제목 없음)"
                val begin = c.getLong(1)
                val end = c.getLong(2)
                val location = c.getString(3).orEmpty()
                LogRepository.addCalendar(title, location, begin, end)
            }
        }
        lastBeginMs = now
    }

    companion object {
        private const val POLL_INTERVAL_MS = 300_000L // 5분
    }
}
