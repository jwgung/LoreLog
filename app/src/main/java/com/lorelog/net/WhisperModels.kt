package com.lorelog.net

import android.content.Context
import java.io.File

/**
 * 다운로드 가능한 whisper.cpp(ggml) 모델 카탈로그. 한국어는 multilingual 모델이 필요하므로
 * .en 모델은 쓰지 않는다. 클수록 정확하지만 느리고 용량이 크다.
 * 파일은 HuggingFace ggerganov/whisper.cpp 에서 받는다.
 */
data class WhisperModelInfo(val id: String, val label: String, val approxMb: Int) {
    val fileName: String get() = "ggml-$id.bin"
    val url: String get() = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$id.bin?download=true"
}

object WhisperModels {
    val ALL = listOf(
        WhisperModelInfo("tiny", "작게 · 빠름 (~75MB)", 75),
        WhisperModelInfo("base", "보통 · 추천 (~142MB)", 142),
        WhisperModelInfo("small", "크게 · 정확 (~466MB)", 466),
    )
    const val DEFAULT_ID = "base"

    fun byId(id: String): WhisperModelInfo =
        ALL.firstOrNull { it.id == id } ?: ALL.first { it.id == DEFAULT_ID }

    fun dir(context: Context): File =
        File(context.applicationContext.filesDir, "whisper").apply { mkdirs() }

    fun fileFor(context: Context, id: String): File = File(dir(context), byId(id).fileName)

    fun isReady(context: Context, id: String): Boolean {
        val f = fileFor(context, id)
        return f.exists() && f.length() > 1_000_000L
    }
}
