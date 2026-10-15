package com.lifelog.collect

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.CallLog
import android.provider.ContactsContract
import android.provider.Telephony
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
 * 소통 수집기. 통화 기록과 문자를 주기적으로 읽어 누구와 언제 연락했는지 남긴다.
 * 연락처로 번호 → 이름을 맞춘다. 시작 시점 이후의 새 기록만 담는다(과거 전체는 안 가져옴).
 */
class CommsCollector(context: Context) {

    private val appContext = context.applicationContext
    private val cr = appContext.contentResolver
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val nameCache = HashMap<String, String?>()

    private var job: Job? = null
    private var lastCallMs = 0L
    private var lastSmsMs = 0L

    fun start() {
        if (job?.isActive == true) return
        val hasCall = granted(Manifest.permission.READ_CALL_LOG)
        val hasSms = granted(Manifest.permission.READ_SMS)
        if (!hasCall && !hasSms) {
            LogRepository.setStatus("통화·문자 기록을 켜려면 권한이 필요합니다")
            return
        }
        val now = System.currentTimeMillis()
        lastCallMs = now
        lastSmsMs = now
        job = scope.launch {
            while (isActive) {
                if (hasCall) pollCalls()
                if (hasSms) pollSms()
                delay(POLL_INTERVAL_MS)
            }
        }
    }

    fun stop() {
        job?.cancel()
        job = null
    }

    private fun granted(p: String) =
        ContextCompat.checkSelfPermission(appContext, p) == PackageManager.PERMISSION_GRANTED

    private fun pollCalls() {
        val cols = arrayOf(CallLog.Calls.NUMBER, CallLog.Calls.TYPE, CallLog.Calls.DATE, CallLog.Calls.DURATION, CallLog.Calls.CACHED_NAME)
        cr.query(CallLog.Calls.CONTENT_URI, cols, "${CallLog.Calls.DATE} > ?", arrayOf(lastCallMs.toString()), "${CallLog.Calls.DATE} ASC")?.use { c ->
            while (c.moveToNext()) {
                val number = c.getString(0) ?: "?"
                val type = c.getInt(1)
                val date = c.getLong(2)
                val duration = c.getLong(3)
                val cached = c.getString(4)
                val direction = when (type) {
                    CallLog.Calls.INCOMING_TYPE -> "in"
                    CallLog.Calls.OUTGOING_TYPE -> "out"
                    CallLog.Calls.MISSED_TYPE -> "missed"
                    else -> "other"
                }
                LogRepository.addComm("call", direction, number, cached ?: name(number), "${duration}s", date)
                if (date > lastCallMs) lastCallMs = date
            }
        }
    }

    private fun pollSms() {
        val cols = arrayOf(Telephony.Sms.ADDRESS, Telephony.Sms.BODY, Telephony.Sms.DATE, Telephony.Sms.TYPE)
        cr.query(Telephony.Sms.CONTENT_URI, cols, "${Telephony.Sms.DATE} > ?", arrayOf(lastSmsMs.toString()), "${Telephony.Sms.DATE} ASC")?.use { c ->
            while (c.moveToNext()) {
                val address = c.getString(0) ?: "?"
                val body = c.getString(1) ?: ""
                val date = c.getLong(2)
                val type = c.getInt(3)
                val direction = if (type == Telephony.Sms.MESSAGE_TYPE_SENT) "out" else "in"
                LogRepository.addComm("sms", direction, address, name(address), body, date)
                if (date > lastSmsMs) lastSmsMs = date
            }
        }
    }

    private fun name(number: String): String? = nameCache.getOrPut(number) {
        if (!granted(Manifest.permission.READ_CONTACTS)) return@getOrPut null
        val uri = Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number))
        cr.query(uri, arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    }

    companion object {
        private const val POLL_INTERVAL_MS = 30_000L
    }
}
