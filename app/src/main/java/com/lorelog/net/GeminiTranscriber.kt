package com.lorelog.net

import android.content.Context
import android.util.Base64
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * 오디오 파일을 Gemini(generateContent, inline audio)로 한국어 전사한다.
 * 요약용 LlmConfig(Gemini) 키·모델을 그대로 재사용한다. 여전히 클라우드(소리가 구글로 감).
 * inline 요청 한도(20MB) 때문에 큰 파일은 건너뛴다(그 경우 파일을 남겨 다른 엔진/수동 처리).
 */
object GeminiTranscriber {
    private const val MAX_INLINE = 18 * 1024 * 1024

    suspend fun transcribe(context: Context, file: File): String? = withContext(Dispatchers.IO) {
        val key = LlmConfig.apiKey(context) ?: return@withContext null
        if (file.length() > MAX_INLINE) return@withContext null
        // OpenAI 호환 base(.../openai)에서 네이티브 generateContent base 유도
        val base = LlmConfig.baseUrl(context).trim().trimEnd('/').removeSuffix("/openai")
        val model = LlmConfig.model(context)
        val url = "$base/models/$model:generateContent?key=$key"
        var conn: HttpURLConnection? = null
        try {
            val b64 = Base64.encodeToString(file.readBytes(), Base64.NO_WRAP)
            val body = JSONObject().apply {
                put("contents", JSONArray().put(JSONObject().apply {
                    put(
                        "parts",
                        JSONArray()
                            .put(JSONObject().apply {
                                put("inline_data", JSONObject()
                                    .put("mime_type", "audio/wav")
                                    .put("data", b64))
                            })
                            .put(JSONObject().put(
                                "text",
                                "이 오디오를 한국어로 그대로 받아써. 설명·따옴표 없이 전사한 말만 출력. 말소리가 없으면 아무것도 출력하지 마.",
                            )),
                    )
                }))
            }.toString()
            conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                doOutput = true
                connectTimeout = 15_000
                readTimeout = 120_000
                setRequestProperty("Content-Type", "application/json")
            }
            conn.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
            if (conn.responseCode !in 200..299) return@withContext null
            val resp = conn.inputStream.bufferedReader(Charsets.UTF_8).use { it.readText() }
            val parts = JSONObject(resp).optJSONArray("candidates")?.optJSONObject(0)
                ?.optJSONObject("content")?.optJSONArray("parts") ?: return@withContext null
            val sb = StringBuilder()
            for (i in 0 until parts.length()) {
                parts.optJSONObject(i)?.optString("text")?.let { sb.append(it) }
            }
            sb.toString().trim().takeIf { it.isNotEmpty() }
        } catch (e: Exception) {
            null
        } finally {
            conn?.disconnect()
        }
    }
}
