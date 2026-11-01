package com.lorelog.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectCodeTest {
    private val token = "0123456789abcdef0123456789abcdef"

    @Test
    fun aTokenGoesOnlyOverHttpsOrToTheTailnetAndThisDevice() {
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
        )) assertTrue(ok, ConnectCode.safeForToken(ok))
        for (bad in listOf(
            "http://192.168.0.10:8765",
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
    fun manualEntryWithATokenFollowsTheSameRule() {
        assertEquals(null, ConnectCode.manualProblem("http://pc.tail8d1ab7.ts.net:8765/ingest/lorelog", token))
        assertEquals(null, ConnectCode.manualProblem("http://192.168.0.10:8765/ingest/lorelog", ""))
        assertEquals(null, ConnectCode.manualProblem("", ""))
        assertTrue(ConnectCode.manualProblem("http://192.168.0.10:8765/ingest/lorelog", token)!!.contains("tailnet"))
    }
}
