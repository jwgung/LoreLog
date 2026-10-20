package com.lorelog.audio

import android.content.Context
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.util.Log
import java.io.File
import java.io.RandomAccessFile
import java.util.concurrent.TimeUnit

/**
 * 전사가 끝난 음성의 오디오 보관소. "무슨 소리였는지" 다시 들어볼 수 있게 WAV 를 AAC(m4a)로
 * 압축해 voice_kept/ 에 둔다(4분 배치 ≈ 1.4MB). 텍스트가 본체이고 오디오는 부속이라
 * 내보낸 뒤에는 바로, 내보내지 않았으면 [RETENTION_MS] 뒤에 지운다. 내보내기에는 포함하지 않는다.
 */
object VoiceAudio {
    private const val TAG = "VoiceAudio"
    private const val DIR = "voice_kept"
    const val RETENTION_MS = 7 * 24 * 60 * 60 * 1000L
    private const val SAMPLE_RATE = 16_000
    private const val BIT_RATE = 48_000
    private const val WAV_HEADER = 44
    private val NAME_TS = Regex("^(\\d+)")

    fun dir(context: Context): File {
        val base = context.applicationContext.getExternalFilesDir(null) ?: context.applicationContext.cacheDir
        return File(base, DIR).apply { mkdirs() }
    }

    fun file(context: Context, name: String): File = File(dir(context), name)

    /** 파일명에서 녹음 시각. 보관 파일은 <ts>.m4a 또는 <ts>.wav. */
    fun recordedAt(name: String): Long? = NAME_TS.find(name)?.groupValues?.get(1)?.toLongOrNull()

    /**
     * 전사가 끝난 WAV 를 보관소로 옮긴다. AAC 로 압축하고 원본 WAV 는 지운다. 인코딩이 안 되는 기기면
     * WAV 그대로 옮긴다. 보관 파일명(DB 에 저장)을 돌려주고, 아무것도 못 했으면 null(WAV 는 호출자가 지움).
     */
    fun keep(context: Context, wav: File, recordedAtMs: Long): String? {
        val base = uniqueBase(context, recordedAtMs)
        val m4a = file(context, "$base.m4a")
        val ok = try {
            encodeAac(wav, m4a)
        } catch (e: Exception) {
            Log.w(TAG, "aac encode failed: ${e.message}")
            false
        }
        if (ok && m4a.length() > 0) {
            wav.delete()
            return m4a.name
        }
        m4a.delete()
        val raw = file(context, "$base.wav")
        return if (wav.renameTo(raw) || runCatching { wav.copyTo(raw, overwrite = true); wav.delete() }.isSuccess) raw.name else null
    }

    private fun uniqueBase(context: Context, ts: Long): String {
        var t = ts
        while (file(context, "$t.m4a").exists() || file(context, "$t.wav").exists()) t++
        return t.toString()
    }

    /** 16kHz mono 16bit PCM WAV → AAC-LC m4a. 동기 인코딩(4분 분량에 2초 안팎). */
    private fun encodeAac(wav: File, out: File): Boolean {
        val pcmBytes = wav.length() - WAV_HEADER
        if (pcmBytes <= 0) return false
        val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, SAMPLE_RATE, 1).apply {
            setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
            setInteger(MediaFormat.KEY_BIT_RATE, BIT_RATE)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 16 * 1024)
        }
        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC)
        val muxer = MediaMuxer(out.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
        var track = -1
        var muxerStarted = false
        try {
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            codec.start()
            RandomAccessFile(wav, "r").use { f ->
                f.seek(WAV_HEADER.toLong())
                val info = MediaCodec.BufferInfo()
                var inputDone = false
                var outputDone = false
                var readBytes = 0L
                val chunk = ByteArray(8 * 1024)
                while (!outputDone) {
                    if (!inputDone) {
                        val inIdx = codec.dequeueInputBuffer(10_000)
                        if (inIdx >= 0) {
                            val buf = codec.getInputBuffer(inIdx)!!
                            buf.clear()
                            val n = f.read(chunk, 0, minOf(chunk.size, buf.capacity()))
                            if (n <= 0) {
                                codec.queueInputBuffer(inIdx, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                                inputDone = true
                            } else {
                                buf.put(chunk, 0, n)
                                val ptsUs = readBytes * 1_000_000L / (SAMPLE_RATE * 2)
                                readBytes += n
                                codec.queueInputBuffer(inIdx, 0, n, ptsUs, 0)
                            }
                        }
                    }
                    val outIdx = codec.dequeueOutputBuffer(info, 10_000)
                    when {
                        outIdx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                            track = muxer.addTrack(codec.outputFormat)
                            muxer.start()
                            muxerStarted = true
                        }
                        outIdx >= 0 -> {
                            val buf = codec.getOutputBuffer(outIdx)!!
                            if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) info.size = 0
                            if (info.size > 0 && muxerStarted) {
                                buf.position(info.offset)
                                buf.limit(info.offset + info.size)
                                muxer.writeSampleData(track, buf, info)
                            }
                            codec.releaseOutputBuffer(outIdx, false)
                            if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) outputDone = true
                        }
                    }
                }
            }
            return muxerStarted
        } finally {
            runCatching { codec.stop() }
            runCatching { codec.release() }
            if (muxerStarted) runCatching { muxer.stop() }
            runCatching { muxer.release() }
        }
    }

    /** WAV 길이(ms). 16kHz mono 16bit 기준. */
    fun wavDurationMs(f: File): Long = ((f.length() - WAV_HEADER).coerceAtLeast(0) * 1000L) / (SAMPLE_RATE * 2)

    /** DB 에서 참조하지 않는 보관 파일 정리(앱 강제 종료 등으로 남은 고아). */
    fun deleteUnreferenced(context: Context, referenced: Set<String>) {
        dir(context).listFiles()?.forEach { f ->
            if (f.name !in referenced && f.lastModified() < System.currentTimeMillis() - TimeUnit.MINUTES.toMillis(10)) f.delete()
        }
    }

    fun deleteAll(context: Context) {
        dir(context).listFiles()?.forEach { it.delete() }
    }

    /** 사람이 읽을 길이 표기("1분 58초"). */
    fun formatDuration(ms: Long): String {
        val s = (ms / 1000).toInt()
        return if (s >= 60) "${s / 60}분 ${s % 60}초" else "${s}초"
    }
}
