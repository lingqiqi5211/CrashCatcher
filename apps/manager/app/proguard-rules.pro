# Deliberately small. Almost nothing here needs a rule:
#
# - Activities are named in the manifest, so AGP keeps them.
# - The view-model factory compares `::class.java` in a `when` and constructs each one
#   directly, so no view model is reached by reflection.
# - Wire property names come from kotlinx's generated descriptors, which are compile-time
#   string constants; SnakeCase naming and every @SerialName survive obfuscation because
#   nothing reads a Kotlin property name at runtime.
#
# What does need saying is below.

# R8 in full mode will drop a @Serializable class's generated serializer when it cannot
# see the reflective lookup that reaches it. kotlinx ships consumer rules that cover the
# common shapes; these are the upstream-recommended keeps, repeated here so a dependency
# bump that loses them cannot silently break the protocol — the failure mode is a
# SerializationException at the first daemon request, not a build error.
-if @kotlinx.serialization.Serializable class **
-keepclassmembers class <1> {
    static <1>$Companion Companion;
}
-if @kotlinx.serialization.Serializable class ** {
    static **$* *;
}
-keepclassmembers class <2>$<3> {
    kotlinx.serialization.KSerializer serializer(...);
}

# The bridge starts this by component name from outside the app, and the daemon has the
# string hardcoded. R8 does not rename manifest-declared classes, but it may drop one it
# thinks is unreachable — nothing in this APK ever references it.
-keep class io.github.lingqiqi5211.crashcatcher.ui.detail.CrashDetailActivity { *; }
