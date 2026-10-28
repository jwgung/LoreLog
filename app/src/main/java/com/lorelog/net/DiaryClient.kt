package com.lorelog.net

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/** 서버의 일기 엔드포인트(GET/POST /diary/<날짜>). 주소와 토큰은 내보내기 설정(EndpointConfig)을 그대로 쓴다. */
object DiaryClient {
    sealed class Draft {
        data class Ready(val summary: List<String>, val question: String, val corrections: String, val reflection: String) : Draft()
        data object Generating : Draft()
        data class Error(val message: String) : Draft()
    }

    /** 내보내기 URL(…/ingest/lorelog)에서 서버 주소만 뽑는다. */
    private fun base(context: Context): String? {
        val u = EndpointConfig.url(context) ?: return null
        return runCatching { URL(u).let { "${it.protocol}://${it.authority}" } }.getOrNull()
    }

    private fun open(context: Context, path: String, method: String): HttpURLConnection? {
        val b = base(context) ?: return null
        return (URL("$b$path").openConnection() as HttpURLConnection).apply {
            requestMethod = method
            connectTimeout = 15_000
            readTimeout = 30_000
            EndpointConfig.token(context)?.let { setRequestProperty("Authorization", "Bearer $it") }
        }
    }

    suspend fun fetchDraft(context: Context, day: String, refresh: Boolean = false): Draft = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/diary/$day" + if (refresh) "?refresh=1" else "", "GET")
                ?: return@withContext Draft.Error("서버 주소가 설정되지 않았습니다(설정 → 내보내기).")
            when (val code = conn.responseCode) {
                200 -> {
                    val o = JSONObject(conn.inputStream.bufferedReader().readText())
                    val arr = o.optJSONArray("summary") ?: JSONArray()
                    Draft.Ready(
                        (0 until arr.length()).map { arr.getString(it) },
                        o.optString("question"), o.optString("corrections"), o.optString("reflection"),
                    )
                }
                202 -> Draft.Generating
                else -> Draft.Error("서버 응답 $code: " + (conn.errorStream?.bufferedReader()?.readText() ?: ""))
            }
        } catch (e: Exception) {
            Draft.Error("서버에 연결하지 못했습니다(${e.javaClass.simpleName}). tailnet 연결을 확인하세요.")
        } finally {
            conn?.disconnect()
        }
    }

    data class Sender(val label: String, val title: String, val count: Int, val allowed: Boolean)

    /** 최근 14일 알림 상대 목록과 일기 재료로 허용했는지 여부. 실패하면 null. */
    suspend fun fetchSenders(context: Context): List<Sender>? = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/diary/senders", "GET") ?: return@withContext null
            if (conn.responseCode != 200) return@withContext null
            val arr = JSONArray(conn.inputStream.bufferedReader().readText())
            (0 until arr.length()).map {
                val o = arr.getJSONObject(it)
                Sender(o.getString("label"), o.getString("title"), o.optInt("count"), o.optBoolean("allowed"))
            }
        } catch (e: Exception) {
            null
        } finally {
            conn?.disconnect()
        }
    }

    /** 일기 재료로 허용할 목록을 저장한다. 항목은 "앱: 상대" 또는 "앱: *"(앱 전체). */
    suspend fun saveAllow(context: Context, allow: List<String>): Boolean = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/diary/senders", "POST") ?: return@withContext false
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "application/json")
            conn.outputStream.use { it.write(JSONObject().put("allow", JSONArray(allow)).toString().toByteArray(Charsets.UTF_8)) }
            conn.responseCode in 200..299
        } catch (e: Exception) {
            false
        } finally {
            conn?.disconnect()
        }
    }

    /** 사용자가 한 말을 일기에 남기고 비서의 답을 받는다(서버의 로컬 LLM). [wrong]이 있으면 그 문장을 고치는 말이다. 실패하면 null. */
    suspend fun chat(context: Context, day: String, message: String, wrong: String? = null): String? = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/diary/$day/chat", "POST") ?: return@withContext null
            conn.readTimeout = 150_000
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "application/json")
            val body = JSONObject().put("message", message).apply { if (wrong != null) put("wrong", wrong) }
            conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
            if (conn.responseCode !in 200..299) return@withContext null
            JSONObject(conn.inputStream.bufferedReader().readText()).optString("reply").takeIf { it.isNotBlank() }
        } catch (e: Exception) {
            null
        } finally {
            conn?.disconnect()
        }
    }

    /** 정정(틀린 것 → 맞는 것 쌍)과 회고를 저장한다. */
    suspend fun save(context: Context, day: String, corrections: List<Pair<String, String>>, reflection: String, append: Boolean = true): Boolean =
        withContext(Dispatchers.IO) {
            var conn: HttpURLConnection? = null
            try {
                conn = open(context, "/diary/$day", "POST") ?: return@withContext false
                conn.doOutput = true
                conn.setRequestProperty("Content-Type", "application/json")
                val body = JSONObject().apply {
                    put("reflection", reflection)
                    put("append", append)
                    put("corrections", JSONArray().also { a ->
                        corrections.forEach { (w, r) -> a.put(JSONObject().put("wrong", w).put("right", r)) }
                    })
                }
                conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
                conn.responseCode in 200..299
            } catch (e: Exception) {
                false
            } finally {
                conn?.disconnect()
            }
        }
}
