package com.lifelog.net

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/** 오디오 파일을 OpenAI 호환 /audio/transcriptions(Groq Whisper)로 전사한다. 성공 시 텍스트, 실패 시 null. */
object GroqTranscriber {
    suspend fun transcribe(context: Context, file: File): String? = withContext(Dispatchers.IO) {
        val key = SttConfig.apiKey(context) ?: return@withContext null
        val url = SttConfig.baseUrl(context).trimEnd('/') + "/audio/transcriptions"
        val boundary = "----lifelog${System.currentTimeMillis()}"
        var conn: HttpURLConnection? = null
        try {
            conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                doOutput = true
                connectTimeout = 15_000
                readTimeout = 120_000
                setRequestProperty("Authorization", "Bearer $key")
                setRequestProperty("Content-Type", "multipart/form-data; boundary=$boundary")
            }
            conn.outputStream.use { out ->
                fun field(name: String, value: String) {
                    out.write("--$boundary\r\nContent-Disposition: form-data; name=\"$name\"\r\n\r\n$value\r\n".toByteArray())
                }
                field("model", SttConfig.model(context))
                field("response_format", "json")
                out.write("--$boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"${file.name}\"\r\nContent-Type: audio/mp4\r\n\r\n".toByteArray())
                file.inputStream().use { it.copyTo(out) }
                out.write("\r\n--$boundary--\r\n".toByteArray())
            }
            val code = conn.responseCode
            if (code !in 200..299) return@withContext null
            val body = conn.inputStream.bufferedReader(Charsets.UTF_8).use { it.readText() }.trim()
            val text = if (body.startsWith("{")) JSONObject(body).optString("text", "") else body
            text.trim().takeIf { it.isNotEmpty() }
        } catch (e: Exception) {
            null
        } finally {
            conn?.disconnect()
        }
    }
}
