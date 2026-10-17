package com.lifelog.net

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/** OpenAI 호환 GET /models 로 모델 id 목록을 받아온다. 실패하면 빈 목록. */
object ModelCatalog {
    suspend fun fetch(baseUrl: String, apiKey: String): List<String> = withContext(Dispatchers.IO) {
        if (baseUrl.isBlank() || apiKey.isBlank()) return@withContext emptyList()
        val url = baseUrl.trim().trimEnd('/') + "/models"
        var conn: HttpURLConnection? = null
        try {
            conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = 10_000
                readTimeout = 15_000
                setRequestProperty("Authorization", "Bearer ${apiKey.trim()}")
            }
            if (conn.responseCode !in 200..299) return@withContext emptyList()
            val body = conn.inputStream.bufferedReader(Charsets.UTF_8).use { it.readText() }
            val arr = JSONObject(body).optJSONArray("data") ?: return@withContext emptyList()
            buildList {
                for (i in 0 until arr.length()) {
                    arr.optJSONObject(i)?.optString("id")?.takeIf { it.isNotBlank() }?.let { add(it) }
                }
            }.distinct().sorted()
        } catch (e: Exception) {
            emptyList()
        } finally {
            conn?.disconnect()
        }
    }
}
