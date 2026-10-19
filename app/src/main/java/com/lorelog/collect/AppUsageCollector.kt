package com.lorelog.collect

import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.pm.PackageManager
import com.lorelog.data.LogRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * 앱 사용 수집기. UsageStatsManager 를 주기적으로 폴링해 전면 앱이 바뀐 시점을 기록한다.
 * '사용 정보 접근' 특수 권한이 없으면 상태만 알리고 쉰다.
 */
class AppUsageCollector(context: Context) {

    private val appContext = context.applicationContext
    private val usm = appContext.getSystemService(Context.USAGE_STATS_SERVICE) as UsageStatsManager
    private val pm = appContext.packageManager
    private val labelCache = HashMap<String, String>()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    private var job: Job? = null
    private var lastQueryTs = 0L
    private var lastPackage: String? = null

    fun start() {
        if (job?.isActive == true) return
        if (!UsageAccess.isGranted(appContext)) {
            LogRepository.setStatus("앱 사용 기록을 켜려면 '사용 정보 접근' 권한이 필요합니다")
            return
        }
        lastQueryTs = System.currentTimeMillis()
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

    private fun poll() {
        val now = System.currentTimeMillis()
        if (now <= lastQueryTs) return
        val events = usm.queryEvents(lastQueryTs, now)
        val event = UsageEvents.Event()
        while (events.getNextEvent(event)) {
            if (event.eventType != UsageEvents.Event.ACTIVITY_RESUMED) continue
            val pkg = event.packageName ?: continue
            if (pkg == appContext.packageName || pkg == lastPackage) continue
            lastPackage = pkg
            LogRepository.addApp(pkg, label(pkg), event.timeStamp)
        }
        lastQueryTs = now
    }

    private fun label(pkg: String): String = labelCache.getOrPut(pkg) {
        try {
            pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
        } catch (e: PackageManager.NameNotFoundException) {
            pkg
        }
    }

    companion object {
        private const val POLL_INTERVAL_MS = 6_000L
    }
}
