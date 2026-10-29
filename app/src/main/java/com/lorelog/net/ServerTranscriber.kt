package com.lorelog.net

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * 내 서버(gungbuntu)의 음성 인식으로 전사한다. 서버가 tailnet 안의 음성 모델로 넘기므로 소리가 밖으로 나가지 않는다.
 * 내보내기 설정(주소·토큰)을 그대로 쓴다. 서버에 못 닿으면 null(대기 파일이 남아 나중에 다시 시도),
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
