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
 * 보존 정리. 하루 한 번, 이미 내보냈고 보존 기간([RETENTION_DAYS]일)보다 오래된 원본을 지운다.
 * 장기 보관은 내보낸 쪽(KG)이 맡고, 기기엔 최근 구간만 남긴다. 요약은 지우지 않는다.
 */
class PruneWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = try {
        LogRepository.pruneExportedOlderThan(applicationContext, TimeUnit.DAYS.toMillis(RETENTION_DAYS))
        Result.success()
    } catch (e: Exception) {
        Result.retry()
    }

    companion object {
        const val RETENTION_DAYS = 30L
        private const val WORK_NAME = "daily-prune"

        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<PruneWorker>(1, TimeUnit.DAYS).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                WORK_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                request,
            )
        }
    }
}
