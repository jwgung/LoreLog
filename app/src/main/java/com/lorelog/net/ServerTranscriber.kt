package com.lorelog.net

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * 금고의 /transcribe 로 전사한다. 금고가 사용자가 고른 음성 모델로 넘긴다. 폰은 전사하지 않는다(#67).
 * 금고 연결 설정(주소·토큰)을 그대로 쓴다. 금고에 못 닿으면 null(대기 파일이 남아 나중에 다시 시도),
 * 말소리가 없었으면 빈 문자열.
 */
object ServerTranscriber {
    fun isReady(context: Context): Boolean = EndpointConfig.isConfigured(context)

    suspend fun transcribe(context: Context, file: File): String? = withContext(Dispatchers.IO) {
        val u = EndpointConfig.url(context) ?: return@withContext null
        var conn: HttpURLConnection? = null
        try {
            val base = URL(u).let { "${it.protocol}://${it.authority}" }
            conn = (URL("$base/transcribe").openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                doOutput = true
                connectTimeout = 15_000
                readTimeout = 300_000
                setRequestProperty("Content-Type", "audio/wav")
                EndpointConfig.token(context)?.let { setRequestProperty("Authorization", "Bearer $it") }
                setFixedLengthStreamingMode(file.length())
            }
            file.inputStream().use { it.copyTo(conn.outputStream) }
            if (conn.responseCode !in 200..299) return@withContext null
            JSONObject(conn.inputStream.bufferedReader().readText()).optString("text", "").trim()
        } catch (e: Exception) {
            null
        } finally {
            conn?.disconnect()
        }
    }
}
