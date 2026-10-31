package com.lorelog.collect

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.provider.ContactsContract
import com.lorelog.net.EndpointConfig
import com.lorelog.net.HttpSink
import org.json.JSONArray
import org.json.JSONObject
import java.net.URL
import java.security.MessageDigest

/**
 * 폰 주소록을 금고로 보낸다(#22). 서버의 사서가 "이 사람이 누구인지"(이름과 메일·전화를 한 사람으로 묶기)에 쓴다.
 * 바뀌었을 때만, 목록 전체를 한 번에 보낸다(줄마다 같은 snap). 서버는 가장 새 목록만 쓰므로 폰에서 지운 연락처는 사라진다.
 * 보내는 것은 이름·메일·전화·회사뿐이다(사진, 메모, 생일은 보내지 않는다). 권한이 없거나 꺼 두면 보내지 않는다.
 */
object ContactsExport {
    private const val PREF = "contacts_export"
    private const val KEY_DIGEST = "digest"

    private class Entry(val name: String) {
        val emails = sortedSetOf<String>()
        val phones = sortedSetOf<String>()
        var org = ""
    }

    private fun read(context: Context): List<Entry> {
        val byId = sortedMapOf<Long, Entry>()
        val cols = arrayOf(
            ContactsContract.Data.CONTACT_ID,
            ContactsContract.Data.DISPLAY_NAME,
            ContactsContract.Data.MIMETYPE,
            ContactsContract.Data.DATA1,
        )
        val kinds = arrayOf(
            ContactsContract.CommonDataKinds.Email.CONTENT_ITEM_TYPE,
            ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE,
            ContactsContract.CommonDataKinds.Organization.CONTENT_ITEM_TYPE,
        )
        context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI, cols,
            "${ContactsContract.Data.MIMETYPE} IN (?, ?, ?)", kinds, null,
        )?.use { c ->
            while (c.moveToNext()) {
                val id = c.getLong(0)
                val value = c.getString(3)?.trim().orEmpty()
                if (value.isEmpty()) continue
                val e = byId.getOrPut(id) { Entry(c.getString(1)?.trim().orEmpty()) }
                when (c.getString(2)) {
                    kinds[0] -> e.emails.add(value.lowercase())
                    kinds[1] -> e.phones.add(value)
                    kinds[2] -> if (e.org.isEmpty()) e.org = value
                }
            }
        }
        return byId.values.filter { it.emails.isNotEmpty() || it.phones.isNotEmpty() }
    }

    private fun json(e: Entry, snap: Long): JSONObject = JSONObject().apply {
        put("type", "contact"); put("snap", snap); put("ts", snap)
        put("name", e.name); put("org", e.org)
        put("emails", JSONArray(e.emails.toList())); put("phones", JSONArray(e.phones.toList()))
    }

    private fun digest(entries: List<Entry>): String {
        val text = entries.joinToString("\n") { "${it.name}|${it.org}|${it.emails}|${it.phones}" }
        return MessageDigest.getInstance("SHA-256").digest(text.toByteArray()).joinToString("") { "%02x".format(it) }
    }

    /**
     * 내보내기 URL(…/ingest/lorelog) 옆의 …/ingest/contacts(앞에 붙은 경로는 그대로). 주소록은 남의 연락처라 평문으로 보내지
     * 않는다: https, 또는 WireGuard로 암호화되는 tailnet(*.ts.net, 100.64.0.0/10)·이 기기(loopback)의 http만.
     */
    private fun url(context: Context): String? {
        val u = EndpointConfig.url(context) ?: return null
        val parsed = runCatching { URL(u) }.getOrNull() ?: return null
        val host = parsed.host.lowercase()
        val tailnet = host.endsWith(".ts.net") || host == "localhost" || host == "127.0.0.1" || host == "::1" ||
            host.split('.').let { p -> p.size == 4 && p[0] == "100" && (p[1].toIntOrNull() ?: 0) in 64..127 }
        if (parsed.protocol != "https" && !(parsed.protocol == "http" && tailnet)) return null
        val path = parsed.path.trimEnd('/')
        val prefix = if (path.endsWith("/ingest/lorelog")) path.removeSuffix("/ingest/lorelog") else ""
        return "${parsed.protocol}://${parsed.authority}$prefix/ingest/contacts"
    }

    private fun marker(snap: Long): String = JSONObject().apply {
        put("type", "contact"); put("snap", snap); put("ts", snap); put("name", "")
        put("emails", JSONArray()); put("phones", JSONArray())
    }.toString() + "\n"

    /**
     * 바뀐 주소록을 보낸다. 꺼 두었거나 권한을 거둔 뒤에는 빈 목록을 한 번 보내 서버에서도 지운다.
     * 반환: 보낸 연락처 수, 0 바뀐 것 없음(또는 빈 목록을 보냄), -1 보내지 않음(권한·설정·주소 없음), -2 전송 실패.
     */
    suspend fun push(context: Context): Int {
        val url = url(context) ?: return -1
        val prefs = context.getSharedPreferences(PREF, Context.MODE_PRIVATE)
        val key = "$KEY_DIGEST:$url" // per server: a new server gets the list even if the phone's did not change
        val snap = System.currentTimeMillis()
        val allowed = CollectorPrefs.enabled(context, CollectorPrefs.CONTACTS) &&
            context.checkSelfPermission(Manifest.permission.READ_CONTACTS) == PackageManager.PERMISSION_GRANTED
        if (!allowed) {
            if (prefs.getString(key, null) == null) return -1 // nothing was sent there
            if (!HttpSink.post(url, EndpointConfig.token(context), marker(snap))) return -2
            prefs.edit().remove(key).apply()
            return 0
        }
        val entries = runCatching { read(context) }.getOrElse { return -1 }
        val d = digest(entries)
        if (prefs.getString(key, null) == d) return 0
        // an empty book is sent too (one marker line), so contacts deleted on the phone go from the server as well
        val body = if (entries.isEmpty()) marker(snap) else entries.joinToString("\n", postfix = "\n") { json(it, snap).toString() }
        if (!HttpSink.post(url, EndpointConfig.token(context), body)) return -2
        prefs.edit().putString(key, d).apply()
        return entries.size
    }
}
