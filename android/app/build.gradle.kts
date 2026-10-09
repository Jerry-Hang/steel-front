plugins {
    id("com.android.application")
}

android {
    namespace = "com.steelfront"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.steelfront.game"
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    // GameActivity 的原生 JNI 注册在 extractNativeLibs=false（默认）下会
    // RegisterNatives failed 崩溃 ⇒ 用传统打包（.so 解压到 lib/）。
    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // 引擎的 assets/ 原样打进 APK 的 assets/（AAssetManager 路径与桌面相对路径同形）
    sourceSets["main"].assets.srcDirs("src/main/assets")
}

dependencies {
    // GameActivity：winit 的 android-game-activity 后端对应的 Activity 基类
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.6.1")
}
