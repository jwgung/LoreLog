package com.lorelog.net

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** 되묻기 후보(#108): 서버가 보낸 choices를 버튼으로, 누른 버튼은 순번이나 "no"로 보낸다. */
class ChoicesTest {
    @Test
    fun theCandidatesOfAMatterOrAPersonAreOneButtonEachSentByPosition() {
        val c = DiaryClient.parseChoices(JSONObject("""{"kind": "matters", "options": ["견적 일차 · 2026-03-01 시작", "견적 이차 · 2026-05-01 시작"]}"""))!!
        val b = DiaryClient.buttons(c)
        assertEquals(listOf("견적 일차 · 2026-03-01 시작", "견적 이차 · 2026-05-01 시작"), b.map { it.label })
        assertEquals(listOf<Any>(1, 2), b.map { it.value })
        val people = DiaryClient.buttons(DiaryClient.parseChoices(JSONObject("""{"kind": "people", "options": ["김영희 · x.com"]}"""))!!)
        assertEquals(listOf("김영희 · x.com"), people.map { it.label })
    }

    @Test
    fun theMatterUnderWayIsAnsweredYesOrNo() {
        val b = DiaryClient.buttons(DiaryClient.parseChoices(JSONObject("""{"kind": "now", "options": ["견적 사차 · 2026-08-01 시작"]}"""))!!)
        assertEquals(listOf("네, 그 건", "아니요"), b.map { it.label })
        assertEquals(listOf<Any>(1, "no"), b.map { it.value })
    }

    @Test
    fun choicesTheAppCannotShowAreNone() {
        assertNull(DiaryClient.parseChoices(null))
        assertNull(DiaryClient.parseChoices(JSONObject("""{"kind": "matters", "options": []}""")))
        assertNull(DiaryClient.parseChoices(JSONObject("""{"kind": "later", "options": ["a"]}""")))
        assertNull(DiaryClient.parseChoices(JSONObject("""{"options": ["a"]}""")))
        // 빈 글자는 빼고, 많아야 셋
        val c = DiaryClient.parseChoices(JSONObject("""{"kind": "matters", "options": ["a", " ", "b", "c", "d"]}"""))!!
        assertEquals(listOf("a", "b", "c"), c.options)
    }

    @Test
    fun aChoiceGoesAsAPositionOrNoAndNeverWithACorrection() {
        val pick = DiaryClient.chatBody("견적 이차 · 2026-05-01 시작", null, "summary", 2)
        assertEquals(2, pick.getInt("choice"))
        assertEquals("견적 이차 · 2026-05-01 시작", pick.getString("message"))
        assertFalse(pick.has("wrong"))
        assertEquals("no", DiaryClient.chatBody("아니요", null, "summary", "no").getString("choice"))
        val plain = DiaryClient.chatBody("오늘 피곤하네", null, "summary", null)
        assertFalse(plain.has("choice"))
        val fix = DiaryClient.chatBody("20분", "10초 통화", "summary", 1)
        assertTrue(fix.has("wrong") && !fix.has("choice"))
    }
}
