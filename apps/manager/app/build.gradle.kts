import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    // No `org.jetbrains.kotlin.android`: AGP 9 has built-in Kotlin support and refuses
    // to apply alongside the standalone plugin.
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.kotlin.serialization)
}

/**
 * Release signing, read from an untracked `keystore.properties`.
 *
 * The daemon authenticates the manager by pinning its signing certificate, so a
 * debug build signed with the throwaway debug key would be rejected on the socket.
 * Both build types therefore use the release key when it is available, and the
 * build says so plainly when it is not rather than producing an app that silently
 * cannot connect.
 */
val keystoreProperties = Properties().apply {
    val file = rootProject.file("keystore.properties")
    if (file.isFile) file.inputStream().use { load(it) }
}
val hasReleaseKey = keystoreProperties.getProperty("storeFile") != null

/**
 * The release version, from the repository's `version.properties`.
 *
 * Shared with `cch-packager`, which writes the same value into `module.prop`, so the APK
 * and the module it is pinned to always report the same version. Hardcoding it here is
 * how the two drift.
 *
 * `versionCode` is derived rather than tracked separately: it has to increase for every
 * release Android will accept as an update, and a second hand-maintained number is a
 * second thing to forget. `0.1.0` becomes 10000, `1.2.3` becomes 10203 — monotonic as
 * long as minor and patch stay below 100, which is checked below rather than assumed.
 */
val releaseVersion: String = Properties().apply {
    val file = rootProject.file("../../version.properties")
    require(file.isFile) { "version.properties is missing at ${file.absolutePath}" }
    file.inputStream().use { load(it) }
}.getProperty("version") ?: error("version.properties has no `version`")

val releaseVersionCode: Int = run {
    val parts = releaseVersion.split('.').map {
        it.toIntOrNull() ?: error("version `$releaseVersion` is not major.minor.patch")
    }
    require(parts.size == 3) { "version `$releaseVersion` is not major.minor.patch" }
    val (major, minor, patch) = parts
    require(minor < 100 && patch < 100) {
        "version `$releaseVersion` overflows the versionCode encoding (minor/patch < 100)"
    }
    major * 10_000 + minor * 100 + patch
}

android {
    namespace = "io.github.lingqiqi5211.crashcatcher"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "io.github.lingqiqi5211.crashcatcher"
        minSdk = libs.versions.minSdk.get().toInt()
        targetSdk = libs.versions.targetSdk.get().toInt()
        versionCode = releaseVersionCode
        versionName = releaseVersion
    }

    if (hasReleaseKey) {
        signingConfigs {
            create("pinned") {
                storeFile = rootProject.file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        debug {
            if (hasReleaseKey) signingConfig = signingConfigs.getByName("pinned")
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (hasReleaseKey) signingConfig = signingConfigs.getByName("pinned")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    androidResources {
        generateLocaleConfig = true
    }

    sourceSets {
        getByName("main").kotlin.srcDir("src/main/kotlin")
        getByName("test").kotlin.srcDir("src/test/kotlin")
    }

    testOptions {
        unitTests {
            isIncludeAndroidResources = true
            all {
                // Robolectric loads a full android-all jar per SDK/qualifier sandbox.
                it.maxHeapSize = "2g"
            }
        }
    }

    packaging {
        resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}")
    }
}

kotlin {
    // JVM 21 bytecode even on a newer host JDK, matching MeowUI.
    jvmToolchain(21)
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.datastore.preferences)

    implementation(platform(libs.compose.bom))
    implementation(libs.compose.runtime)
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.core)
    implementation(libs.compose.material.icons.extended)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.compose.ui.tooling.preview)

    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)

    implementation(libs.material.kolor)
    implementation(libs.appiconloader)

    implementation(libs.meowui)
    implementation(libs.miuix.ui)
    implementation(libs.miuix.preference)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.test.ext.junit)
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
    debugImplementation(libs.compose.ui.test.manifest)
}
