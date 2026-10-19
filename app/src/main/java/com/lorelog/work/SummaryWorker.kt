package com.lorelog.work

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkerParameters
import androidx.work.WorkManager
import com.lorelog.data.LogRepository
import com.lorelog.summary.SummaryPrefs
import java.util.concurrent.TimeUnit

/**
 * 하루 한 번 최근 24시간 기록을 온디바이스 요약 엔진으로 요약한다.
 * 미지원 기기면 요약 없이 넘어가고(흐름 유지), 지원 기기에서 요약이 쌓인다.
 */
class SummaryWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = try {
        if (SummaryPrefs.enabled(applicationContext)) {
            LogRepository.summarizeRecent(applicationContext, TimeUnit.HOURS.toMillis(24))
        }
        Result.success()
    } catch (e: Exception) {
        Result.retry()
    }

    companion object {
        private const val WORK_NAME = "daily-summary"

        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<SummaryWorker>(1, TimeUnit.DAYS).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                WORK_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                request,
            )
        }
    }
}
