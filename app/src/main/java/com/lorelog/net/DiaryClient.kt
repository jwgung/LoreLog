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
                ?: return@withContext Draft.Error("서랍 주소가 설정되지 않았습니다(설정 → 서랍 연결).")
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
                else -> Draft.Error("서랍 응답 $code: " + (conn.errorStream?.bufferedReader()?.readText() ?: ""))
            }
        } catch (e: Exception) {
            Draft.Error("서랍에 연결하지 못했습니다(${e.javaClass.simpleName}). 와이파이나 Tailscale 연결을 확인하세요.")
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

    /** 답이 인용한 기록 하나(날짜, 종류, 제목). 답 아래에 출처로 작게 보인다(#64). */
    data class Source(val day: String, val kind: String, val title: String)

    /**
     * 비서의 답, 정정이었으면 무엇이 바뀌었는지 한 줄(서버가 만든다), 그리고 사서가 찾아 답에 인용한 기록.
     * 예전 서버는 sources를 보내지 않는다: 그때는 빈 목록.
     */
    data class ChatReply(val reply: String, val changed: String?, val sources: List<Source> = emptyList())

    /**
     * 사용자가 한 말을 일기에 남기고 비서의 답을 받는다(서버의 로컬 LLM). [wrong]이 있으면 그것을 고치는 말이다:
     * [about]이 "summary"면 오늘 정리의 문장, "answer"면 비서의 답. 실패하면 null.
     */
    suspend fun chat(context: Context, day: String, message: String, wrong: String? = null, about: String = "summary"): ChatReply? = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/diary/$day/chat", "POST") ?: return@withContext null
            conn.readTimeout = 150_000
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "application/json")
            val body = JSONObject().put("message", message).apply { if (wrong != null) { put("wrong", wrong); put("about", about) } }
            conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
            if (conn.responseCode !in 200..299) return@withContext null
            val o = JSONObject(conn.inputStream.bufferedReader().readText())
            val reply = o.optString("reply").takeIf { it.isNotBlank() } ?: return@withContext null
            ChatReply(reply, o.optString("changed").takeIf { it.isNotBlank() }, sources(o.optJSONArray("sources")))
        } catch (e: Exception) {
            null
        } finally {
            conn?.disconnect()
        }
    }

    /** 서버가 보낸 출처 목록. 없거나 모양이 다르면 그 항목은 건너뛴다. */
    private fun sources(arr: org.json.JSONArray?): List<Source> {
        if (arr == null) return emptyList()
        return (0 until arr.length()).mapNotNull { i ->
            val o = arr.optJSONObject(i) ?: return@mapNotNull null
            val title = o.optString("title").trim()
            val day = o.optString("day").trim()
            if (title.isEmpty() && day.isEmpty()) null else Source(day, o.optString("kind").trim(), title)
        }
    }

    data class ChatLine(val role: String, val text: String, val wrong: String?)

    /** 오늘 서버에 남은 대화(사용자 말, 비서 답, 정정). 앱을 다시 열어도 이어서 보이게 한다. 실패하면 빈 목록. */
    suspend fun fetchChat(context: Context, day: String): List<ChatLine> = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/diary/$day/chat", "GET") ?: return@withContext emptyList()
            if (conn.responseCode != 200) return@withContext emptyList()
            val arr = JSONObject(conn.inputStream.bufferedReader().readText()).getJSONArray("messages")
            (0 until arr.length()).map {
                val o = arr.getJSONObject(it)
                ChatLine(o.getString("role"), o.getString("text"), if (o.has("wrong")) o.getString("wrong") else null)
            }
        } catch (e: Exception) { emptyList() } finally { conn?.disconnect() }
    }

    data class Todo(val id: String, val title: String, val source: String, val due: String, val auto: Boolean = false)
    /** proposed: 기록에서 비서가 찾았지만 사용자가 아직 받지 않은 것. */
    data class Todos(val proposed: List<Todo>, val open: List<Todo>, val done: List<Todo>, val extracting: Boolean)

    private fun parseTodos(arr: JSONArray) = (0 until arr.length()).map {
        val o = arr.getJSONObject(it)
        Todo(o.getString("id"), o.getString("title"), o.optString("source"), o.optString("due"), o.optBoolean("auto"))
    }

    /** 서버의 할 일 목록(열린 것, 최근 끝낸 것). 실패하면 null. */
    suspend fun fetchTodos(context: Context): Todos? = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/todos", "GET") ?: return@withContext null
            if (conn.responseCode != 200) return@withContext null
            val o = JSONObject(conn.inputStream.bufferedReader().readText())
            val proposed = o.optJSONArray("proposed") ?: JSONArray() // 예전 서버에는 없다
            Todos(parseTodos(proposed), parseTodos(o.getJSONArray("open")), parseTodos(o.getJSONArray("done")), o.optBoolean("extracting"))
        } catch (e: Exception) { null } finally { conn?.disconnect() }
    }

    private suspend fun post(context: Context, path: String, body: JSONObject?): Boolean = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, path, "POST") ?: return@withContext false
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "application/json")
            conn.outputStream.use { it.write((body ?: JSONObject()).toString().toByteArray(Charsets.UTF_8)) }
            conn.responseCode in 200..299
        } catch (e: Exception) { false } finally { conn?.disconnect() }
    }

    /** 서버가 기록에서 새 할 일을 뽑게 한다(30분에 한 번만 실제로 돈다). */
    suspend fun refreshTodos(context: Context): Boolean = post(context, "/todos/extract", null)

    /** action: "accept"(제안을 할 일로) | "done" | "dismiss" | "reopen" */
    suspend fun actTodo(context: Context, id: String, action: String): Boolean =
        post(context, "/todos/$id", JSONObject().put("action", action))

    suspend fun addTodo(context: Context, text: String): Boolean = post(context, "/todos", JSONObject().put("text", text))

    /**
     * 틀린 것 하나를 고친다(#10): [wrong]은 틀린 것, [right]는 맞는 것, [ref]는 무엇에 대한 것인지("todo:<id>",
     * "summary:<날짜>", "chat:<날짜>"). 무엇이 바뀌었는지 한 줄을 돌려준다. 실패하면 null.
     */
    suspend fun correct(context: Context, wrong: String, right: String, ref: String): String? = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/corrections", "POST") ?: return@withContext null
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "application/json")
            val body = JSONObject().put("wrong", wrong).put("right", right).put("ref", ref)
            conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
            if (conn.responseCode !in 200..299) return@withContext null
            JSONObject(conn.inputStream.bufferedReader().readText()).optString("changed").takeIf { it.isNotBlank() }
        } catch (e: Exception) {
            null
        } finally {
            conn?.disconnect()
        }
    }

    /**
     * 앞으로의 일정 앞에 비서가 먼저 말할 줄들(#60). [events]는 이 폰 캘린더의 일정(서랍은 자기 캘린더 일정을
     * 더한다), [stopped]는 사용자가 멈춘 조직. 서랍에 닿지 않으면 null.
     */
    suspend fun ahead(context: Context, events: JSONArray, stopped: Set<String>): List<com.lorelog.work.AheadRules.Line>? =
        withContext(Dispatchers.IO) {
            var conn: HttpURLConnection? = null
            try {
                conn = open(context, "/assistant/ahead", "POST") ?: return@withContext null
                conn.doOutput = true
                conn.setRequestProperty("Content-Type", "application/json")
                val body = JSONObject().put("events", events).put("stopped", JSONArray(stopped.toList()))
                conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
                if (conn.responseCode != 200) return@withContext null
                val items = JSONObject(conn.inputStream.bufferedReader().readText()).optJSONArray("items") ?: JSONArray()
                (0 until items.length()).mapNotNull { i ->
                    val o = items.optJSONObject(i) ?: return@mapNotNull null
                    com.lorelog.work.AheadRules.Line(
                        o.optString("event"), o.optLong("start"), o.optLong("at"), o.optString("title"),
                        o.optString("text"), o.optString("question"), o.optString("stop"), o.optString("edge"),
                        o.optString("wrong"), o.optString("matter"),
                    ).takeIf { it.start > 0 && it.edge.isNotBlank() }
                }
            } catch (e: Exception) {
                null
            } finally {
                conn?.disconnect()
            }
        }

    /** 일정 화면(#62)의 한 쪽: 서랍이 아는 일정과 비서 메모. */
    sealed class ScheduleResult {
        data class Ready(val events: List<com.lorelog.data.Schedule.Event>) : ScheduleResult()
        /** 서랍의 색인이 아직 만들어지지 않았다 */
        object NotReady : ScheduleResult()
        object Failed : ScheduleResult()
    }

    suspend fun schedule(context: Context, span: com.lorelog.data.Schedule.Span): ScheduleResult = withContext(Dispatchers.IO) {
        var conn: HttpURLConnection? = null
        try {
            conn = open(context, "/schedule?from=${span.from}&to=${span.to}", "GET") ?: return@withContext ScheduleResult.Failed
            when (conn.responseCode) {
                200 -> ScheduleResult.Ready(com.lorelog.data.Schedule.parse(JSONObject(conn.inputStream.bufferedReader().readText())))
                503 -> ScheduleResult.NotReady
                else -> ScheduleResult.Failed
            }
        } catch (e: Exception) { ScheduleResult.Failed } finally { conn?.disconnect() }
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
