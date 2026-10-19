package com.lorelog.net

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL

/** 기록을 지정 URL 로 POST 하는 내보내기 대상. 본문은 NDJSON(줄마다 JSON 한 건). */
object HttpSink {
    suspend fun post(url: String, token: String?, body: String): Boolean = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                doOutput = true
                connectTimeout = 15_000
                readTimeout = 20_000
                setRequestProperty("Content-Type", "application/x-ndjson")
                if (!token.isNullOrBlank()) setRequestProperty("Authorization", "Bearer $token")
            }
            conn.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
            conn.responseCode in 200..299
        } catch (e: Exception) {
            false
        } finally {
            conn?.disconnect()
        }
    }
}
