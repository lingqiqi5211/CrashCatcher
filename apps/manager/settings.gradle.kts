pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
        gradlePluginPortal()
    }
}

plugins {
    id("org.gradle.toolchains.foojay-resolver-convention") version "1.0.0"
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "CrashCatcher"
include(":app")

// MeowUI comes in as an included build, not a published dependency.
//
// The version this app needs pins a miuix snapshot, and those live on GitHub
// Packages where even a public package wants a token. MeowUI carries miuix as its
// own submodule, so consuming it from source means the whole tree builds with no
// repository credentials at all. Gradle substitutes the module coordinate for the
// included build's project, so nothing in app/build.gradle.kts has to know.
//
// `meowui.dir` in local.properties overrides the location — useful when a checkout
// already exists elsewhere on the machine and cloning it twice is a waste.
val localProperties = java.util.Properties().apply {
    val file = file("local.properties")
    if (file.isFile) file.inputStream().use { load(it) }
}

val meowUiDir = localProperties.getProperty("meowui.dir")
    ?.let(::file)
    ?: file("../../third_party/MeowUI")

require(meowUiDir.resolve("settings.gradle.kts").isFile) {
    """
    MeowUI is not checked out at ${meowUiDir.absolutePath}

    This build consumes MeowUI as an included build rather than a published
    dependency, because the version it needs pins a miuix snapshot that cannot be
    resolved without credentials.

        git submodule update --init --recursive

    (from the repository root, or clone with `git clone --recurse-submodules`.)

    To point at an existing checkout instead, add to local.properties:

        meowui.dir=/path/to/MeowUI
    """.trimIndent()
}

// The included build looks for the SDK in its own local.properties, which a freshly
// initialised submodule does not have. Seed it from this build's copy so that a
// clone plus `git submodule update --init` is all a machine needs. Both files are
// git-ignored. Rewritten when the value drifts, not only when it is missing.
val hostSdkDir = localProperties.getProperty("sdk.dir")
if (hostSdkDir != null) {
    val submoduleProperties = meowUiDir.resolve("local.properties")
    val existing = java.util.Properties().apply {
        if (submoduleProperties.isFile) submoduleProperties.inputStream().use { load(it) }
    }
    if (existing.getProperty("sdk.dir") != hostSdkDir) {
        existing.setProperty("sdk.dir", hostSdkDir)
        submoduleProperties.outputStream().use {
            existing.store(it, "Seeded by the CrashCatcher manager build.")
        }
    }
}

includeBuild(meowUiDir)

// miuix comes in here as well, even though MeowUI already includes it.
//
// Substitution from a *nested* included build does not reach this build's dependency
// graph: MeowUI resolves `top.yukonga.miuix.kmp:*` against its own included copy, but
// when MeowUI is itself an included build, those coordinates resolve here — and since
// 0.9.4-rc01 they exist on Maven Central, so they resolve to the published artifact
// instead of failing loudly. The Android variant of `miuix-preference` there does not
// carry the classes MeowUI compiles against, and the build dies with a page of
// "Unresolved reference 'preference'" pointing at MeowUI's own sources.
//
// Including it directly makes the substitution apply at this level too, so the whole
// tree compiles one miuix — the one the submodule is checked out at.
val miuixDir = meowUiDir.resolve("third_party/miuix")
require(miuixDir.resolve("settings.gradle.kts").isFile) {
    """
    miuix is not checked out at ${miuixDir.absolutePath}

        git submodule update --init --recursive

    (from the repository root, or clone with `git clone --recurse-submodules`.)
    """.trimIndent()
}
includeBuild(miuixDir)
