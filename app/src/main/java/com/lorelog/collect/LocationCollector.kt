package com.lorelog.collect

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Looper
import androidx.core.content.ContextCompat
import com.google.android.gms.location.FusedLocationProviderClient
import com.google.android.gms.location.LocationCallback
import com.google.android.gms.location.LocationRequest
import com.google.android.gms.location.LocationResult
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.Priority
import com.lorelog.data.LogRepository

/**
 * 위치 수집기. FusedLocationProvider 로 일정 간격·이동 거리마다 위치를 기록한다.
 * 위치 권한이 없으면 상태만 알리고 쉰다. 포그라운드 서비스(type=location)에서 돌아
 * 앱이 안 보여도 기록이 이어진다.
 */
class LocationCollector(context: Context) {

    private val appContext = context.applicationContext
    private val client: FusedLocationProviderClient =
        LocationServices.getFusedLocationProviderClient(appContext)

    private val callback = object : LocationCallback() {
        override fun onLocationResult(result: LocationResult) {
            // 배치로 여러 점이 한 번에 올 수 있으므로 전부 저장한다.
            for (loc in result.locations) {
                LogRepository.addLocation(loc.latitude, loc.longitude, loc.accuracy, loc.time)
            }
        }
    }

    fun start() {
        if (ContextCompat.checkSelfPermission(appContext, Manifest.permission.ACCESS_FINE_LOCATION)
            != PackageManager.PERMISSION_GRANTED
        ) {
            LogRepository.setStatus("위치 기록을 켜려면 위치 권한이 필요합니다")
            return
        }
        val request = LocationRequest.Builder(Priority.PRIORITY_BALANCED_POWER_ACCURACY, INTERVAL_MS)
            .setMinUpdateIntervalMillis(MIN_INTERVAL_MS)
            .setMinUpdateDistanceMeters(MIN_DISTANCE_M)
            .setMaxUpdateDelayMillis(MAX_DELAY_MS)
            .build()
        try {
            // 캐시된 마지막 위치가 있으면 첫 점으로 바로 남긴다.
            client.lastLocation.addOnSuccessListener { loc ->
                if (loc != null) {
                    LogRepository.addLocation(loc.latitude, loc.longitude, loc.accuracy, loc.time)
                }
            }
            client.requestLocationUpdates(request, callback, Looper.getMainLooper())
        } catch (e: SecurityException) {
            LogRepository.setStatus("위치 권한이 없어 위치를 기록할 수 없습니다")
        }
    }

    fun stop() {
        client.removeLocationUpdates(callback)
    }

    companion object {
        private const val INTERVAL_MS = 300_000L      // 5분
        private const val MIN_INTERVAL_MS = 120_000L  // 빨라도 2분
        private const val MIN_DISTANCE_M = 50f
        private const val MAX_DELAY_MS = 600_000L     // 배치 허용(최대 10분 지연)
    }
}
