package com.lorelog.collect

import android.app.AppOpsManager
import android.content.Context
import android.os.Process

/** '사용 정보 접근'(PACKAGE_USAGE_STATS) 특수 권한이 켜져 있는지. 설정에서 사용자가 직접 켠다. */
object UsageAccess {
    fun isGranted(context: Context): Boolean {
        val appOps = context.getSystemService(Context.APP_OPS_SERVICE) as AppOpsManager
        val mode = appOps.unsafeCheckOpNoThrow(
            AppOpsManager.OPSTR_GET_USAGE_STATS,
            Process.myUid(),
            context.packageName,
        )
        return mode == AppOpsManager.MODE_ALLOWED
    }
}
