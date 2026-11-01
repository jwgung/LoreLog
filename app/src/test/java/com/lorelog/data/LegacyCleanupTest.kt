package com.lorelog.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class LegacyCleanupTest {
    @get:Rule val tmp = TemporaryFolder()

    @Test
    fun deletesDownloadedAndPartialWhisperModels() {
        val filesDir = tmp.newFolder("files")
        val whisper = File(filesDir, LegacyCleanup.WHISPER_DIR).apply { mkdirs() }
        File(whisper, "ggml-base.bin").writeBytes(ByteArray(1024))
        File(whisper, "ggml-small.bin.part").writeBytes(ByteArray(16))
        val other = File(filesDir, "voice_kept").apply { mkdirs() }
        File(other, "keep.m4a").writeBytes(ByteArray(8))

        assertTrue(LegacyCleanup.deleteWhisperModels(filesDir))

        assertFalse(whisper.exists())
        assertTrue(File(other, "keep.m4a").exists())
    }

    @Test
    fun missingWhisperDirIsAlreadyClean() {
        assertTrue(LegacyCleanup.deleteWhisperModels(tmp.newFolder("empty")))
    }

    @Test
    fun runsOnceUntilVersionRecorded() {
        assertTrue(LegacyCleanup.needed(0))
        assertFalse(LegacyCleanup.needed(LegacyCleanup.VERSION))
    }

    @Test
    fun removesOnlyGeminiSecretsNotVaultOrDbKey() {
        assertEquals(listOf("llm_base_url", "llm_model", "llm_api_key"), LegacyCleanup.OBSOLETE_SECRET_KEYS)
        listOf("endpoint_url", "endpoint_token", "db_key").forEach { assertFalse(it in LegacyCleanup.OBSOLETE_SECRET_KEYS) }
    }
}
