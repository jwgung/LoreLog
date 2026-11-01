package com.lorelog.net

import java.net.InetAddress
import java.net.URI
import java.net.URLDecoder

/**
 * 데스크톱 앱이 보여 주는 한 줄 '연결 코드'(#69): `lorelog://connect?u=<금고 주소>&t=<토큰>`. 토큰이 들어 있으니 로그에 남기지 않는다.
 * 토큰은 https, 또는 WireGuard로 암호화되는 tailnet(*.ts.net, 100.64.0.0/10, fd7a:115c:a1e0::/48)·이 기기(loopback)의
 * http로만 보낸다. 코드로 넣든 손으로 넣든 같은 규칙이다(금고 서버의 `life_server::onboard::safe_for_token`과 같다).
 */
object ConnectCode {
    const val PREFIX = "lorelog://connect?"
    private const val MIN_TOKEN = 16
    private const val INGEST = "/ingest/lorelog"
    private const val NOT_SAFE = "이 주소로는 기록과 토큰을 보내지 않습니다. https 주소이거나 tailnet(*.ts.net, 100.x) 주소여야 합니다"

    sealed class Parsed {
        /** [url]은 기록을 보내는 주소(…/ingest/lorelog)까지 붙인 것. */
        data class Ok(val url: String, val token: String) : Parsed() {
            override fun toString() = "Ok(url=$url, token=<hidden>)"
        }
        data class Bad(val reason: String) : Parsed()
    }

    fun safeForToken(url: String): Boolean {
        val u = runCatching { URI(url.trim()) }.getOrNull() ?: return false
        val scheme = u.scheme?.lowercase() ?: return false
        val host = u.host?.lowercase()?.removeSuffix(".")?.removePrefix("[")?.removeSuffix("]") ?: return false
        if (host.isEmpty()) return false
        val inside = host.endsWith(".ts.net") || host == "localhost" || ipInside(host)
        return scheme == "https" || (scheme == "http" && inside)
    }

    /** 숫자로 쓴 주소만 본다(이름을 찾아보지 않는다): loopback, tailnet 범위. */
    private fun ipInside(host: String): Boolean {
        val v4 = host.split('.')
        // 0으로 시작하는 칸(100.064.0.1)은 8진수로 읽는 곳이 있어 다른 주소가 될 수 있다: 받지 않는다
        if (v4.size == 4 && v4.all { p -> !(p.length > 1 && p.startsWith('0')) && p.toIntOrNull()?.let { it in 0..255 } == true }) {
            val a = v4[0].toInt(); val b = v4[1].toInt()
            return a == 127 || (a == 100 && b in 64..127)
        }
        if (!host.contains(':') || !host.all { it.isDigit() || it in 'a'..'f' || it == ':' }) return false
        val b = runCatching { InetAddress.getByName(host).address }.getOrNull() ?: return false
        if (b.size != 16) return false
        val loopback = b.take(15).all { it.toInt() == 0 } && b[15].toInt() == 1
        val tailnet = (b[0].toInt() and 0xff) == 0xfd && (b[1].toInt() and 0xff) == 0x7a &&
            (b[2].toInt() and 0xff) == 0x11 && (b[3].toInt() and 0xff) == 0x5c &&
            (b[4].toInt() and 0xff) == 0xa1 && (b[5].toInt() and 0xff) == 0xe0
        return loopback || tailnet
    }

    private fun tokenOk(t: String) = t.length >= MIN_TOKEN && t.none { it.isWhitespace() || it.isISOControl() }

    /** 금고 주소(서버 주소만 있으면 …/ingest/lorelog를 붙인다). */
    fun exportUrl(base: String): String {
        val b = base.trim().trimEnd('/')
        val path = runCatching { URI(b).path.orEmpty() }.getOrDefault("")
        return if (path.contains("/ingest/")) b else b + INGEST
    }

    fun parse(code: String): Parsed {
        val c = code.trim()
        if (!c.startsWith(PREFIX)) return Parsed.Bad("LoreLog 연결 코드가 아닙니다(lorelog://connect? 로 시작합니다)")
        var url: String? = null
        var token: String? = null
        for (pair in c.removePrefix(PREFIX).split('&')) {
            val k = pair.substringBefore('=')
            val v = runCatching { URLDecoder.decode(pair.substringAfter('=', ""), "UTF-8") }.getOrNull()
                ?: return Parsed.Bad("연결 코드가 깨졌습니다. 다시 복사해 주세요")
            when (k) {
                "u" -> url = v.trim().trimEnd('/')
                "t" -> token = v
            }
        }
        val u = url?.takeIf { it.isNotEmpty() }
        val t = token
        if (u == null || t == null) return Parsed.Bad("연결 코드에 주소나 토큰이 빠졌습니다. 다시 복사해 주세요")
        if (!safeForToken(u)) return Parsed.Bad(NOT_SAFE)
        if (!tokenOk(t)) return Parsed.Bad("연결 코드의 토큰이 올바르지 않습니다. 다시 복사해 주세요")
        return Parsed.Ok(exportUrl(u), t)
    }

    /**
     * 손으로 넣은 주소를 저장해도 되는지. null = 괜찮음, 아니면 이유. 기록 자체도 이 주소로 나가므로 토큰이 없어도
     * 같은 규칙이다(https, 또는 tailnet·이 기기의 http만). 금고는 토큰 없는 기록을 받지 않으니 막아서 잃는 것은 없다.
     */
    fun manualProblem(url: String): String? =
        if (url.isNotBlank() && !safeForToken(url)) NOT_SAFE else null
}
