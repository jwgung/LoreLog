package com.lorelog.summary

import android.content.Context
import com.lorelog.net.LlmConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * 클라우드 LLM 요약기(OpenAI 호환 /chat/completions). URL·모델·키는 [LlmConfig].
 * 무료 티어 키(Gemini AI Studio, Groq 등)를 쓰면 공짜로 돈다. 데이터는 그 제공자로 나간다.
 */
class CloudSummarizer(private val context: Context) : Summarizer {

    override suspend fun summarize(text: String): Summarizer.SummaryResult = withContext(Dispatchers.IO) {
        if (text.isBlank()) return@withContext Summarizer.SummaryResult.Unavailable("요약할 기록이 없습니다")
        val key = LlmConfig.apiKey(context) ?: return@withContext Summarizer.SummaryResult.Unavailable("요약 LLM 키가 없습니다")
        val url = LlmConfig.baseUrl(context).trimEnd('/') + "/chat/completions"
        val request = JSONObject().apply {
            put("model", LlmConfig.model(context))
            put("temperature", 0)
            put("messages", JSONArray().apply {
                put(JSONObject().put("role", "system").put("content", SYSTEM_PROMPT))
                put(JSONObject().put("role", "user").put("content", text))
            })
        }
        var conn: HttpURLConnection? = null
        try {
            conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                doOutput = true
                connectTimeout = 15_000
                readTimeout = 40_000
                setRequestProperty("Content-Type", "application/json")
                setRequestProperty("Authorization", "Bearer $key")
            }
            conn.outputStream.use { it.write(request.toString().toByteArray(Charsets.UTF_8)) }
            val code = conn.responseCode
            if (code !in 200..299) {
                return@withContext Summarizer.SummaryResult.Unavailable("요약 LLM 오류 ($code)")
            }
            val body = conn.inputStream.bufferedReader(Charsets.UTF_8).use { it.readText() }
            val content = JSONObject(body)
                .getJSONArray("choices").getJSONObject(0)
                .getJSONObject("message").getString("content").trim()
            if (content.isEmpty()) Summarizer.SummaryResult.Unavailable("요약 결과가 비었습니다")
            else Summarizer.SummaryResult.Ok(content)
        } catch (e: Exception) {
            Summarizer.SummaryResult.Unavailable(e.message ?: "요약 중 오류")
        } finally {
            conn?.disconnect()
        }
    }

    companion object {
        private const val SYSTEM_PROMPT =
            "다음은 사용자의 하루 기록(음성·화면·앱·위치·알림·통화 등)이다. 한국어로 핵심만 3~5줄로 요약해라. 추측하지 말고 기록에 있는 것만."
    }
}
