package io.github.lingqiqi5211.crashcatcher.ui.crashes

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Folding is what keeps the detail screen's first paint small.
 *
 * A real trace is a handful of app frames buried in platform noise; if folding gets
 * this wrong the screen either hides the frames that matter or shows all forty lines,
 * which is the behaviour being replaced.
 */
class StackFramesTest {

    @Test
    fun `platform frames are recognised however they are indented`() {
        assertTrue(isFrameworkFrame("\tat android.app.Activity.performCreate(Activity.java:8595)"))
        assertTrue(isFrameworkFrame("at java.lang.Thread.run(Thread.java:1012)"))
        assertTrue(isFrameworkFrame("  at androidx.compose.runtime.Composer.skip(Composer.kt:1)"))
        assertTrue(
            isFrameworkFrame("\tat kotlinx.coroutines.DispatchedTask.run(DispatchedTask.kt:100)"),
        )
    }

    @Test
    fun `app frames are left alone`() {
        assertFalse(isFrameworkFrame("\tat com.example.app.MainActivity.onCreate(MainActivity.kt:37)"))
        assertFalse(isFrameworkFrame(""))
    }

    @Test
    fun `the exception header is never folded away`() {
        // It starts with `java.` but it is the single most important line of the
        // trace — matching it on the package prefix would hide the crash's identity.
        assertFalse(isFrameworkFrame("java.lang.IllegalStateException: Fragment already added"))
        assertFalse(isFrameworkFrame("Caused by: java.lang.NullPointerException"))
        assertFalse(isFrameworkFrame("FATAL EXCEPTION: main"))
    }

    @Test
    fun `native backtrace lines are not folded`() {
        // Library paths, not Java packages; this list says nothing useful about them.
        assertFalse(
            isFrameworkFrame("      #00 pc 0000000000001ac4  /system/lib64/libc.so (abort+164)"),
        )
    }

    @Test
    fun `kotlin stdlib is noise but a similarly named package is not`() {
        assertTrue(isFrameworkFrame("at kotlin.collections.CollectionsKt.first(Collections.kt:1)"))
        assertFalse(isFrameworkFrame("at kotlinapp.Feature.run(Feature.kt:1)"))
    }

    @Test
    fun `a run of platform frames collapses into one expander`() {
        val trace = """
            java.lang.IllegalStateException: boom
            ${'\t'}at com.example.app.MainActivity.onCreate(MainActivity.kt:37)
            ${'\t'}at android.app.Activity.performCreate(Activity.java:8595)
            ${'\t'}at android.app.Instrumentation.callActivityOnCreate(Instrumentation.java:1456)
            ${'\t'}at android.app.ActivityThread.performLaunchActivity(ActivityThread.java:3893)
            ${'\t'}at com.example.app.Repo.load(Repo.kt:88)
        """.trimIndent()

        val items = buildStackItems(trace, foldFrameworkFrames = true)

        assertEquals(4, items.size)
        assertTrue(items[0] is StackItem.Line)
        assertTrue(items[1] is StackItem.Line)
        val folded = items[2] as StackItem.FoldedFrames
        assertEquals(3, folded.lines.size)
        assertTrue(items[3] is StackItem.Line)
    }

    @Test
    fun `a short run is not worth an expander`() {
        val trace = """
            ${'\t'}at com.example.app.A.run(A.kt:1)
            ${'\t'}at android.app.Activity.performCreate(Activity.java:1)
            ${'\t'}at com.example.app.B.run(B.kt:1)
        """.trimIndent()

        val items = buildStackItems(trace, foldFrameworkFrames = true)

        // Replacing one line with "show 1 frame" costs a tap and saves nothing.
        assertEquals(3, items.size)
        assertTrue(items.all { it is StackItem.Line })
    }

    @Test
    fun `folding off leaves every line addressable`() {
        val trace = (0 until 10).joinToString("\n") { "\tat android.app.Thing$it.run(T.java:1)" }
        val items = buildStackItems(trace, foldFrameworkFrames = false)

        assertEquals(10, items.size)
        assertTrue(items.all { it is StackItem.Line })
    }

    @Test
    fun `line indices survive folding so keys stay stable`() {
        val trace = """
            header
            ${'\t'}at android.a.A.run(A.java:1)
            ${'\t'}at android.b.B.run(B.java:1)
            ${'\t'}at android.c.C.run(C.java:1)
            ${'\t'}at com.example.app.Tail.run(Tail.kt:1)
        """.trimIndent()

        val items = buildStackItems(trace, foldFrameworkFrames = true)
        val fold = items.filterIsInstance<StackItem.FoldedFrames>().single()

        assertEquals(1, fold.firstIndex)
        assertEquals(listOf(1, 2, 3), fold.lines.map { it.index })
        // The tail keeps its original index, so expanding a fold above it cannot make
        // two items collide on the same list key.
        assertEquals(4, (items.last() as StackItem.Line).line.index)
    }

    @Test
    fun `a blank line inside a platform run does not split it`() {
        val trace = """
            ${'\t'}at com.example.app.A.run(A.kt:1)
            ${'\t'}at android.a.A.run(A.java:1)

            ${'\t'}at android.b.B.run(B.java:1)
            ${'\t'}at android.c.C.run(C.java:1)
        """.trimIndent()

        val items = buildStackItems(trace, foldFrameworkFrames = true)
        val fold = items.filterIsInstance<StackItem.FoldedFrames>().single()

        assertEquals("the paragraph break should not become two expanders", 4, fold.lines.size)
    }

    @Test
    fun `an empty payload produces nothing surprising`() {
        val items = buildStackItems("", foldFrameworkFrames = true)
        assertEquals(1, items.size)
        assertEquals("", (items.single() as StackItem.Line).line.text)
    }

    @Test
    fun `a trace that is entirely platform frames still collapses`() {
        val trace = (0 until 20).joinToString("\n") { "\tat android.app.Thing$it.run(T.java:1)" }
        val items = buildStackItems(trace, foldFrameworkFrames = true)

        val fold = items.filterIsInstance<StackItem.FoldedFrames>().single()
        assertEquals(20, fold.lines.size)
        assertEquals(1, items.size)
    }
}
