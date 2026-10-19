package com.lorelog.collect

import android.content.Context

/**
 * 화면 내용 수집에서 제외할 민감 앱 목록. 은행·결제·비밀번호 관리 앱을 기본으로 넣어 두고,
 * 사용자가 편집할 수 있다. 제외된 앱은 "언제 켰다"(앱 사용)만 남고 화면 내용은 수집하지 않는다.
 * 기본 목록은 완전하지 않다 — 쓰는 앱에 맞춰 설정에서 더하거나 뺀다.
 */
object SensitiveApps {
    private const val PREF = "lifelog_prefs"
    private const val KEY = "excluded_packages"

    /** 한국에서 흔한 은행·결제·비밀번호/인증 앱(일부). */
    val DEFAULTS: Set<String> = setOf(
        // 은행
        "com.kbstar.kbbank", "com.kbstar.liivbank", "com.kbstar.reboot",
        "com.shinhan.sbanking", "com.shinhan.sol",
        "com.wooribank.smart.npib", "com.wooribank.smart.mwib",
        "com.hanabank.ebk.channel.android.hananbank",
        "com.ibk.neobanking",
        "nh.smart.banking", "nh.smart.nhallonepay",
        "com.kakaobank.channel",
        "com.tossbank.app",
        // 결제·지갑
        "com.samsung.android.spay", "com.samsung.android.spayfw",
        "com.kakaopay.app", "viva.republica.toss",
        "com.nhnent.payapp", "com.lguplus.paynow",
        // 카드
        "com.shinhancard.smartshinhan", "com.hanaskcard.paycla",
        "kvp.jjy.MispAndroid320", "com.lotte.lottesmartpay",
        // 비밀번호·인증
        "com.samsung.android.samsungpass", "com.lastpass.lpandroid",
        "com.agilebits.onepassword", "com.x8bit.bitwarden",
        "com.google.android.apps.authenticator2", "org.keepassdroid",
    )

    fun excluded(context: Context): Set<String> =
        context.applicationContext
            .getSharedPreferences(PREF, Context.MODE_PRIVATE)
            .getStringSet(KEY, null) ?: DEFAULTS

    fun isExcluded(context: Context, packageName: String): Boolean =
        excluded(context).contains(packageName)

    fun setExcluded(context: Context, packages: Set<String>) {
        context.applicationContext
            .getSharedPreferences(PREF, Context.MODE_PRIVATE)
            .edit()
            .putStringSet(KEY, packages)
            .apply()
    }
}
