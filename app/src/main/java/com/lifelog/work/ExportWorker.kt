package com.lifelog.work

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkerParameters
import androidx.work.WorkManager
import com.lifelog.data.LogRepository
import java.util.concurrent.TimeUnit

/**
 * 하루 한 번 증분 내보내기를 돌린다. 지난 이후 새 기록만 앱 전용 외부 폴더에 NDJSON 으로 append.
 */
class ExportWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = try {
        LogRepository.exportIncremental(applicationContext)
        LogRepository.pushToHttp(applicationContext)
        Result.success()
    } catch (e: Exception) {
        Result.retry()
    }

    companion object {
        private const val WORK_NAME = "daily-export"

        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<ExportWorker>(1, TimeUnit.DAYS).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                WORK_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                request,
            )
        }
    }
}
