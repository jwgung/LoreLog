package com.lorelog.data

/** 위치 한 점. */
data class LocationPoint(
    val id: Long,
    val timestampMs: Long,
    val latitude: Double,
    val longitude: Double,
    val accuracyM: Float,
)
