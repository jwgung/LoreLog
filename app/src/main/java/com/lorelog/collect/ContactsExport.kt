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

    /** 내보내기 URL(…/ingest/lorelog) 옆의 …/ingest/contacts. */
    private fun url(context: Context): String? {
        val u = EndpointConfig.url(context) ?: return null
        return runCatching { URL(u).let { "${it.protocol}://${it.authority}/ingest/contacts" } }.getOrNull()
    }

    /** 바뀐 주소록을 보낸다. 반환: 보낸 연락처 수, 0 바뀐 것 없음, -1 보내지 않음(권한·설정·주소 없음), -2 전송 실패. */
    suspend fun push(context: Context): Int {
        if (!CollectorPrefs.enabled(context, CollectorPrefs.CONTACTS)) return -1
        if (context.checkSelfPermission(Manifest.permission.READ_CONTACTS) != PackageManager.PERMISSION_GRANTED) return -1
        val url = url(context) ?: return -1
        val entries = runCatching { read(context) }.getOrElse { return -1 }
        val d = digest(entries)
        val prefs = context.getSharedPreferences(PREF, Context.MODE_PRIVATE)
        if (prefs.getString(KEY_DIGEST, null) == d) return 0
        val snap = System.currentTimeMillis()
        val body = entries.joinToString("\n", postfix = "\n") { json(it, snap).toString() }
        // an empty book is sent too (one marker line), so contacts deleted on the phone go from the server as well
        val sent = if (entries.isEmpty()) {
            JSONObject().apply { put("type", "contact"); put("snap", snap); put("ts", snap); put("name", ""); put("emails", JSONArray()); put("phones", JSONArray()) }.toString() + "\n"
        } else body
        if (!HttpSink.post(url, EndpointConfig.token(context), sent)) return -2
        prefs.edit().putString(KEY_DIGEST, d).apply()
        return entries.size
    }
}
