plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.conduit.app"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        isCoreLibraryDesugaringEnabled = true
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "com.conduit.app"
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    buildTypes {
        release {
            // Use release keystore when env vars are provided (CI), else debug for local builds.
            val envKeystorePath = System.getenv("ANDROID_KEYSTORE_PATH")
            val envKeyAlias = System.getenv("ANDROID_KEY_ALIAS")
            val envKeyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            val envStorePassword = System.getenv("ANDROID_STORE_PASSWORD")
            if (!envKeystorePath.isNullOrEmpty() && !envKeyAlias.isNullOrEmpty()) {
                signingConfigs.create("release") {
                    storeFile = file(envKeystorePath)
                    storePassword = envStorePassword
                    keyAlias = envKeyAlias
                    keyPassword = envKeyPassword
                }
                signingConfig = signingConfigs.getByName("release")
            } else {
                signingConfig = signingConfigs.getByName("debug")
            }
        }
    }
}

dependencies {
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.4")
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}
