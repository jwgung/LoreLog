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
 * 바뀌었을 때만, 목록 전체를 한 번에 보낸다(줄마다 같은 snap). 서버는 마지막으로 받은 목록만 쓰므로 폰에서 지운 연락처는 사라진다.
 * 주소록이 가진 것은 다 보낸다(CardDAV 주소록과 같은 모양): 이름, 별명, 메일, 전화, 회사·부서·직함, 주소, 웹사이트, 메모,
 * 생일·기념일, 관계, 메신저, 그룹, 그리고 원본으로 연락처 데이터 행 그대로(사진만 뺀다). 권한이 없거나 꺼 두면 보내지 않는다.
 */
object ContactsExport {
    private const val PREF = "contacts_export"
    private const val KEY_DIGEST = "digest"
    private const val MAX_EACH = 50

    private class Entry(val name: String) {
        var nickname = ""
        val emails = JSONArray()
        val phones = JSONArray()
        val orgs = JSONArray()
        val addresses = JSONArray()
        val urls = JSONArray()
        val notes = JSONArray()
        val events = JSONArray()
        val relations = JSONArray()
        val ims = JSONArray()
        val groups = JSONArray()
        /** 데이터 행 그대로: {mime, data1..data14} (data15는 사진이라 뺀다) */
        val raw = JSONArray()
    }

    private fun add(list: JSONArray, v: Any) {
        if (list.length() >= MAX_EACH) return
        for (i in 0 until list.length()) if (list.get(i).toString() == v.toString()) return
        list.put(v)
    }

    private fun typed(type: String, value: String) = JSONObject().put("type", type).put("value", value)

    private fun groupTitles(context: Context): Map<Long, String> {
        val out = mutableMapOf<Long, String>()
        context.contentResolver.query(
            ContactsContract.Groups.CONTENT_URI, arrayOf(ContactsContract.Groups._ID, ContactsContract.Groups.TITLE), null, null, null,
        )?.use { c -> while (c.moveToNext()) c.getString(1)?.let { out[c.getLong(0)] = it } }
        return out
    }

    private fun read(context: Context): List<Entry> {
        val byId = sortedMapOf<Long, Entry>()
        val data = (1..14).map { "data$it" }
        val cols = arrayOf(ContactsContract.Data.CONTACT_ID, ContactsContract.Data.DISPLAY_NAME, ContactsContract.Data.MIMETYPE) + data
        val groups = groupTitles(context)
        context.contentResolver.query(ContactsContract.Data.CONTENT_URI, cols, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val id = c.getLong(0)
                val mime = c.getString(2) ?: continue
                if (mime == ContactsContract.CommonDataKinds.Photo.CONTENT_ITEM_TYPE) continue
                val e = byId.getOrPut(id) { Entry(c.getString(1)?.trim().orEmpty()) }
                val v = (0 until 14).map { i -> runCatching { c.getString(3 + i) }.getOrNull()?.trim().orEmpty() }
                val row = JSONObject().put("mime", mime)
                v.forEachIndexed { i, x -> if (x.isNotEmpty()) row.put("data${i + 1}", x) }
                add(e.raw, row)
                val label = { typeCol: Int, labelCol: Int, names: (Int) -> String ->
                    val t = v[typeCol].toIntOrNull()
                    if (t == 0) v[labelCol].lowercase() else if (t == null) "" else names(t)
                }
                when (mime) {
                    ContactsContract.CommonDataKinds.Nickname.CONTENT_ITEM_TYPE -> if (e.nickname.isEmpty()) e.nickname = v[0]
                    ContactsContract.CommonDataKinds.Email.CONTENT_ITEM_TYPE -> if (v[0].isNotEmpty()) add(e.emails, typed(
                        label(1, 2) { when (it) { 1 -> "home"; 2 -> "work"; 4 -> "mobile"; else -> "other" } }, v[0].lowercase()))
                    ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE -> if (v[0].isNotEmpty()) add(e.phones, typed(
                        label(1, 2) { when (it) { 1 -> "home"; 2 -> "cell"; 3 -> "work"; 4, 5 -> "fax"; else -> "other" } }, v[0]))
                    ContactsContract.CommonDataKinds.Organization.CONTENT_ITEM_TYPE -> add(e.orgs,
                        JSONObject().put("company", v[0]).put("department", v[4]).put("title", v[3]))
                    ContactsContract.CommonDataKinds.StructuredPostal.CONTENT_ITEM_TYPE -> if (v[0].isNotEmpty()) add(e.addresses, typed(
                        label(1, 2) { when (it) { 1 -> "home"; 2 -> "work"; else -> "other" } }, v[0].replace('\n', ' ')))
                    ContactsContract.CommonDataKinds.Website.CONTENT_ITEM_TYPE -> if (v[0].isNotEmpty()) add(e.urls, v[0])
                    ContactsContract.CommonDataKinds.Note.CONTENT_ITEM_TYPE -> if (v[0].isNotEmpty()) add(e.notes, v[0])
                    ContactsContract.CommonDataKinds.Event.CONTENT_ITEM_TYPE -> if (v[0].isNotEmpty()) add(e.events,
                        JSONObject().put("type", label(1, 2) { when (it) { 1 -> "anniversary"; 3 -> "birthday"; else -> "other" } }).put("date", v[0]))
                    ContactsContract.CommonDataKinds.Relation.CONTENT_ITEM_TYPE -> if (v[0].isNotEmpty()) add(e.relations,
                        JSONObject().put("type", label(1, 2) { "type$it" }).put("name", v[0]))
                    ContactsContract.CommonDataKinds.Im.CONTENT_ITEM_TYPE -> if (v[0].isNotEmpty()) add(e.ims, typed(v[4].ifEmpty { v[5] }, v[0]))
                    ContactsContract.CommonDataKinds.GroupMembership.CONTENT_ITEM_TYPE ->
                        v[0].toLongOrNull()?.let { g -> groups[g]?.let { add(e.groups, it) } }
                }
            }
        }
        return byId.values.toList()
    }

    private fun json(e: Entry, snap: Long): JSONObject = JSONObject().apply {
        put("type", "contact"); put("snap", snap); put("ts", snap)
        put("name", e.name); put("nickname", e.nickname)
        put("emails", e.emails); put("phones", e.phones); put("orgs", e.orgs); put("addresses", e.addresses)
        put("urls", e.urls); put("notes", e.notes); put("events", e.events); put("relations", e.relations)
        put("ims", e.ims); put("groups", e.groups); put("raw", e.raw)
    }

    private fun digest(entries: List<Entry>): String {
        val text = entries.joinToString("\n") { json(it, 0).toString() }
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
