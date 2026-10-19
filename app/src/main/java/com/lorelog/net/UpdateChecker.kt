package com.lorelog.net

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * GitHub 최신 릴리스와 현재 설치 버전을 비교한다(사이드로드라 Play 자동 업데이트가 없음).
 * 네트워크는 수동 '업데이트 확인'에서만 호출. 실패하면 null.
 */
object UpdateChecker {
    private const val REPO = "jwgung/lorelog"

    data class Result(
        val current: String,
        val latest: String,
        val newer: Boolean,
        val pageUrl: String,
        val apkUrl: String?,
    )

    fun currentVersion(context: Context): String = try {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: ""
    } catch (e: Exception) {
        ""
    }

    suspend fun check(context: Context): Result? = withContext(Dispatchers.IO) {
        val current = currentVersion(context)
        var conn: HttpURLConnection? = null
        try {
            conn = (URL("https://api.github.com/repos/$REPO/releases/latest").openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = 10_000
                readTimeout = 15_000
                setRequestProperty("Accept", "application/vnd.github+json")
                setRequestProperty("User-Agent", "LoreLog")
            }
            if (conn.responseCode !in 200..299) return@withContext null
            val body = conn.inputStream.bufferedReader(Charsets.UTF_8).use { it.readText() }
            val obj = JSONObject(body)
            val tag = obj.optString("tag_name").ifBlank { return@withContext null }
            val page = obj.optString("html_url")
            val apk = obj.optJSONArray("assets")?.let { arr ->
                (0 until arr.length()).asSequence()
                    .mapNotNull { arr.optJSONObject(it) }
                    .firstOrNull { it.optString("name").endsWith(".apk", true) }
                    ?.optString("browser_download_url")
            }
            Result(current, tag, isNewer(tag, current), page, apk?.takeIf { it.isNotBlank() })
        } catch (e: Exception) {
            null
        } finally {
            conn?.disconnect()
        }
    }

    /** 태그(vX.Y.Z)가 현재 버전보다 높은지. 숫자 파트만 비교. */
    fun isNewer(latest: String, current: String): Boolean {
        val l = parse(latest); val c = parse(current)
        val n = maxOf(l.size, c.size)
        for (i in 0 until n) {
            val a = l.getOrElse(i) { 0 }; val b = c.getOrElse(i) { 0 }
            if (a != b) return a > b
        }
        return false
    }

    private fun parse(v: String): List<Int> =
        v.trim().trimStart('v', 'V').substringBefore('-').split('.').mapNotNull { it.toIntOrNull() }
}
