package com.lifelog.work

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkerParameters
import androidx.work.WorkManager
import com.lifelog.collect.CallRecordingImporter
import java.util.concurrent.TimeUnit

/** 지정한 폴더의 새 통화 녹음을 주기적으로(2시간) 전사해 가져온다. 폴더 미지정이면 아무것도 안 함. */
class CallRecordingWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = try {
        CallRecordingImporter.importNew(applicationContext)
        Result.success()
    } catch (e: Exception) {
        Result.retry()
    }

    companion object {
        private const val WORK_NAME = "callrec-import"

        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<CallRecordingWorker>(2, TimeUnit.HOURS).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                WORK_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                request,
            )
        }
    }
}
