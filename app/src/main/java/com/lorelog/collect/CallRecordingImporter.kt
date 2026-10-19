package com.lorelog.collect

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.documentfile.provider.DocumentFile
import com.lorelog.data.LogRepository
import com.lorelog.net.GroqTranscriber
import com.lorelog.net.SttConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * 삼성 전화앱 등이 저장한 통화 녹음 파일을 폴더에서 읽어 전사한다.
 * 라이브 통화 캡처는 OS가 막으므로, 이미 저장된 녹음을 가져오는 방식.
 * 사용자가 SAF로 녹음 폴더를 한 번 지정하면, 새 파일만 전사해 타임라인에 남긴다.
 */
object CallRecordingImporter {
    private const val PREF = "callrec"
    private const val KEY_TREE = "tree_uri"
    private const val KEY_DONE = "processed"
    private val AUDIO_EXT = listOf(".m4a", ".mp3", ".amr", ".wav", ".3gp", ".ogg", ".aac")

    private fun p(context: Context) =
        context.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)

    fun folder(context: Context): Uri? =
        p(context).getString(KEY_TREE, null)?.takeIf { it.isNotBlank() }?.let { Uri.parse(it) }

    fun setFolder(context: Context, uri: Uri) {
        try {
            context.contentResolver.takePersistableUriPermission(
                uri, Intent.FLAG_GRANT_READ_URI_PERMISSION,
            )
        } catch (_: Exception) {}
        p(context).edit().putString(KEY_TREE, uri.toString()).apply()
    }

    private fun done(context: Context): MutableSet<String> =
        HashSet(p(context).getStringSet(KEY_DONE, emptySet()) ?: emptySet())

    private fun markDone(context: Context, keys: Set<String>) {
        p(context).edit().putStringSet(KEY_DONE, keys).apply()
    }

    /** 폴더의 새 녹음을 전사해 저장. 처리한 개수 반환. */
    suspend fun importNew(context: Context): Int = withContext(Dispatchers.IO) {
        val tree = folder(context) ?: run {
            LogRepository.setStatus("통화 녹음 폴더를 먼저 지정하세요")
            return@withContext 0
        }
        if (!SttConfig.isConfigured(context)) {
            LogRepository.setStatus("통화 녹음 전사엔 STT 키가 필요합니다")
            return@withContext 0
        }
        val dir = DocumentFile.fromTreeUri(context, tree) ?: return@withContext 0
        val processed = done(context)
        var count = 0
        for (f in dir.listFiles()) {
            val name = f.name ?: continue
            if (!f.isFile) continue
            if (AUDIO_EXT.none { name.lowercase().endsWith(it) }) continue
            val key = "$name:${f.length()}:${f.lastModified()}"
            if (key in processed) continue

            val tmp = File(context.cacheDir, "callrec_${System.currentTimeMillis()}_${name.takeLast(32)}")
            val ok = try {
                context.contentResolver.openInputStream(f.uri)?.use { input ->
                    tmp.outputStream().use { input.copyTo(it) }
                }
                true
            } catch (_: Exception) { false }

            if (ok && tmp.length() > 0) {
                val text = GroqTranscriber.transcribe(context, tmp)
                if (text != null) {
                    LogRepository.addVoice(text, speaker = "통화 · ${party(name)}")
                    count++
                }
            }
            tmp.delete()
            processed.add(key)
        }
        markDone(context, processed)
        LogRepository.setStatus("통화 녹음 $count 건 전사")
        count
    }

    /** 파일명에서 상대(이름·번호) 추정. 없으면 파일명 앞부분. */
    private fun party(name: String): String {
        val base = name.substringBeforeLast('.')
        // 흔한 형태: "통화 녹음 홍길동_20260101...", "Call_01012345678_..."
        val cleaned = base.replace(Regex("(통화 녹음|Call|call|voice|recording)"), " ").trim()
        val token = cleaned.split(Regex("[_\\s]+")).firstOrNull { it.isNotBlank() } ?: base
        return token.take(24)
    }
}
