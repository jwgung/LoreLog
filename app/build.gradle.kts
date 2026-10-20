plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.lorelog"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.lorelog"
        minSdk = 29
        targetSdk = 36
        versionCode = (providers.gradleProperty("appVersionCode").orNull?.toIntOrNull()) ?: 4
        versionName = providers.gradleProperty("appVersionName").orNull ?: "0.3.2"
        ndk {
            abiFilters += listOf("arm64-v8a") // whisper 가 arm64 전용이고 onnxruntime(13MB)을 ABI 마다 싸 가기엔 무겁다
        }
    }

    val releaseStore: String? = providers.gradleProperty("lifelog.storeFile").orNull
    signingConfigs {
        if (releaseStore != null) {
            create("release") {
                storeFile = file(releaseStore)
                storePassword = providers.gradleProperty("lifelog.storePassword").get()
                keyAlias = providers.gradleProperty("lifelog.keyAlias").get()
                keyPassword = providers.gradleProperty("lifelog.keyPassword").get()
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName(if (releaseStore != null) "release" else "debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures {
        compose = true
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.core:core-ktx:1.17.0")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.11.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("com.google.android.gms:play-services-location:21.3.0")
    implementation("net.zetetic:sqlcipher-android:4.19.1")
    implementation("androidx.sqlite:sqlite:2.4.0")
    implementation("androidx.security:security-crypto:1.1.0")
    implementation("androidx.work:work-runtime-ktx:2.10.5")
    implementation("androidx.documentfile:documentfile:1.0.1")
    implementation("com.github.gkonovalov.android-vad:silero:2.0.10")
    implementation("dev.ffmpegkit-maintained:whisper-android:1.0.0")
    implementation("com.google.mlkit:genai-summarization:1.0.0-beta1")
    implementation("androidx.concurrent:concurrent-futures-ktx:1.2.0")

    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.material3:material3")

    debugImplementation("androidx.compose.ui:ui-tooling")

    testImplementation("junit:junit:4.13.2")
}
