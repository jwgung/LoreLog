package com.lorelog.net

import android.content.Context
import android.util.Base64
import android.util.Log
import com.lorelog.data.LogRepository
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ConcurrentHashMap

/**
 * 오디오 파일을 Gemini(generateContent, inline audio)로 한국어 전사한다.
 * 요약용 LlmConfig(Gemini) 키·URL 을 재사용하고 모델은 TranscribeConfig 의 전사용 모델을 쓴다.
 * 여전히 클라우드(소리가 구글로 감). 실패 사유(HTTP 코드·메시지)는 로그와 상태줄에 남긴다.
 * inline 요청 한도(20MB) 때문에 큰 파일은 건너뛴다(그 경우 파일을 남겨 다른 엔진/수동 처리).
 *
 * 반환: 텍스트 = 성공, "" = 말소리 없음(저장할 것 없음, 파일 삭제해도 됨), null = 실패(파일 보관·재시도).
 * 지시문은 systemInstruction 으로만 보내고 contents 에는 오디오만 넣는다(텍스트 파트를 섞으면 무음일 때
 * 모델이 그 지시문을 그대로 받아쓰는 환각이 난다). 무음이면 센티널을 내게 해 걸러낸다.
 *
 * 무료 티어 한도는 "프로젝트당·모델당 하루 N회"라 모델을 여러 개 돌려 쓰면 예산이 모델 수만큼 는다.
 * 주 모델이 일일 한도에 걸리면 그 모델만 재개 시각까지 쉬게 하고 다음 후보로 바로 넘어간다.
 */
object GeminiTranscriber {
    private const val TAG = "GeminiTranscriber"
    private const val MAX_INLINE = 18 * 1024 * 1024
    private const val SILENT = "[무음]"
    private const val SYSTEM =
        "너는 한국어 음성 받아쓰기 엔진이다. 입력 오디오의 말을 들리는 그대로 한국어로 옮겨 적어라. " +
            "설명·요약·번역·따옴표·머리말을 붙이지 말고 전사한 말만 출력한다. " +
            "사람 말소리가 없거나 알아들을 수 없으면 정확히 $SILENT 만 출력한다."

    /** 지시문이 그대로 새어 나오거나 '못 들었다'는 식의 응답은 전사가 아니다. */
    private val HALLUCINATION_MARKERS = listOf(
        "받아써", "전사한 말만", "말소리가 없으면", "오디오를 듣지 못", "오디오가 없", "오디오를 제공", "음성이 없",
        "no audio", "no speech", "cannot hear",
    )

    /** 마지막 요청의 분당 한도 429 에서 서버가 알려준 재시도 대기(ms). 없으면 0. */
    @Volatile var lastRetryDelayMs: Long = 0L
        private set

    /** 모델별 일일 한도 소진 → 재개 시각(ms). 그 전엔 그 모델로 요청하지 않는다. */
    private val pausedUntil = ConcurrentHashMap<String, Long>()

    private fun isModelPaused(model: String): Boolean = (pausedUntil[model] ?: 0L) > System.currentTimeMillis()

    /** 쓸 수 있는 모델이 하나도 없으면 true(모두 일일 한도). 이때 파일은 그대로 보관된다. */
    fun isPaused(context: Context): Boolean = TranscribeConfig.geminiCandidates(context).all { isModelPaused(it) }

    /** 가장 먼저 풀리는 모델의 재개 시각 텍스트. */
    fun resumeText(context: Context): String {
        val t = TranscribeConfig.geminiCandidates(context).mapNotNull { pausedUntil[it] }.minOrNull() ?: return "?"
        return java.text.SimpleDateFormat("M/d HH:mm", java.util.Locale.KOREA).format(java.util.Date(t))
    }

    /** 모델 출력 정리. 무음 센티널·지시문 에코·숫자/기호뿐인 토막은 "" 로. */
    /** 자막 타임코드("00:00:01,847 --> 00:02:18,487"), "[00:12]" 식 시각 표기, 자막 번호 줄. 본문이 아니라 지운다. */
    private val TIMECODE = Regex(
        "(?m)\\d{1,2}:\\d{2}:\\d{2}[,.]\\d{1,3}\\s*-->\\s*\\d{1,2}:\\d{2}:\\d{2}[,.]\\d{1,3}" +
            "|\\[\\d{1,2}:\\d{2}(:\\d{2})?([,.]\\d{1,3})?\\]" +
            "|^\\s*\\d{1,4}\\s*$",
    )

    internal fun clean(raw: String): String {
        val t = TIMECODE.replace(raw, " ").replace(Regex("[ \\t]{2,}"), " ").trim().trim('"', '“', '”', '\'').trim()
        if (t.isEmpty() || t.contains(SILENT) || t.equals("[silence]", true)) return ""
        val l = t.lowercase()
        if (HALLUCINATION_MARKERS.any { l.contains(it) }) return ""
        // "네, 알겠습니다. 내용을 전사해 드릴게요." 식 서두 줄은 전사가 아니다
        val lines = t.lines()
        val body = if (lines.size > 1) {
            // 짧은 서두 줄만 버린다. 실제 대화에서 나온 "네 알겠습니다"는 본문 안에 있어 길이로 구분된다.
            lines.dropWhile { line -> line.length <= 40 && PREAMBLE.any { line.contains(it) } }.joinToString("\n").trim()
        } else t
        if (body.isEmpty()) return ""
        return finish(body)
    }

    private val PREAMBLE = listOf("알겠습니다", "전사해 드", "전사해드", "전사 결과", "다음은 ", "받아쓰기 결과", "transcript:", "here is")

    private fun finish(t: String): String {
        // 한글/영문 글자가 하나도 없으면(예: "0", "00:00", "...") 버린다
        if (!t.any { it.isLetter() }) return ""
        return t
    }

    private sealed class Outcome {
        data class Ok(val text: String) : Outcome()
        data class DailyQuota(val retryMs: Long) : Outcome()
        /** 이 모델만의 문제(응답 지연·과부하): 다음 모델로 넘어간다. */
        object Retry : Outcome()
        /** HTTP 400: 요청 형식을 거부. thinking 설정 형식을 바꿔 같은 모델에 다시 보낸다. */
        object BadRequest : Outcome()
        object Fail : Outcome()
    }

    /**
     * thinking 끄기 형식 후보(순서대로 시도). 받아쓰기엔 thinking 이 불필요하고 켜져 있으면 30~60초 걸리거나 503 이 난다.
     * 0: thinkingBudget 0 (3.x Flash, 3.1-flash-lite) · 1: thinkingLevel minimal (3.5-flash-lite, 3.6-flash) · 2: 설정 없음.
     * 모델별로 통한 형식은 [thinkingVariant] 에 기억한다(프로세스 수명).
     */
    private val THINKING_VARIANTS: List<(() -> JSONObject)?> = listOf(
        { JSONObject().put("thinkingBudget", 0) },
        { JSONObject().put("thinkingLevel", "minimal") },
        null,
    )
    private val thinkingVariant = ConcurrentHashMap<String, Int>()

    suspend fun transcribe(context: Context, file: File): String? = withContext(Dispatchers.IO) {
        val key = LlmConfig.apiKey(context) ?: run {
            Log.w(TAG, "no api key"); return@withContext null
        }
        if (file.length() > MAX_INLINE) {
            Log.w(TAG, "skip ${file.name}: ${file.length()} bytes > inline limit"); return@withContext null
        }
        val candidates = TranscribeConfig.geminiCandidates(context)
        val b64 = Base64.encodeToString(file.readBytes(), Base64.NO_WRAP)
        for (model in candidates) {
            if (isModelPaused(model)) continue
            // thinking 설정은 모델마다 받는 형식이 다르고 예고 없이 바뀐다(2026-10-21: 3.5-flash-lite 가 thinkingBudget 0 을
            // 400 으로 거부하기 시작). 400 이면 같은 모델에 다음 형식으로 바로 다시 보내고, 통한 형식을 기억한다.
            var variant = thinkingVariant[model] ?: 0
            var result: Outcome
            while (true) {
                result = request(context, key, model, b64, variant)
                if (result is Outcome.BadRequest && variant < THINKING_VARIANTS.lastIndex) {
                    variant++
                    Log.w(TAG, "$model rejected thinking variant ${variant - 1}, retry with $variant")
                    continue
                }
                if (result is Outcome.Ok) thinkingVariant[model] = variant
                break
            }
            when (val r = result) {
                is Outcome.Ok -> return@withContext r.text
                is Outcome.DailyQuota -> {
                    val until = System.currentTimeMillis() + (r.retryMs.takeIf { it > 0 } ?: 60 * 60_000L)
                    pausedUntil[model] = until
                    Log.w(TAG, "$model daily quota exhausted, paused until $until; trying next")
                    continue
                }
                Outcome.Retry -> continue
                // 모든 thinking 형식이 거부됨: 이 모델 자체가 요청을 못 받는 것. 다음 모델로.
                Outcome.BadRequest -> continue
                Outcome.Fail -> return@withContext null
            }
        }
        if (isPaused(context)) {
            LogRepository.setStatus("Gemini 일일 무료 한도 모두 소진. ${resumeText(context)}부터 자동 재개 — 그동안 음성은 파일로 보관")
            Log.i(TAG, "all candidates paused (${candidates.joinToString()}), keep ${file.name}")
        } else {
            LogRepository.setStatus("전사 실패 — 모든 모델이 응답 지연/과부하. 나중에 자동 재시도")
            Log.w(TAG, "all candidates failed transiently, keep ${file.name}")
        }
        null
    }

    private fun request(context: Context, key: String, model: String, b64: String, thinking: Int): Outcome {
        // OpenAI 호환 base(.../openai)에서 네이티브 generateContent base 유도
        val base = LlmConfig.baseUrl(context).trim().trimEnd('/').removeSuffix("/openai")
        val url = "$base/models/$model:generateContent?key=$key"
        var conn: HttpURLConnection? = null
        return try {
            val body = JSONObject().apply {
                put("systemInstruction", JSONObject().put("parts", JSONArray().put(JSONObject().put("text", SYSTEM))))
                // thinking 을 끈다: 받아쓰기엔 불필요하고, 켜져 있으면 3.6+ Flash 가 30~60초 걸리거나 503 이 난다
                // (실측: 3.6-flash 503/타임아웃 → budget 0 이면 2.7초). Lite 모델도 이 설정을 받는다.
                put("generationConfig", JSONObject().apply {
                    put("temperature", 0)
                    THINKING_VARIANTS[thinking]?.let { put("thinkingConfig", it()) }
                })
                put("contents", JSONArray().put(JSONObject().apply {
                    put(
                        "parts",
                        JSONArray().put(JSONObject().apply {
                            put("inline_data", JSONObject().put("mime_type", "audio/wav").put("data", b64))
                        }),
                    )
                }))
            }.toString()
            conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                doOutput = true
                connectTimeout = 15_000
                readTimeout = 150_000
                setRequestProperty("Content-Type", "application/json")
            }
            conn.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
            val code = conn.responseCode
            if (code !in 200..299) {
                val err = try { conn.errorStream?.bufferedReader(Charsets.UTF_8)?.use { it.readText() } } catch (_: Exception) { null }
                val msg = err?.let { runCatching { JSONObject(it).optJSONObject("error")?.optString("message") }.getOrNull() }
                    ?.takeIf { it.isNotBlank() } ?: err?.take(200) ?: ""
                val retry = if (code == 429) parseRetryDelayMs(err) else 0L
                val daily = code == 429 && (err?.contains("PerDay") == true || retry > 10 * 60_000L)
                Log.w(TAG, "HTTP $code model=$model retry=${retry}ms daily=$daily: $msg")
                if (daily) return Outcome.DailyQuota(retry)
                lastRetryDelayMs = retry
                LogRepository.setStatus(
                    if (code == 429) "전사 한도 초과(분당, $model). 잠시 뒤 자동 재시도합니다."
                    else "전사 실패 HTTP $code ($model): ${msg.take(80)}",
                )
                if (code == 503 || code == 504) return Outcome.Retry
                if (code == 400) return Outcome.BadRequest
                return Outcome.Fail
            }
            val resp = conn.inputStream.bufferedReader(Charsets.UTF_8).use { it.readText() }
            val parts = JSONObject(resp).optJSONArray("candidates")?.optJSONObject(0)
                ?.optJSONObject("content")?.optJSONArray("parts")
            if (parts == null) {
                Log.w(TAG, "no candidates/parts in response: ${resp.take(200)}")
                return Outcome.Fail
            }
            lastRetryDelayMs = 0L
            val sb = StringBuilder()
            for (i in 0 until parts.length()) {
                parts.optJSONObject(i)?.optString("text")?.let { sb.append(it) }
            }
            val out = clean(sb.toString())
            if (out.isEmpty()) Log.i(TAG, "no speech / filtered ($model): ${sb.toString().trim().take(60)}")
            Outcome.Ok(out)
        } catch (e: java.net.SocketTimeoutException) {
            // 모델 대기열이 길어 응답이 안 온 것. 다른 모델은 멀쩡할 수 있으니 넘어간다.
            Log.w(TAG, "transcribe timeout ($model), try next")
            LogRepository.setStatus("전사 응답 지연($model) — 다음 모델 시도")
            Outcome.Retry
        } catch (e: Exception) {
            Log.w(TAG, "transcribe failed ($model): ${e.javaClass.simpleName}: ${e.message}")
            LogRepository.setStatus("전사 실패: ${e.javaClass.simpleName}")
            Outcome.Fail
        } finally {
            conn?.disconnect()
        }
    }

    /** 429 본문의 RetryInfo.retryDelay("12s") 또는 메시지의 "retry in 12.3s" 를 ms 로. */
    private fun parseRetryDelayMs(err: String?): Long {
        if (err == null) return 0L
        runCatching {
            val details = JSONObject(err).optJSONObject("error")?.optJSONArray("details")
            if (details != null) for (i in 0 until details.length()) {
                val d = details.optJSONObject(i) ?: continue
                val rd = d.optString("retryDelay")
                if (rd.endsWith("s")) return (rd.dropLast(1).toDoubleOrNull()?.times(1000))?.toLong() ?: 0L
            }
        }
        val m = Regex("retry in ([0-9.]+)s", RegexOption.IGNORE_CASE).find(err)
        return m?.groupValues?.get(1)?.toDoubleOrNull()?.times(1000)?.toLong() ?: 0L
    }
}
