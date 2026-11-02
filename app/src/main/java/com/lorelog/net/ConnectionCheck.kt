package com.lorelog.net

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL

/**
 * 저장한 서랍 주소·토큰으로 실제로 닿는지 한 번 확인한다.
 * 토큰이 필요한 가벼운 GET(/diary/allow)을 써서 주소와 토큰을 함께 본다.
 */
object ConnectionCheck {
    sealed class Result(val message: String) {
        data object Connected : Result("✓ 서랍에 연결됐어요. 기록은 지금부터 서랍으로 갑니다.")
        data object BadToken : Result("⚠ 서랍에는 닿았지만 토큰이 맞지 않아요. 데스크톱에서 연결 코드를 다시 복사해 넣으세요.")
        data class NotDrawer(val code: Int) : Result("⚠ 이 주소는 서랍이 아닌 것 같아요(응답 $code). 주소를 다시 확인하세요.")
        data class Unreachable(val error: String) : Result(
            "⚠ 서랍에 닿지 못했어요($error). PC가 켜져 있는지, 같은 와이파이나 Tailscale에 있는지 확인하세요.",
        )
    }

    /** 응답 코드(못 받았으면 null)와 예외 이름으로 판정한다. */
    fun judge(code: Int?, error: String?): Result = when (code) {
        null -> Result.Unreachable(error ?: "응답 없음")
        200 -> Result.Connected
        401 -> Result.BadToken
        else -> Result.NotDrawer(code)
    }

    suspend fun run(context: Context): Result = withContext(Dispatchers.IO) {
        val u = EndpointConfig.url(context) ?: return@withContext Result.Unreachable("주소 없음")
        var conn: HttpURLConnection? = null
        try {
            val base = URL(u).let { "${it.protocol}://${it.authority}" }
            conn = (URL("$base/diary/allow").openConnection() as HttpURLConnection).apply {
                connectTimeout = 8_000
                readTimeout = 8_000
                EndpointConfig.token(context)?.let { setRequestProperty("Authorization", "Bearer $it") }
            }
            judge(conn.responseCode, null)
        } catch (e: Exception) {
            judge(null, e.javaClass.simpleName)
        } finally {
            conn?.disconnect()
        }
    }
}
