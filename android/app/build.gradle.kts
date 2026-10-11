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
    // 🔴 直接指向**仓库根的 `assets/`**，不做拷贝（2026-08-… → 2026-10-11 修）。
//
// 此前是先把 `assets/` 手工拷到 `src/main/assets/`，而那份拷贝**被 gitignore**、
// 没有任何东西保证它跟得上源码 ⇒ **改了着色器忘记拷 = 设备上跑旧 .spv**，
// 而本地（桌面）一切正常，很难往这里想。
// 实测（2026-10-11）：APK 里 `triangle.frag.spv` 39324 字节，本地已 40404 字节 ——
// 差的正是当天加的运动向量那段；表现是"`RV3D_MV_DEBUG=1` 打开了、引擎日志也报 true，
// 但画面上什么都没有"。
// 这与 `jniLibs` 那条 `.so` 是**同一类**问题（见 `PORT-PROGRESS.md` 的工程教训）：
// 构建产物必须**只有一份真源**，靠人记得拷迟早会错。
sourceSets["main"].assets.srcDirs("../../assets")
}

dependencies {
    // GameActivity：winit 的 android-game-activity 后端对应的 Activity 基类
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.6.1")
}
