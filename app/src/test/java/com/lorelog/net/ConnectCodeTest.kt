package com.lorelog.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectCodeTest {
    private val token = "0123456789abcdef0123456789abcdef"

    @Test
    fun aTokenGoesOnlyOverHttpsOrToTheTailnetTheSameNetworkAndThisDevice() {
        for (ok in listOf(
            "https://vault.example.com/ingest/lorelog",
            "https://pc.tail8d1ab7.ts.net:48765",
            "http://pc.tail8d1ab7.ts.net:8765/ingest/lorelog",
            "http://PC.Tail8d1ab7.TS.NET:8765",
            "http://100.101.1.2:8765",
            "http://127.0.0.1:8765",
            "http://localhost:8765",
            "http://[::1]:8765",
            "http://[fd7a:115c:a1e0::1]:8765",
            // 같은 와이파이(사설 IPv4 주소): 암호화 없는 http
            "http://192.168.0.10:8765",
            "http://192.168.100.3:8765/ingest/lorelog",
            "http://10.0.0.1:8765",
            "http://10.255.255.254:8765",
            "http://172.16.0.1:8765",
            "http://172.31.255.1:8765",
        )) assertTrue(ok, ConnectCode.safeForToken(ok))
        // 서랍 서버의 safe_for_token 시험(server/tests/onboard.rs)과 같은 목록
        for (bad in listOf(
            "http://172.15.0.1:8765",
            "http://172.32.0.1:8765",
            "http://192.169.0.1:8765",
            "http://11.0.0.1:8765",
            "http://169.254.1.1:8765",
            "http://0.0.0.0:8765",
            "http://8.8.8.8:8765",
            "http://012.0.0.1:8765",
            "http://192.168.00.1:8765",
            "http://0xa.0.0.1:8765",
            "http://10.1:8765",
            "http://3232235521:8765",
            "http://100.64.1:8765",
            "http://10.0.0.1@evil.com/",
            "http://10.0.0.1\\@evil.com/",
            "http://evil.com\\@10.0.0.1/",
            "http://%31%30.0.0.1:8765",
            "http://+10.0.0.1:8765",
            "http://nas.local:8765",
            "http://nas:8765",
            "http://[fd00::1]:8765",
            "http://[::ffff:192.168.0.1]:8765",
            "http://evil.example.com",
            "http://100.200.1.2:8765",
            "http://100.064.0.1:8765",
            "http://0100.64.0.1:8765",
            "http://ts.net.evil.com",
            "http://127.0.0.1@evil.com/",
            "http://pc.ts.net@evil.com/",
            "ftp://pc.tail8d1ab7.ts.net",
            "",
            "not a url",
        )) assertFalse(bad, ConnectCode.safeForToken(bad))
    }

    @Test
    fun aCodeFromTheDesktopFillsTheExportAddressAndToken() {
        val code = "lorelog://connect?u=http%3A%2F%2Fpc.tail8d1ab7.ts.net%3A8765&t=$token"
        val r = ConnectCode.parse("  $code\n") as ConnectCode.Parsed.Ok
        assertEquals("http://pc.tail8d1ab7.ts.net:8765/ingest/lorelog", r.url)
        assertEquals(token, r.token)
    }

    @Test
    fun aCodeForTheSameWifiFromTheDesktopIsRead() {
        // 서랍 서버가 만드는 그대로의 한 줄(server/tests/onboard.rs): 양쪽이 같은 규칙을 쓴다
        val r = ConnectCode.parse("lorelog://connect?u=http%3A%2F%2F192.168.100.3%3A8765&t=$token") as ConnectCode.Parsed.Ok
        assertEquals("http://192.168.100.3:8765/ingest/lorelog", r.url)
        assertEquals(token, r.token)
        assertTrue(ConnectCode.parse("lorelog://connect?u=http%3A%2F%2F012.0.0.1%3A8765&t=$token") is ConnectCode.Parsed.Bad)
    }

    @Test
    fun theSameWifiAddressGetsTheNoteAndOthersDoNot() {
        for (lan in listOf("http://192.168.100.3:8765/ingest/lorelog", " http://10.0.0.1:8765 ", "http://172.16.0.1")) {
            assertTrue(lan, ConnectCode.overSameWifi(lan))
        }
        for (other in listOf(
            "https://192.168.100.3:8765", // 암호화된다
            "http://pc.tail8d1ab7.ts.net:8765",
            "http://100.101.1.2:8765",
            "http://127.0.0.1:8765",
            "http://8.8.8.8:8765",
            "http://012.0.0.1:8765",
            "",
        )) assertFalse(other, ConnectCode.overSameWifi(other))
        assertEquals(
            "같은 와이파이 안에서만 닿고, 기록과 토큰이 암호화 없이 와이파이로 갑니다. 집이나 사무실처럼 믿는 와이파이에서만 쓰세요.",
            ConnectCode.SAME_WIFI_NOTE,
        )
    }

    @Test
    fun escapedCharactersSurviveAndAGivenIngestPathIsKept() {
        val odd = "ab+c/d=e&f?g%h_0123456789"
        val code = "lorelog://connect?u=https%3A%2F%2Fvault.example.com%2Fingest%2Florelog&t=ab%2Bc%2Fd%3De%26f%3Fg%25h_0123456789"
        val r = ConnectCode.parse(code) as ConnectCode.Parsed.Ok
        assertEquals("https://vault.example.com/ingest/lorelog", r.url)
        assertEquals(odd, r.token)
    }

    @Test
    fun aBadCodeIsRefusedWithAReason() {
        for (bad in listOf(
            "",
            "hello",
            "https://x.ts.net/connect?u=https%3A%2F%2Fx.ts.net&t=$token",
            "lorelog://connect?u=https%3A%2F%2Fx.ts.net",
            "lorelog://connect?t=$token",
            "lorelog://connect?u=https%3A%2F%2Fx.ts.net&t=short",
            "lorelog://connect?u=https%3A%2F%2Fx.ts.net&t=$token%0A",
            "lorelog://connect?u=http%3A%2F%2Fevil.example.com&t=$token",
            "lorelog://connect?u=https%3A%2F%2Fx.ts.net&t=%ZZ",
        )) {
            val r = ConnectCode.parse(bad)
            assertTrue(bad, r is ConnectCode.Parsed.Bad && r.reason.isNotBlank())
        }
        val r = ConnectCode.parse("lorelog://connect?u=http%3A%2F%2Fevil.example.com&t=$token") as ConnectCode.Parsed.Bad
        assertTrue(r.reason, r.reason.contains("tailnet"))
    }

    @Test
    fun manualEntryFollowsTheSameRule() {
        assertEquals(null, ConnectCode.manualProblem("http://pc.tail8d1ab7.ts.net:8765/ingest/lorelog"))
        assertEquals(null, ConnectCode.manualProblem("http://192.168.0.10:8765/ingest/lorelog"))
        // records go out in plain text too: an address the token may not go to is not kept either
        val problem = ConnectCode.manualProblem("http://8.8.8.8:8765/ingest/lorelog")!!
        assertTrue(problem, problem.contains("tailnet") && problem.contains("와이파이"))
        assertEquals(null, ConnectCode.manualProblem(""))
    }
}
