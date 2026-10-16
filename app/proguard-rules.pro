# SQLCipher (네이티브 브리지)
-keep class net.zetetic.** { *; }
-keep class net.sqlcipher.** { *; }
-dontwarn net.zetetic.**
# Tink / EncryptedSharedPreferences (암호화 키·prefs)
-keep class com.google.crypto.tink.** { *; }
-keep class androidx.security.crypto.** { *; }
-dontwarn com.google.crypto.tink.**

# Room (WorkManager 내부 WorkDatabase 가 Room 생성 _Impl 을 리플렉션으로 생성)
-keep class * extends androidx.room.RoomDatabase { <init>(); }
-keep class androidx.room.RoomDatabase { *; }
-keep class androidx.work.impl.** { *; }
-dontwarn androidx.room.**
